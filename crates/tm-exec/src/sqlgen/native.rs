//! Native regions as FROM items: `tm_path(start, path, mode, max_hops, view[,
//! graphs]) AS pN` with a correlated or bound start (design D7), preceded by the
//! graph enumeration of a `GRAPH ?g` path whose graph no joined pattern binds, and
//! `tm_lftj(plan) AS pN` for a cyclic BGP routed to the LFTJ operator.

use tm_core::{Result, SqlValue};
use tm_ir::Var;

use super::{Col, Gen, IsoPat, Item, Rel, Select};
use crate::lftj::{self, RegionPlan};
use crate::plan::analyze::{Dom, VClass};
use crate::plan::{Node, PGraphs, PPath, PTerm, PText};
use crate::result::{RegionKind, RouteNote};
use crate::scan::view_predicates;

impl Gen<'_> {
    /// Compiles a path pattern; its start is a constant or a column of `acc`.
    pub fn path(&mut self, p: &PPath, acc: &Rel) -> Result<Rel> {
        let start = match &p.arg {
            PTerm::Id(id) => self.params.id(*id),
            PTerm::Var(v) => match acc.col(v) {
                Some(c) => c.sql.clone(),
                None => {
                    return self.unsupported(
                        "a path whose bound endpoint is not bound by a pattern joined with it",
                    )
                }
            },
        };
        let a = self.alias('p');
        let path = self.params.push(SqlValue::Text(p.text.clone()));
        let mode = self.params.push(SqlValue::Text(p.mode.to_string()));
        let max = self.params.push(
            p.max_hops
                .map_or(SqlValue::Null, |m| SqlValue::Integer(m as i64)),
        );
        let view = self.params.push(SqlValue::Text(p.view_text.clone()));
        let mut items = Vec::new();
        let mut graph_col: Option<(Var, String)> = None;
        let graphs = match &p.graphs {
            PGraphs::Any => None,
            PGraphs::Ids(ids) => Some(match ids.as_slice() {
                [one] => self.params.id(*one),
                _ => {
                    let list: Vec<String> = ids.iter().map(|g| g.raw().to_string()).collect();
                    self.params
                        .push(SqlValue::Text(format!("[{}]", list.join(","))))
                }
            }),
            PGraphs::Var(g) => Some(match acc.col(g) {
                // a pattern of the join binds the graph: one call per row, in its graph
                Some(c) => c.sql.clone(),
                // otherwise each graph of the view in turn
                None => {
                    let e = self.alias('x');
                    items.push(Item {
                        sql: format!("{} AS {e}", self.graph_enumeration(p)),
                        on: None,
                    });
                    let col = format!("{e}.g");
                    graph_col = Some((g.clone(), col.clone()));
                    col
                }
            }),
        };
        self.path_region(&a, p.note);
        let call = match graphs {
            None => format!("tm_path({start}, {path}, {mode}, {max}, {view}) AS {a}"),
            Some(g) => format!("tm_path({start}, {path}, {mode}, {max}, {view}, {g}) AS {a}"),
        };
        items.push(Item {
            sql: call,
            on: None,
        });
        let mut r = Rel {
            items,
            ..Rel::default()
        };
        if let Some((g, col)) = graph_col {
            r.set(&g, Col::term(col, false));
        }
        match &p.other {
            PTerm::Id(id) => {
                let ph = self.params.id(*id);
                r.conds.push(format!("{a}.\"end\" = {ph}"));
            }
            PTerm::Var(v) => r.set(v, Col::term(format!("{a}.\"end\""), false)),
        }
        if let Some(v) = &p.bind_arrival {
            // epoch ms, NULL for −∞
            r.set(
                v,
                Col {
                    sql: format!("{a}.arrival"),
                    dom: Dom::Computed(VClass::Int),
                    mm: true,
                    eid_of: None,
                },
            );
        }
        if let Some(b) = &p.bind_path {
            r.set(
                b,
                Col {
                    sql: format!("{a}.path_json"),
                    dom: Dom::PathJson {
                        reversed: p.note == RouteNote::PathInverted,
                    },
                    mm: false,
                    eid_of: None,
                },
            );
        }
        Ok(r)
    }

    /// Compiles a text recall: one `tm_text(…)` call whose rows bind the eid and
    /// the optional score, rank and confidence.
    pub fn text(&mut self, t: &PText) -> Result<Rel> {
        let a = self.alias('x');
        let query = self.params.push(SqlValue::Text(t.query.clone()));
        let mode = self.params.push(SqlValue::Text(t.mode.to_string()));
        let view = self.params.push(SqlValue::Text(t.view_text.clone()));
        let graphs = match &t.graphs {
            None => self.params.push(SqlValue::Null),
            Some(ids) => {
                let list: Vec<String> = ids.iter().map(|g| g.raw().to_string()).collect();
                self.params
                    .push(SqlValue::Text(format!("[{}]", list.join(","))))
            }
        };
        let limit = match t.limit {
            None => self.params.push(SqlValue::Null),
            Some(n) => self.params.push(SqlValue::Integer(i64::from(n))),
        };
        let mut r = Rel {
            items: vec![Item {
                sql: format!("tm_text({query}, {mode}, {view}, {graphs}, {limit}) AS {a}"),
                on: None,
            }],
            ..Rel::default()
        };
        r.set(&t.eid, Col::term(format!("{a}.eid"), false));
        let computed = |sql: String, class: VClass, mm: bool| Col {
            sql,
            dom: Dom::Computed(class),
            mm,
            eid_of: None,
        };
        if let Some(v) = &t.score {
            r.set(v, computed(format!("{a}.score"), VClass::Double, false));
        }
        if let Some(v) = &t.rank {
            r.set(v, computed(format!("{a}.rank"), VClass::Int, false));
        }
        if let Some(v) = &t.confidence {
            r.set(v, computed(format!("{a}.confidence"), VClass::Double, true));
        }
        Ok(r)
    }

    /// The graphs visible in the path's view, as a derived table with column `g`:
    /// objects of visible `sys:inGraph` memberships whose member statement is
    /// visible too. No rows when no membership was ever written.
    fn graph_enumeration(&mut self, p: &PPath) -> String {
        let Some(ig) = p.in_graph else {
            return "(SELECT NULL AS g WHERE 0)".to_string();
        };
        let ig = self.params.id(ig);
        let mut member = vec![format!("gm.p = {ig}")];
        member.extend(view_predicates("gm", &p.view, &mut self.params));
        let mut stmt = vec!["ge.eid = gm.s".to_string()];
        stmt.extend(view_predicates("ge", &p.view, &mut self.params));
        format!(
            "(SELECT DISTINCT gm.o AS g FROM triple AS gm WHERE {} AND \
             EXISTS (SELECT 1 FROM triple AS ge WHERE {}))",
            member.join(" AND "),
            stmt.join(" AND ")
        )
    }
}

impl Gen<'_> {
    /// Compiles a pure cyclic BGP routed to the LFTJ operator: one
    /// `tm_lftj(plan) AS pN` call whose columns bind the region's variables.
    ///
    /// The call sits in a derived table that SQLite cannot flatten (`LIMIT -1`), so
    /// in the inner loop of a join it is materialised once instead of being called
    /// again for every outer row. Under relationship isomorphism the eids of the
    /// grouped patterns are columns too, so patterns outside the region can be
    /// kept distinct from them.
    pub fn lftj_region(&mut self, inputs: &[Node], plan: &RegionPlan) -> Result<Rel> {
        let a = self.alias('p');
        let rid = self.new_region(RegionKind::NativeLftj, RouteNote::LftjNative);
        self.regions[rid].aliases.push(a.clone());
        let spec = self.params.push(SqlValue::Text(plan.spec.to_text()));
        let mut inner = Rel {
            items: vec![Item {
                sql: format!("{}({spec}) AS {a}", lftj::NAME),
                on: None,
            }],
            ..Rel::default()
        };
        let col_var = |i: usize| match &plan.columns[i] {
            Some(v) => v.clone(),
            None => Var::new(format!("~lftj{rid}c{i}")),
        };
        for i in 0..plan.columns.len() {
            inner.set(&col_var(i), Col::term(format!("{a}.c{i}"), false));
        }
        let mut r = self.derive(
            inner,
            Select {
                limit: Some("-1".to_string()),
                ..Select::default()
            },
            Vec::new(),
        );
        for (n, c) in inputs.iter().zip(&plan.iso_columns) {
            if let (Node::Triple(t), Some(c)) = (n, c) {
                let eid = r.col(&col_var(*c)).map(|c| c.sql.clone());
                if let (Some(group), Some(eid)) = (t.iso_group, eid) {
                    r.iso.push(IsoPat {
                        group,
                        eid,
                        pred: match t.p {
                            PTerm::Id(id) => Some(id),
                            PTerm::Var(_) => None,
                        },
                        optional: false,
                    });
                }
            }
        }
        Ok(r)
    }
}
