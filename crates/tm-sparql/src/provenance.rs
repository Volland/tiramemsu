//! Query provenance (`query-provenance`): which stored statements produced each
//! solution of a `SELECT`.
//!
//! [`instrument`] rewrites a lowered query so that every stored triple pattern
//! binds its eid in a hidden column, and [`ProvenancePlan::assemble`] turns the
//! executor's rows into [`Solutions`] with one sorted eid list per row. The
//! solutions themselves never change: a hidden eid keeps the set semantics of an
//! unbound one ([`tm_ir::var::PROVENANCE_PREFIX`]), and the other eids with the
//! same `(s, p, o)` are found by a sibling lookup after the main query.
//!
//! What counts: the patterns that produced the row, including matched `OPTIONAL`
//! parts, the `UNION` branch taken, `GRAPH` memberships, annotations, fixed-length
//! paths and `SERVICE` time scopes. What does not: statements only tested by
//! `FILTER EXISTS`, `NOT EXISTS` or `MINUS` (expressions are never walked),
//! virtual predicates and recursive path regions.

// @lat: [[query#Front Ends#SPARQL#Query Provenance]]

use std::collections::{BTreeSet, HashMap};

use tm_core::{Eid, Result, Value};
use tm_ir::validate::output_vars;
use tm_ir::var::PROVENANCE_PREFIX;
use tm_ir::{
    Agg, AggFunc, Aggregate, Expr, GraphSel, IrQuery, Join, Op, OrderLimit, Project, TermOrVar,
    TriplePattern, Values, Var, View,
};

use crate::error::{unsupported, PROVENANCE_ASK, PROVENANCE_CONSTRUCT, PROVENANCE_EMPTY_DISTINCT};
use crate::lower::vars::is_internal;
use crate::lower::{QueryForm, QueryPlan};
use crate::results::Solutions;

/// Canonical eids per sibling lookup (one `VALUES` row each).
const SIBLING_BATCH: usize = 500;

/// One hidden provenance column of an instrumented query.
#[derive(Clone, Debug, PartialEq)]
pub struct ProvColumn {
    /// The column's variable.
    pub var: Var,
    /// The view of the pattern that bound it (the sibling lookup reads it).
    pub view: View,
    /// The eid is the canonical eid of its `(s, p, o)`: every visible eid with the
    /// same `(s, p, o)` counts.
    pub siblings: bool,
    /// The cell is a `GROUP_CONCAT` list of statement IRIs, not one statement.
    pub list: bool,
}

/// An instrumented `SELECT`: the query to run and how to read its rows back.
///
/// Build it with [`instrument`], run [`ProvenancePlan::query`] on any executor,
/// and pass the rows (every column, as [`Solutions`]) to
/// [`ProvenancePlan::assemble`].
#[derive(Clone, Debug, PartialEq)]
pub struct ProvenancePlan {
    /// The query to execute: the lowered query plus the hidden columns.
    pub query: IrQuery,
    /// The user-visible result variables, in projection order.
    pub vars: Vec<Var>,
    /// The hidden provenance columns.
    pub columns: Vec<ProvColumn>,
    /// Top-level `DISTINCT`, applied while assembling.
    pub distinct: bool,
    /// Top-level `OFFSET` after `DISTINCT`, applied while assembling.
    pub skip: usize,
    /// Top-level `LIMIT` after `DISTINCT`, applied while assembling.
    pub limit: Option<usize>,
}

/// Rewrites a lowered `SELECT` so that its rows carry provenance.
///
/// # Errors
///
/// `Unsupported` with [`PROVENANCE_ASK`] or [`PROVENANCE_CONSTRUCT`] for the other
/// query forms, and with [`PROVENANCE_EMPTY_DISTINCT`] for a `SELECT DISTINCT` of
/// no variable inside a subquery. Nothing is executed.
///
/// # Example
///
/// ```
/// use tm_ir::View;
/// use tm_sparql::{env::Env, prepare, provenance::instrument, Prepared};
///
/// let env = Env::new(View::NOW);
/// let Prepared::Query(plan) = prepare("SELECT ?o WHERE { v:a v:p ?o }", &env)? else {
///     unreachable!()
/// };
/// let p = instrument(&plan)?;
/// assert_eq!(p.vars.len(), 1); // ?o only; the hidden eid column is extra
/// assert_eq!(p.columns.len(), 1);
/// assert!(p.columns[0].siblings);
///
/// let Prepared::Query(ask) = prepare("ASK { v:a v:p ?o }", &env)? else {
///     unreachable!()
/// };
/// assert!(instrument(&ask).is_err());
/// # Ok::<(), tm_core::Error>(())
/// ```
pub fn instrument(plan: &QueryPlan) -> Result<ProvenancePlan> {
    match plan.form {
        QueryForm::Select => {}
        QueryForm::Ask => return Err(unsupported(PROVENANCE_ASK)),
        QueryForm::Construct { .. } => return Err(unsupported(PROVENANCE_CONSTRUCT)),
    }
    let vars = output_vars(&plan.query.root);
    let mut pass = Pass::default();
    let root = pass.root(plan.query.root.clone())?;
    Ok(ProvenancePlan {
        query: IrQuery::new(root, plan.query.semantics),
        vars,
        columns: pass.columns,
        distinct: pass.distinct,
        skip: pass.skip,
        limit: pass.limit,
    })
}

/// The state of one instrumentation run.
#[derive(Default)]
struct Pass {
    next: u32,
    columns: Vec<ProvColumn>,
    distinct: bool,
    skip: usize,
    limit: Option<usize>,
}

/// Adds the columns of `more` that `cols` does not have yet (a variable bound on
/// both sides of a join or in two union branches is one column).
fn merge(cols: &mut Vec<ProvColumn>, more: Vec<ProvColumn>) {
    for c in more {
        if !cols.iter().any(|x| x.var == c.var) {
            cols.push(c);
        }
    }
}

/// A constant `OFFSET` or `LIMIT` count.
fn count(t: &TermOrVar) -> Result<usize> {
    match t {
        TermOrVar::Const(Value::Int(n)) if *n >= 0 => Ok(*n as usize),
        other => Err(tm_core::Error::invalid_query(format!(
            "a SPARQL slice is a non-negative integer, not {other:?}"
        ))),
    }
}

impl Pass {
    /// A fresh hidden variable `~<kind><n>`; `kind` is `prov` for eids that keep set
    /// semantics, `pe` for other statement eids, `pa` for aliases and `pg` for
    /// group lists.
    fn fresh(&mut self, kind: &str) -> Var {
        let v = Var::new(format!("~{kind}{}", self.next));
        self.next += 1;
        v
    }

    fn fresh_prov(&mut self) -> Var {
        let v = Var::new(format!("{PROVENANCE_PREFIX}{}", self.next));
        self.next += 1;
        v
    }

    /// The root: a top-level `DISTINCT` and the slice above it move to assembly, so
    /// rows are merged before they are counted.
    fn root(&mut self, op: Op) -> Result<Op> {
        let (op, cols) = match op {
            Op::OrderLimit(ol) if matches!(&*ol.input, Op::Project(p) if p.distinct) => {
                let Op::Project(p) = *ol.input else {
                    unreachable!("matched above")
                };
                self.distinct = true;
                self.skip = ol.skip.as_ref().map(count).transpose()?.unwrap_or(0);
                self.limit = ol.limit.as_ref().map(count).transpose()?;
                let (inner, cols) = self.project(p, true)?;
                let op = if ol.keys.is_empty() {
                    inner
                } else {
                    Op::OrderLimit(OrderLimit {
                        input: Box::new(inner),
                        keys: ol.keys,
                        skip: None,
                        limit: None,
                    })
                };
                (op, cols)
            }
            Op::Project(p) => {
                self.distinct = p.distinct;
                self.project(p, true)?
            }
            other => self.op(other)?,
        };
        self.columns = cols;
        Ok(op)
    }

    /// A projection keeps the provenance columns of its input. Below the root, a
    /// user eid variable is aliased first (projecting `?r` would change the scope
    /// of the outer query), and `DISTINCT` becomes a group over the projected
    /// variables so that merged rows union their provenance in SQL.
    fn project(&mut self, p: Project, top: bool) -> Result<(Op, Vec<ProvColumn>)> {
        let (input, cols) = self.op(*p.input)?;
        let (input, cols) = if top {
            (input, cols)
        } else {
            self.alias_user_vars(input, cols)
        };
        let (input, cols) = if p.distinct && !top {
            if p.vars.is_empty() {
                return Err(unsupported(PROVENANCE_EMPTY_DISTINCT));
            }
            self.group(input, p.vars.clone(), Vec::new(), cols)
        } else {
            (input, cols)
        };
        let mut vars = p.vars;
        for c in &cols {
            if !vars.contains(&c.var) {
                vars.push(c.var.clone());
            }
        }
        let op = Op::Project(Project {
            input: Box::new(input),
            vars,
            distinct: false,
        });
        Ok((op, cols))
    }

    /// Replaces each user-named column by a fresh hidden alias.
    fn alias_user_vars(&mut self, mut op: Op, cols: Vec<ProvColumn>) -> (Op, Vec<ProvColumn>) {
        let mut out = Vec::with_capacity(cols.len());
        for c in cols {
            if is_internal(&c.var) {
                out.push(c);
                continue;
            }
            let alias = self.fresh("pa");
            op = op.extend(alias.name(), Expr::Var(c.var.clone()));
            out.push(ProvColumn { var: alias, ..c });
        }
        (op, out)
    }

    /// An aggregate over `input`: each provenance column becomes the list of its
    /// values over the group.
    fn group(
        &mut self,
        input: Op,
        group: Vec<Var>,
        mut aggs: Vec<Agg>,
        cols: Vec<ProvColumn>,
    ) -> (Op, Vec<ProvColumn>) {
        let mut out = Vec::with_capacity(cols.len());
        for c in cols {
            let v = self.fresh("pg");
            aggs.push(Agg::new(
                v.name(),
                AggFunc::GroupConcat {
                    sep: " ".to_string(),
                },
                Expr::Var(c.var.clone()),
            ));
            out.push(ProvColumn {
                var: v,
                list: true,
                ..c
            });
        }
        let op = Op::Aggregate(Aggregate {
            input: Box::new(input),
            group,
            aggs,
        });
        (op, out)
    }

    /// Instruments `op` and returns its provenance columns. Expressions are not
    /// walked: statements tested by `EXISTS` or `MINUS` do not count.
    fn op(&mut self, op: Op) -> Result<(Op, Vec<ProvColumn>)> {
        Ok(match op {
            Op::Triple(t) => self.triple(t),
            op @ (Op::Path(_) | Op::Text(_) | Op::Values(_)) => (op, Vec::new()),
            Op::Join(j) => {
                let mut inputs = Vec::with_capacity(j.inputs.len());
                let mut cols = Vec::new();
                for i in j.inputs {
                    let (i, c) = self.op(i)?;
                    inputs.push(i);
                    merge(&mut cols, c);
                }
                let op = Op::Join(Join {
                    inputs,
                    null_safe: j.null_safe,
                });
                (op, cols)
            }
            Op::Union(u) => {
                let mut inputs = Vec::with_capacity(u.inputs.len());
                let mut cols = Vec::new();
                for i in u.inputs {
                    let (i, c) = self.op(i)?;
                    inputs.push(i);
                    merge(&mut cols, c);
                }
                (Op::union(inputs), cols)
            }
            Op::LeftJoin(mut l) => {
                let (left, mut cols) = self.op(*l.left)?;
                let (right, more) = self.op(*l.right)?;
                merge(&mut cols, more);
                l.left = Box::new(left);
                l.right = Box::new(right);
                (Op::LeftJoin(l), cols)
            }
            Op::Filter(mut f) => {
                let (input, cols) = self.op(*f.input)?;
                f.input = Box::new(input);
                (Op::Filter(f), cols)
            }
            Op::Extend(mut e) => {
                let (input, cols) = self.op(*e.input)?;
                e.input = Box::new(input);
                (Op::Extend(e), cols)
            }
            Op::OrderLimit(mut o) => {
                let (input, cols) = self.op(*o.input)?;
                o.input = Box::new(input);
                (Op::OrderLimit(o), cols)
            }
            Op::RowNumber(mut r) => {
                let (input, cols) = self.op(*r.input)?;
                r.input = Box::new(input);
                (Op::RowNumber(r), cols)
            }
            Op::Unnest(mut u) => {
                let (input, cols) = self.op(*u.input)?;
                u.input = Box::new(input);
                (Op::Unnest(u), cols)
            }
            Op::Aggregate(a) => {
                let (input, cols) = self.op(*a.input)?;
                self.group(input, a.group, a.aggs, cols)
            }
            Op::Project(p) => self.project(p, false)?,
        })
    }

    /// A stored triple pattern binds its eid; a graph selection of one graph or a
    /// variable also binds the eid of the membership statement (the same join as
    /// `tm-exec`'s graph lowering, with the membership eid bound).
    fn triple(&mut self, mut t: TriplePattern) -> (Op, Vec<ProvColumn>) {
        let virtual_pred =
            matches!(&t.p, TermOrVar::Const(Value::Iri(p)) if tm_ir::vocab::is_virtual(p));
        if virtual_pred {
            // reads columns of the subject statement, whose own pattern counts
            return (Op::Triple(t), Vec::new());
        }
        let view = t.view;
        let col = |var: Var, siblings: bool| ProvColumn {
            var,
            view,
            siblings,
            list: false,
        };
        let one_graph = match &t.graph {
            GraphSel::Any => {
                let c = match &t.eid {
                    Some(e) => col(e.clone(), false),
                    None => {
                        let e = self.fresh_prov();
                        t.eid = Some(e.clone());
                        col(e, true)
                    }
                };
                return (Op::Triple(t), vec![c]);
            }
            GraphSel::Var(g) => TermOrVar::Var(g.clone()),
            GraphSel::Set(gs) => match gs.as_slice() {
                [g] => g.clone(),
                _ => {
                    // several graphs: the membership is an existence test
                    let e = match &t.eid {
                        Some(e) => e.clone(),
                        None => {
                            let e = self.fresh("pe");
                            t.eid = Some(e.clone());
                            e
                        }
                    };
                    return (Op::Triple(t), vec![col(e, false)]);
                }
            },
        };
        t.graph = GraphSel::Any;
        // not a `~prov` name: like the graph lowering's own eid, it turns the
        // canonical-eid predicate off for the statement pattern
        let e = match &t.eid {
            Some(e) => e.clone(),
            None => {
                let e = self.fresh("pe");
                t.eid = Some(e.clone());
                e
            }
        };
        let m = self.fresh_prov();
        let mut member = TriplePattern::new(
            TermOrVar::Var(e.clone()),
            TermOrVar::iri(tm_ir::vocab::SYS_IN_GRAPH),
            one_graph,
            view,
        );
        member.eid = Some(m.clone());
        let op = Op::join(vec![Op::Triple(t), Op::Triple(member)]);
        (op, vec![col(e, false), col(m, true)])
    }
}

/// The statement eids of one provenance cell: a statement, or a `GROUP_CONCAT`
/// list of statement IRIs.
fn cell_eids(cell: &Option<Value>, out: &mut Vec<Eid>) {
    match cell {
        Some(Value::Stmt(e)) => out.push(*e),
        Some(Value::Str(list)) => {
            for iri in list.split_whitespace() {
                if let Value::Stmt(e) = Value::Iri(iri.to_string()).canonical() {
                    out.push(e);
                }
            }
        }
        _ => {}
    }
}

/// The sibling lookup of `eids` under `view`: each canonical eid `?c` with every
/// visible eid `?e` of the same `(s, p, o)`, itself included.
fn sibling_query(view: View, eids: &[Eid]) -> IrQuery {
    let [c, e, s, p, o] = ["c", "e", "s", "p", "o"].map(Var::new);
    let pattern = |eid: &Var| {
        let mut t = TriplePattern::new(
            TermOrVar::Var(s.clone()),
            TermOrVar::Var(p.clone()),
            TermOrVar::Var(o.clone()),
            view,
        );
        t.eid = Some(eid.clone());
        Op::Triple(t)
    };
    let values = Op::Values(Values {
        vars: vec![c.clone()],
        rows: eids
            .iter()
            .map(|x| vec![Some(TermOrVar::Const(Value::Stmt(*x)))])
            .collect(),
    });
    let root = Op::Project(Project {
        input: Box::new(Op::join(vec![values, pattern(&c), pattern(&e)])),
        vars: vec![c.clone(), e.clone()],
        distinct: false,
    });
    IrQuery::sparql(root)
}

impl ProvenancePlan {
    /// Turns the rows of [`ProvenancePlan::query`] into the user's solutions with
    /// provenance. `raw` holds every result column (visible and hidden); `run`
    /// executes the sibling lookups (at most one per distinct view and batch of
    /// canonical eids) on the same view source.
    ///
    /// # Errors
    ///
    /// The errors of `run`, and `InvalidQuery` when `raw` lacks a result column.
    pub fn assemble(
        &self,
        raw: Solutions,
        run: &mut dyn FnMut(&IrQuery) -> Result<Solutions>,
    ) -> Result<Solutions> {
        let find = |v: &Var| {
            raw.col(v.name()).ok_or_else(|| {
                tm_core::Error::invalid_query(format!("the result has no column {v}"))
            })
        };
        let visible: Vec<usize> = self.vars.iter().map(find).collect::<Result<_>>()?;
        let hidden: Vec<(&ProvColumn, usize)> = self
            .columns
            .iter()
            .map(|c| Ok((c, find(&c.var)?)))
            .collect::<Result<_>>()?;
        let siblings = self.siblings(&raw, &hidden, run)?;
        let mut rows: Vec<Vec<Option<Value>>> = Vec::new();
        let mut prov: Vec<BTreeSet<Eid>> = Vec::new();
        let mut first: HashMap<String, usize> = HashMap::new();
        let mut eids = Vec::new();
        for row in raw.rows {
            let mut set = BTreeSet::new();
            for (c, i) in &hidden {
                eids.clear();
                cell_eids(&row[*i], &mut eids);
                for e in &eids {
                    match siblings.get(&(c.view, *e)) {
                        Some(all) if c.siblings => set.extend(all.iter().copied()),
                        _ => {
                            set.insert(*e);
                        }
                    }
                }
            }
            let cells: Vec<Option<Value>> = visible.iter().map(|i| row[*i].clone()).collect();
            if self.distinct {
                // rows equal on the projected cells merge into the first one
                let key = format!("{cells:?}");
                if let Some(&k) = first.get(&key) {
                    prov[k].extend(set);
                    continue;
                }
                first.insert(key, rows.len());
            }
            rows.push(cells);
            prov.push(set);
        }
        let end = self
            .limit
            .map_or(rows.len(), |l| self.skip.saturating_add(l).min(rows.len()));
        let start = self.skip.min(end);
        Ok(Solutions {
            vars: self.vars.iter().map(|v| v.name().to_string()).collect(),
            rows: rows.drain(start..end).collect(),
            provenance: Some(
                prov.drain(start..end)
                    .map(|s| s.into_iter().collect())
                    .collect(),
            ),
        })
    }

    /// Every visible eid sharing the `(s, p, o)` of each canonical eid in `raw`,
    /// keyed by the view it was matched in.
    fn siblings(
        &self,
        raw: &Solutions,
        hidden: &[(&ProvColumn, usize)],
        run: &mut dyn FnMut(&IrQuery) -> Result<Solutions>,
    ) -> Result<HashMap<(View, Eid), Vec<Eid>>> {
        let mut wanted: Vec<(View, BTreeSet<Eid>)> = Vec::new();
        let mut eids = Vec::new();
        for (c, i) in hidden.iter().filter(|(c, _)| c.siblings) {
            let slot = match wanted.iter().position(|(v, _)| *v == c.view) {
                Some(k) => k,
                None => {
                    wanted.push((c.view, BTreeSet::new()));
                    wanted.len() - 1
                }
            };
            for row in &raw.rows {
                eids.clear();
                cell_eids(&row[*i], &mut eids);
                wanted[slot].1.extend(eids.iter().copied());
            }
        }
        let mut out: HashMap<(View, Eid), Vec<Eid>> = HashMap::new();
        for (view, set) in wanted {
            let set: Vec<Eid> = set.into_iter().collect();
            for batch in set.chunks(SIBLING_BATCH) {
                let r = run(&sibling_query(view, batch))?;
                let (Some(ci), Some(ei)) = (r.col("c"), r.col("e")) else {
                    continue;
                };
                for row in &r.rows {
                    if let (Some(Value::Stmt(c)), Some(Value::Stmt(e))) = (&row[ci], &row[ei]) {
                        out.entry((view, *c)).or_default().push(*e);
                    }
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::Env;
    use crate::{prepare, Prepared};

    fn plan(text: &str) -> Result<ProvenancePlan> {
        let env = Env::new(View::NOW);
        let Prepared::Query(q) = prepare(text, &env)? else {
            panic!("not a query")
        };
        instrument(&q)
    }

    fn names(p: &ProvenancePlan) -> Vec<String> {
        p.columns.iter().map(|c| c.var.name().to_string()).collect()
    }

    fn stmt(n: u64) -> Option<Value> {
        Some(Value::Stmt(Eid::new(n)))
    }

    fn iri(s: &str) -> Option<Value> {
        Some(Value::iri(format!("urn:tiramemsu:v:{s}")))
    }

    // query-provenance "Matched statements count": hidden columns per pattern
    // @lat: [[tests#Query Provenance#Instrumentation Binds Hidden Eids]]
    #[test]
    fn hidden_columns_per_pattern() {
        let p = plan("SELECT ?o WHERE { v:a v:p ?o OPTIONAL { ?o v:q ?z } }").unwrap();
        assert_eq!(names(&p), ["~prov0", "~prov1"]);
        assert!(p.columns.iter().all(|c| c.siblings && !c.list));
        assert_eq!(p.vars, [Var::new("o")]);
        // a reifier's eid is its own column and keeps one row per eid
        let p = plan("SELECT ?c WHERE { v:a v:p ?c ~ ?r }").unwrap();
        assert_eq!(names(&p), ["r"]);
        assert!(!p.columns[0].siblings);
        // EXISTS, MINUS and virtual predicates add nothing
        let p = plan(
            "SELECT ?o WHERE { v:a v:p ?o ~ ?r . ?r tm:txAdded ?t \
             FILTER EXISTS { ?o v:q ?z } MINUS { ?o v:s ?w } }",
        )
        .unwrap();
        assert_eq!(names(&p), ["r"]);
    }

    // query-provenance "Time scopes and graphs": the membership eid is a column
    #[test]
    fn graph_patterns_bind_the_membership() {
        let p = plan("SELECT ?c WHERE { GRAPH <urn:g1> { v:a v:p ?c } }").unwrap();
        assert_eq!(names(&p), ["~pe0", "~prov1"]);
        assert!(!p.columns[0].siblings && p.columns[1].siblings);
        let text = p.query.root.to_string();
        assert!(text.contains("inGraph"), "{text}");
        // several FROM graphs: the statement only
        let p = plan("SELECT ?c FROM <urn:g1> FROM <urn:g2> WHERE { v:a v:p ?c }").unwrap();
        assert_eq!(names(&p), ["~pe0"]);
    }

    // query-provenance "Modifiers, aggregates and subqueries": plan shapes
    #[test]
    fn modifiers_and_groups() {
        let p =
            plan("SELECT DISTINCT ?s WHERE { ?s v:p ?o } ORDER BY ?s LIMIT 2 OFFSET 1").unwrap();
        assert!(p.distinct);
        assert_eq!((p.skip, p.limit), (1, Some(2)));
        assert!(
            matches!(&p.query.root, Op::OrderLimit(o) if o.limit.is_none() && o.skip.is_none())
        );
        let p = plan("SELECT ?s (COUNT(*) AS ?n) WHERE { ?s v:p ?o } GROUP BY ?s").unwrap();
        assert!(p.columns.iter().all(|c| c.list));
        let p = plan("SELECT ?s WHERE { { SELECT DISTINCT ?s WHERE { ?s v:p ?o } } }").unwrap();
        assert!(!p.distinct);
        assert!(p.columns.iter().all(|c| c.list));
        // a user eid leaving a subquery is aliased
        let p = plan("SELECT ?s WHERE { { SELECT ?s WHERE { ?s v:p ?o ~ ?r } } }").unwrap();
        assert!(names(&p)[0].starts_with("~pa"));
    }

    // query-provenance "Unsupported combinations fail before execution"
    #[test]
    fn unsupported_forms() {
        for (q, want) in [
            ("ASK { ?s ?p ?o }", PROVENANCE_ASK),
            (
                "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }",
                PROVENANCE_CONSTRUCT,
            ),
        ] {
            assert!(matches!(
                plan(q),
                Err(tm_core::Error::Unsupported { feature }) if feature == want
            ));
        }
    }

    // assembly: sibling expansion, lists, DISTINCT merge and the slice
    #[test]
    fn assemble_rows() {
        let mut p = plan("SELECT DISTINCT ?s WHERE { ?s v:p ?o }").unwrap();
        p.skip = 1;
        p.limit = Some(1);
        let h = p.columns[0].var.name().to_string();
        let raw = Solutions {
            vars: vec!["s".into(), h],
            rows: vec![
                vec![iri("a"), stmt(1)],
                vec![iri("b"), stmt(3)],
                vec![iri("a"), stmt(2)],
                vec![iri("c"), stmt(4)],
            ],
            provenance: None,
        };
        let mut calls = 0;
        let sol = p
            .assemble(raw, &mut |q| {
                calls += 1;
                assert!(matches!(&q.root, Op::Project(p) if p.vars.len() == 2));
                // eid 3 has a sibling 5 with the same (s, p, o)
                Ok(Solutions {
                    vars: vec!["c".into(), "e".into()],
                    rows: vec![
                        vec![stmt(1), stmt(1)],
                        vec![stmt(2), stmt(2)],
                        vec![stmt(3), stmt(3)],
                        vec![stmt(3), stmt(5)],
                        vec![stmt(4), stmt(4)],
                    ],
                    provenance: None,
                })
            })
            .unwrap();
        assert_eq!(calls, 1);
        assert_eq!(sol.vars, ["s"]);
        assert_eq!(sol.rows, vec![vec![iri("b")]]);
        assert_eq!(sol.provenance(0), Some(&[Eid::new(3), Eid::new(5)][..]));
        // a group list cell parses back
        let mut out = Vec::new();
        cell_eids(
            &Some(Value::str("urn:tiramemsu:stmt:7 urn:tiramemsu:stmt:2")),
            &mut out,
        );
        assert_eq!(out, [Eid::new(7), Eid::new(2)]);
    }
}
