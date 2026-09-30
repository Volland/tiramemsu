//! Join, LeftJoin, Filter, Union, Values, Unnest and Aggregate (design D8, D11,
//! D12).

use std::collections::BTreeSet;

use tm_core::{Result, SqlValue};
use tm_ir::{AggFunc, Missing, Var};

use super::{Col, Gen, IsoPat, Item, Rel, Select};
use crate::plan::analyze::{Dom, VClass};
use crate::plan::route::route_bgp;
use crate::plan::{Cell, Node, PAgg, PExpr, PTerm, PValues};
use crate::result::{RegionKind, RouteNote};

fn leaf(n: &Node) -> bool {
    matches!(n, Node::Triple(_) | Node::Virtual(_) | Node::Volatile(_))
}

fn vars_of(n: &Node) -> BTreeSet<Var> {
    let mut out = BTreeSet::new();
    let mut add = |t: &PTerm| {
        if let PTerm::Var(v) = t {
            out.insert(v.clone());
        }
    };
    match n {
        Node::Triple(t) => {
            add(&t.s);
            add(&t.p);
            add(&t.o);
            if let Some(e) = &t.eid {
                out.insert(e.clone());
            }
        }
        Node::Virtual(v) => {
            add(&v.subject);
            if let crate::plan::PObj::Var(o) = &v.object {
                out.insert(o.clone());
            }
        }
        Node::Volatile(v) => {
            add(&v.s);
            add(&v.o);
        }
        _ => {}
    }
    out
}

impl Gen<'_> {
    /// The join condition of a shared variable and the merged column (design D11):
    /// under `Unbound`, a maybe-missing side is compatible with anything and the
    /// bound value wins; under `Null3VL` a missing value joins nothing.
    pub fn join_eq(&mut self, a: &Col, b: &Col, null_safe: bool) -> Result<(String, Col)> {
        let base = if a.dom == b.dom {
            format!("{} = {}", a.sql, b.sql)
        } else {
            self.value_eq_cols(a, b)?
        };
        if null_safe {
            let cond = if a.dom == b.dom {
                format!("{} IS {}", a.sql, b.sql)
            } else {
                format!(
                    "(({a} IS NULL AND {b} IS NULL) OR {base})",
                    a = a.sql,
                    b = b.sql
                )
            };
            let merged = Col {
                sql: format!("COALESCE({}, {})", a.sql, b.sql),
                dom: a.dom.clone(),
                mm: a.mm && b.mm,
                eid_of: None,
            };
            return Ok((cond, merged));
        }
        if (a.mm || b.mm) && self.sem.missing == Missing::Unbound {
            let mut ors = Vec::new();
            if a.mm {
                ors.push(format!("{} IS NULL", a.sql));
            }
            if b.mm {
                ors.push(format!("{} IS NULL", b.sql));
            }
            ors.push(base);
            let merged = if a.mm {
                Col {
                    sql: format!("COALESCE({}, {})", a.sql, b.sql),
                    dom: a.dom.clone(),
                    mm: a.mm && b.mm,
                    eid_of: None,
                }
            } else {
                a.clone()
            };
            return Ok((format!("({})", ors.join(" OR ")), merged));
        }
        Ok((
            base,
            Col {
                mm: false,
                ..a.clone()
            },
        ))
    }

    /// Merges `r` into `acc` as an inner join.
    fn merge(&mut self, acc: &mut Rel, r: Rel, null_safe: &[Var]) -> Result<()> {
        acc.items.extend(r.items);
        acc.conds.extend(r.conds);
        for b in &r.iso {
            for a in &acc.iso {
                if a.group != b.group {
                    continue;
                }
                if let (Some(p), Some(q)) = (a.pred, b.pred) {
                    if p != q {
                        continue;
                    }
                }
                let ne = format!("{}.eid <> {}.eid", a.alias, b.alias);
                acc.conds.push(if a.optional || b.optional {
                    format!(
                        "({x}.eid IS NULL OR {y}.eid IS NULL OR {ne})",
                        x = a.alias,
                        y = b.alias
                    )
                } else {
                    ne
                });
            }
        }
        acc.iso.extend(r.iso);
        for (v, c) in r.cols {
            match acc.col(&v).cloned() {
                Some(prev) => {
                    let (cond, merged) = self.join_eq(&prev, &c, null_safe.contains(&v))?;
                    acc.conds.push(cond);
                    acc.set(&v, merged);
                }
                None => acc.set(&v, c),
            }
        }
        Ok(())
    }

    /// A natural join. Ordinary inputs compile first, then virtual patterns (so
    /// they can reuse the alias of their subject's eid pattern), then paths (whose
    /// start must already be bound).
    pub fn join(&mut self, inputs: &[Node], null_safe: &[Var]) -> Result<Rel> {
        let pure = !inputs.is_empty() && inputs.iter().all(leaf);
        let saved = self.bgp_ctx;
        if pure {
            let edges: Vec<BTreeSet<Var>> = inputs
                .iter()
                .filter(|n| matches!(n, Node::Triple(_)))
                .map(vars_of)
                .collect();
            let note = route_bgp(&edges, self.opts, self.reg);
            self.bgp_ctx = Some(self.new_region(RegionKind::Sql, note));
        } else if inputs.iter().any(leaf) {
            // patterns joined with non-leaf inputs (paths, ...) still form one SQL region
            self.bgp_ctx = Some(self.new_region(RegionKind::Sql, RouteNote::None));
        } else {
            self.bgp_ctx = None;
        }
        let mut acc = Rel::default();
        let r: Result<()> = (|| {
            for n in inputs {
                if !matches!(n, Node::Virtual(_) | Node::Path(_)) {
                    let r = self.compile(n)?;
                    self.merge(&mut acc, r, null_safe)?;
                }
            }
            for n in inputs {
                if let Node::Virtual(v) = n {
                    let reuse = match &v.subject {
                        PTerm::Var(s) => acc.col(s).and_then(|c| match &c.eid_of {
                            Some((alias, view)) if *view == v.view => Some(alias.clone()),
                            _ => None,
                        }),
                        PTerm::Id(_) => None,
                    };
                    let r = self.virtual_pattern(v, reuse.as_deref())?;
                    self.merge(&mut acc, r, null_safe)?;
                }
            }
            // a path starts from a bound term or column; one whose start is another
            // path's end waits for that path
            let mut pending: Vec<&crate::plan::PPath> = inputs
                .iter()
                .filter_map(|n| match n {
                    Node::Path(p) => Some(p),
                    _ => None,
                })
                .collect();
            while !pending.is_empty() {
                let ready = pending.iter().position(|p| match &p.arg {
                    PTerm::Id(_) => true,
                    PTerm::Var(v) => acc.col(v).is_some(),
                });
                let p = pending.remove(ready.unwrap_or(0));
                let r = self.path(p, &acc)?;
                self.merge(&mut acc, r, null_safe)?;
            }
            Ok(())
        })();
        self.bgp_ctx = saved;
        r?;
        if acc.items.is_empty() {
            let u = self.unit();
            acc.items = u.items;
        }
        Ok(acc)
    }

    /// `left LEFT JOIN right ON <shared vars> AND <right view preds> AND <cond>`.
    pub fn left_join(&mut self, left: &Node, right: &Node, cond: Option<&PExpr>) -> Result<Rel> {
        let mut l = self.compile(left)?;
        let r = self.compile(right)?;
        let simple = r.items.len() == 1 && r.items[0].on.is_none();
        let (item_sql, mut on, rcols, riso) = if simple {
            let item = r.items.into_iter().next().expect("one item");
            (item.sql, r.conds, r.cols, r.iso)
        } else {
            let d = self.derive(r, Select::default(), Vec::new());
            let item = d.items.into_iter().next().expect("one item");
            (item.sql, Vec::new(), d.cols, Vec::new())
        };
        let mut merged = l.clone();
        for (v, c) in &rcols {
            match l.col(v).cloned() {
                Some(prev) => {
                    let (eq, m) = self.join_eq(&prev, c, false)?;
                    on.push(eq);
                    // no match leaves the right side NULL: keep the left value
                    let keep = if prev.mm && self.sem.missing == Missing::Unbound {
                        Col { mm: true, ..m }
                    } else {
                        prev
                    };
                    merged.set(v, keep);
                }
                None => merged.set(
                    v,
                    Col {
                        mm: true,
                        eid_of: None,
                        ..c.clone()
                    },
                ),
            }
        }
        if let Some(c) = cond {
            // the condition sees both sides' values
            let mut scope = merged.clone();
            for (v, c) in &rcols {
                if l.col(v).is_none() {
                    scope.set(v, c.clone());
                }
            }
            on.push(self.cond(c, &scope, false)?);
        }
        for b in riso {
            for a in &l.iso {
                if a.group == b.group && !matches!((a.pred, b.pred), (Some(p), Some(q)) if p != q) {
                    on.push(format!("{}.eid <> {}.eid", a.alias, b.alias));
                }
            }
            l.iso.push(IsoPat {
                optional: true,
                ..b
            });
        }
        merged.iso = l.iso;
        merged.items.push(Item {
            sql: item_sql,
            on: Some(on),
        });
        Ok(merged)
    }

    /// Filter: conjuncts go to WHERE; directly above an Aggregate they go to
    /// HAVING.
    pub fn filter(&mut self, input: &Node, cond: &PExpr) -> Result<Rel> {
        if let Node::Aggregate(i, group, aggs) = input {
            return self.aggregate(i, group, aggs, Some(cond));
        }
        let mut r = self.compile(input)?;
        let parts = match cond {
            PExpr::And(xs) => xs.clone(),
            other => vec![other.clone()],
        };
        for p in parts {
            let c = self.cond(&p, &r, true)?;
            r.conds.push(c);
        }
        Ok(r)
    }

    /// `(SELECT … UNION ALL SELECT …) AS qN`, variables absent from a branch NULL.
    pub fn union(&mut self, inputs: &[Node]) -> Result<Rel> {
        let rels: Vec<Rel> = inputs
            .iter()
            .map(|n| self.compile(n))
            .collect::<Result<_>>()?;
        let mut vars: Vec<Var> = Vec::new();
        for r in &rels {
            for (v, _) in &r.cols {
                if !vars.contains(v) {
                    vars.push(v.clone());
                }
            }
        }
        let mut cols = Vec::new();
        for v in &vars {
            let doms: Vec<&Dom> = rels
                .iter()
                .filter_map(|r| r.col(v))
                .filter(|c| c.sql != "NULL")
                .map(|c| &c.dom)
                .collect();
            if doms.windows(2).any(|w| w[0] != w[1]) {
                return self.unsupported("a Union column mixing stored terms and computed values");
            }
            let mm = rels.iter().any(|r| r.col(v).is_none_or(|c| c.mm));
            cols.push((doms.first().map_or(Dom::Term, |d| (*d).clone()), mm));
        }
        let mut parts = Vec::new();
        for r in &rels {
            let list = vars
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    (
                        r.col(v).map_or("NULL".to_string(), |c| c.sql.clone()),
                        format!("c{i}"),
                    )
                })
                .collect();
            parts.push(self.select(
                r,
                &Select {
                    list,
                    ..Select::default()
                },
            ));
        }
        let q = self.alias('q');
        let mut out = Rel {
            items: vec![Item {
                sql: format!("({}) AS {q}", parts.join(" UNION ALL ")),
                on: None,
            }],
            ..Rel::default()
        };
        for (i, (v, (dom, mm))) in vars.iter().zip(cols).enumerate() {
            out.set(
                v,
                Col {
                    sql: format!("{q}.c{i}"),
                    dom,
                    mm,
                    eid_of: None,
                },
            );
        }
        Ok(out)
    }

    /// `(SELECT column1 AS c0, … FROM (VALUES (?1, ?2), …)) AS qN`.
    pub fn values(&mut self, v: &PValues) -> Result<Rel> {
        let q = self.alias('q');
        if v.vars.is_empty() {
            let rows = vec!["(1)"; v.rows.len().max(1)].join(", ");
            return Ok(Rel {
                items: vec![Item {
                    sql: format!("(SELECT 1 AS c0 FROM (VALUES {rows})) AS {q}"),
                    on: None,
                }],
                ..Rel::default()
            });
        }
        let mut rows = Vec::new();
        for r in &v.rows {
            let cells: Vec<String> = r
                .iter()
                .map(|c| match c {
                    None => "NULL".to_string(),
                    Some(Cell::Id(id)) => self.params.id(*id),
                    Some(Cell::Int(i)) => self.params.push(SqlValue::Integer(*i)),
                    Some(Cell::EmptyList) => self.params.push(SqlValue::Text("[]".into())),
                })
                .collect();
            rows.push(format!("({})", cells.join(", ")));
        }
        let list: Vec<String> = (0..v.vars.len())
            .map(|i| format!("column{} AS c{i}", i + 1))
            .collect();
        let mut out = Rel {
            items: vec![Item {
                sql: format!(
                    "(SELECT {} FROM (VALUES {})) AS {q}",
                    list.join(", "),
                    rows.join(", ")
                ),
                on: None,
            }],
            ..Rel::default()
        };
        for (i, var) in v.vars.iter().enumerate() {
            let cells: Vec<&Option<Cell>> = v.rows.iter().map(|r| &r[i]).collect();
            let dom = if cells.iter().any(|c| matches!(c, Some(Cell::Int(_)))) {
                Dom::Computed(VClass::Int)
            } else if cells.iter().any(|c| matches!(c, Some(Cell::EmptyList))) {
                Dom::List(Box::new(Dom::Term))
            } else {
                Dom::Term
            };
            out.set(
                var,
                Col {
                    sql: format!("{q}.c{i}"),
                    dom,
                    mm: cells.iter().any(|c| c.is_none()),
                    eid_of: None,
                },
            );
        }
        Ok(out)
    }

    /// One row per list element: `json_each(<list>) AS qN`, correlated.
    pub fn unnest(&mut self, input: &Node, list: &PExpr, var: &Var) -> Result<Rel> {
        let mut r = self.compile(input)?;
        let lv = self.value(list, &r)?;
        let elem = match &lv.dom {
            Dom::List(e) => (**e).clone(),
            Dom::TermOrList | Dom::Term => Dom::Term,
            Dom::Computed(c) => Dom::Computed(*c),
            Dom::PathJson { .. } => Dom::Computed(VClass::Dynamic),
        };
        let sql = self.val_sql(&lv);
        let q = self.alias('q');
        r.items.push(Item {
            sql: format!("json_each({sql}) AS {q}"),
            on: None,
        });
        let c = Col {
            sql: format!("{q}.value"),
            dom: elem,
            mm: false,
            eid_of: None,
        };
        match r.col(var).cloned() {
            Some(prev) => {
                let (cond, m) = self.join_eq(&prev, &c, false)?;
                r.conds.push(cond);
                r.set(var, m);
            }
            None => r.set(var, c),
        }
        Ok(r)
    }

    /// `(SELECT g…, agg… FROM … GROUP BY g… [HAVING …]) AS qN` (design D12).
    pub fn aggregate(
        &mut self,
        input: &Node,
        group: &[Var],
        aggs: &[PAgg],
        having: Option<&PExpr>,
    ) -> Result<Rel> {
        let r = self.compile(input)?;
        let mut out_cols: Vec<(Var, Col)> = Vec::new();
        let mut group_by = Vec::new();
        for g in group {
            let c = r.col(g).cloned().unwrap_or_else(Col::null);
            if c.sql != "NULL" {
                group_by.push(c.sql.clone());
            }
            out_cols.push((g.clone(), Col { eid_of: None, ..c }));
        }
        for a in aggs {
            let col = self.agg(a, &r)?;
            out_cols.push((a.var.clone(), col));
        }
        let mut having_sql = Vec::new();
        if let Some(h) = having {
            let scope = Rel {
                cols: out_cols.clone(),
                ..r.clone()
            };
            having_sql.push(self.cond(h, &scope, true)?);
        }
        let list: Vec<(String, String)> = out_cols
            .iter()
            .enumerate()
            .map(|(i, (_, c))| (c.sql.clone(), format!("c{i}")))
            .collect();
        let body = self.select(
            &r,
            &Select {
                list,
                group_by,
                having: having_sql,
                ..Select::default()
            },
        );
        let q = self.alias('q');
        let mut out = Rel {
            items: vec![Item {
                sql: format!("({body}) AS {q}"),
                on: None,
            }],
            ..Rel::default()
        };
        for (i, (v, c)) in out_cols.into_iter().enumerate() {
            out.set(
                &v,
                Col {
                    sql: format!("{q}.c{i}"),
                    ..c
                },
            );
        }
        Ok(out)
    }

    fn agg(&mut self, a: &PAgg, r: &Rel) -> Result<Col> {
        let d = if a.distinct { "DISTINCT " } else { "" };
        let computed = |sql: String, c: VClass, mm: bool| Col {
            sql,
            dom: Dom::Computed(c),
            mm,
            eid_of: None,
        };
        let Some(arg) = &a.arg else {
            return Ok(computed("count(*)".to_string(), VClass::Int, false));
        };
        let v = self.value(arg, r)?;
        let x = self.val_sql(&v);
        Ok(match &a.func {
            AggFunc::Count => computed(format!("count({d}{x})"), VClass::Int, false),
            AggFunc::Sum => {
                let n = self.num_of(&v, &x);
                let c = match v.dom {
                    Dom::Computed(VClass::Int) => VClass::Int,
                    Dom::Computed(VClass::Double) => VClass::Double,
                    _ => VClass::Dynamic,
                };
                computed(format!("sum({d}{n})"), c, true)
            }
            AggFunc::Avg => {
                let n = self.num_of(&v, &x);
                computed(format!("avg({d}{n})"), VClass::Double, true)
            }
            AggFunc::Min | AggFunc::Max => {
                let f = if a.func == AggFunc::Min {
                    "tm_min_by"
                } else {
                    "tm_max_by"
                };
                let key = self.key_of(&v, &x);
                Col {
                    sql: format!("{f}({x}, {key})"),
                    dom: v.dom.clone(),
                    mm: true,
                    eid_of: None,
                }
            }
            AggFunc::Sample => Col {
                sql: format!("min({x})"),
                dom: v.dom.clone(),
                mm: true,
                eid_of: None,
            },
            AggFunc::GroupConcat { sep } => {
                if a.distinct {
                    return self.unsupported("group_concat(DISTINCT …) with a separator");
                }
                let s = self.str_of(&v, &x);
                let p = self.params.push(SqlValue::Text(sep.clone()));
                computed(format!("group_concat({s}, {p})"), VClass::Str, true)
            }
            AggFunc::Collect => Col {
                sql: format!("json_group_array({d}{x}) FILTER (WHERE {x} IS NOT NULL)"),
                dom: Dom::List(Box::new(v.dom.clone())),
                mm: false,
                eid_of: None,
            },
        })
    }

    /// Registers a native path region.
    pub(crate) fn path_region(&mut self, alias: &str, note: RouteNote) {
        let rid = self.new_region(RegionKind::NativePath, note);
        self.regions[rid].aliases.push(alias.to_string());
    }
}
