//! Parameter substitution: every `$name` becomes the constant supplied at
//! execution, before any SQL runs. A referenced but missing parameter fails with
//! `InvalidQuery` naming it.

use tm_core::{Result, Value};
use tm_ir::validate::check_count;
use tm_ir::{Expr, GraphSel, IrQuery, Op, Params, TermOrVar, TriplePattern, Var};

use crate::error::invalid;

struct Binder<'a> {
    params: &'a Params,
    /// Numbers the internal eid variables of graph joins.
    fresh: std::cell::Cell<u32>,
}

/// Prefix of the internal eid variables a graph selector introduces (`~` cannot
/// start a user variable, and the SPARQL front end uses one-letter kinds).
const GRAPH_EID_PREFIX: &str = "~gsel";
const GRAPH_VAR_PREFIX: &str = "~gvar";

impl Binder<'_> {
    fn value(&self, name: &str) -> Result<Value> {
        self.params
            .get(name)
            .cloned()
            .ok_or_else(|| invalid(format!("missing parameter `{name}`")))
    }

    fn term(&self, t: &TermOrVar) -> Result<TermOrVar> {
        Ok(match t {
            TermOrVar::Param(p) => TermOrVar::Const(self.value(p)?),
            other => other.clone(),
        })
    }

    fn expr(&self, e: &Expr) -> Result<Expr> {
        let b = |x: &Expr| self.expr(x).map(Box::new);
        let v = |xs: &[Expr]| xs.iter().map(|x| self.expr(x)).collect::<Result<Vec<_>>>();
        Ok(match e {
            Expr::Param(p) => Expr::Const(self.value(p)?),
            Expr::Var(_) | Expr::Const(_) | Expr::Bound(_) => e.clone(),
            Expr::Cmp(op, a, c) => Expr::Cmp(*op, b(a)?, b(c)?),
            Expr::SameTerm(a, c) => Expr::SameTerm(b(a)?, b(c)?),
            Expr::And(xs) => Expr::And(v(xs)?),
            Expr::Or(xs) => Expr::Or(v(xs)?),
            Expr::Not(a) => Expr::Not(b(a)?),
            Expr::In(a, xs, n) => Expr::In(b(a)?, v(xs)?, *n),
            Expr::Arith(op, a, c) => Expr::Arith(*op, b(a)?, b(c)?),
            Expr::Neg(a) => Expr::Neg(b(a)?),
            Expr::Coalesce(xs) => Expr::Coalesce(v(xs)?),
            Expr::If(a, c, d) => Expr::If(b(a)?, b(c)?, b(d)?),
            Expr::Func(f, xs) => Expr::Func(*f, v(xs)?),
            Expr::Exists(op, n) => Expr::Exists(Box::new(self.op(op)?), *n),
            Expr::Lookup(l) => {
                let mut l = l.clone();
                l.subject = b(&l.subject)?;
                l.pred = self.term(&l.pred)?;
                Expr::Lookup(l)
            }
            Expr::List(xs) => Expr::List(v(xs)?),
        })
    }

    /// Lowers a graph selector to an ordinary membership join under the pattern's
    /// own view (`lat.md/data-model#Named Graphs`, design Decision 6). The statement
    /// pattern binds its eid `e`, and the membership pattern `(e sys:inGraph g)`
    /// constrains it:
    ///
    /// - `Set([g])` joins `(e inGraph g)`, so the small side can drive the join;
    /// - `Set([g1, g2, ..])` keeps the statement once through
    ///   `EXISTS { (e inGraph ?x) FILTER ?x IN (g1, g2, ..) }`;
    /// - `Var(g)` joins `(e inGraph ?g)`: one solution per membership.
    fn graph_join(&self, mut t: TriplePattern) -> Result<Op> {
        let sel = std::mem::take(&mut t.graph);
        if sel.is_any() {
            return Ok(Op::Triple(t));
        }
        let n = self.fresh.get();
        self.fresh.set(n + 1);
        let e = t.eid.clone().unwrap_or_else(|| {
            let v = Var::new(format!("{GRAPH_EID_PREFIX}{n}"));
            t.eid = Some(v.clone());
            v
        });
        let member = |o: TermOrVar| {
            Op::Triple(TriplePattern::new(
                TermOrVar::Var(e.clone()),
                TermOrVar::iri(tm_ir::vocab::SYS_IN_GRAPH),
                o,
                t.view,
            ))
        };
        let membership = match sel {
            GraphSel::Any => unreachable!("handled above"),
            GraphSel::Var(g) => member(TermOrVar::Var(g)),
            GraphSel::Set(gs) => {
                let gs = gs
                    .iter()
                    .map(|g| self.term(g))
                    .collect::<Result<Vec<_>>>()?;
                match gs.as_slice() {
                    [one] => member(one.clone()),
                    _ => {
                        let x = Var::new(format!("{GRAPH_VAR_PREFIX}{n}"));
                        let list = gs
                            .into_iter()
                            .map(|g| match g {
                                TermOrVar::Const(v) => Ok(Expr::val(v)),
                                _ => {
                                    Err(invalid("a graph set holds constants and parameters only"))
                                }
                            })
                            .collect::<Result<Vec<_>>>()?;
                        let inner = member(TermOrVar::Var(x.clone())).filter(Expr::In(
                            Box::new(Expr::Var(x)),
                            list,
                            false,
                        ));
                        return Ok(Op::Triple(t).filter(Expr::exists(inner)));
                    }
                }
            }
        };
        Ok(Op::join(vec![Op::Triple(t), membership]))
    }

    fn op(&self, op: &Op) -> Result<Op> {
        let b = |x: &Op| self.op(x).map(Box::new);
        Ok(match op {
            Op::Triple(t) => {
                let mut t = t.clone();
                t.s = self.term(&t.s)?;
                t.p = self.term(&t.p)?;
                t.o = self.term(&t.o)?;
                self.graph_join(t)?
            }
            Op::Path(p) => {
                let mut p = p.clone();
                p.start = self.term(&p.start)?;
                p.end = self.term(&p.end)?;
                // the path engine filters by graph itself: only parameters are bound
                if let GraphSel::Set(gs) = &mut p.graph {
                    for g in gs.iter_mut() {
                        *g = self.term(g)?;
                    }
                }
                Op::Path(p)
            }
            Op::Text(t) => {
                let mut t = t.clone();
                t.query = self.term(&t.query)?;
                if let GraphSel::Set(gs) = &mut t.graph {
                    for g in gs.iter_mut() {
                        *g = self.term(g)?;
                    }
                }
                Op::Text(t)
            }
            Op::Values(v) => {
                let mut v = v.clone();
                for r in &mut v.rows {
                    for c in r.iter_mut().flatten() {
                        *c = self.term(c)?;
                    }
                }
                Op::Values(v)
            }
            Op::Unnest(u) => {
                let mut u = u.clone();
                u.input = b(&u.input)?;
                u.list = self.expr(&u.list)?;
                Op::Unnest(u)
            }
            Op::Join(j) => {
                let mut j = j.clone();
                j.inputs = j.inputs.iter().map(|i| self.op(i)).collect::<Result<_>>()?;
                Op::Join(j)
            }
            Op::LeftJoin(l) => {
                let mut l = l.clone();
                l.left = b(&l.left)?;
                l.right = b(&l.right)?;
                l.cond = l.cond.as_ref().map(|c| self.expr(c)).transpose()?;
                Op::LeftJoin(l)
            }
            Op::Filter(f) => {
                let mut f = f.clone();
                f.input = b(&f.input)?;
                f.cond = self.expr(&f.cond)?;
                Op::Filter(f)
            }
            Op::Union(u) => {
                let mut u = u.clone();
                u.inputs = u.inputs.iter().map(|i| self.op(i)).collect::<Result<_>>()?;
                Op::Union(u)
            }
            Op::Extend(e) => {
                let mut e = e.clone();
                e.input = b(&e.input)?;
                e.expr = self.expr(&e.expr)?;
                Op::Extend(e)
            }
            Op::Aggregate(a) => {
                let mut a = a.clone();
                a.input = b(&a.input)?;
                for g in &mut a.aggs {
                    g.arg = g.arg.as_ref().map(|x| self.expr(x)).transpose()?;
                }
                Op::Aggregate(a)
            }
            Op::Project(p) => {
                let mut p = p.clone();
                p.input = b(&p.input)?;
                Op::Project(p)
            }
            Op::OrderLimit(o) => {
                let mut o = o.clone();
                o.input = b(&o.input)?;
                for k in &mut o.keys {
                    k.expr = self.expr(&k.expr)?;
                }
                o.skip = o.skip.as_ref().map(|t| self.term(t)).transpose()?;
                o.limit = o.limit.as_ref().map(|t| self.term(t)).transpose()?;
                if let Some(s) = &o.skip {
                    check_count("skip", s)?;
                }
                if let Some(l) = &o.limit {
                    check_count("limit", l)?;
                }
                Op::OrderLimit(o)
            }
            Op::RowNumber(r) => {
                let mut r = r.clone();
                r.input = b(&r.input)?;
                for k in &mut r.order {
                    k.expr = self.expr(&k.expr)?;
                }
                Op::RowNumber(r)
            }
        })
    }
}

/// Substitutes every parameter of `q` from `params`.
pub fn bind(q: &IrQuery, params: &Params) -> Result<IrQuery> {
    Ok(IrQuery {
        root: Binder {
            params,
            fresh: std::cell::Cell::new(0),
        }
        .op(&q.root)?,
        semantics: q.semantics,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tm_core::Error;
    use tm_ir::builder::IrBuilder;

    #[test]
    fn substitutes_and_names_missing() {
        let b = IrBuilder::sparql();
        let q = b.query(b.triple("?p", "v:email", "$email"));
        let p = tm_ir::params([("email", Value::str("a@example.org"))]);
        let bound = bind(&q, &p).unwrap();
        match bound.root {
            Op::Triple(t) => assert_eq!(t.o, TermOrVar::Const(Value::str("a@example.org"))),
            _ => panic!(),
        }
        match bind(&q, &Params::new()) {
            Err(Error::InvalidQuery { msg }) => assert!(msg.contains("email"), "{msg}"),
            other => panic!("{other:?}"),
        }
        // a negative bound limit is rejected
        let q = b.query(Op::OrderLimit(tm_ir::OrderLimit {
            input: Box::new(b.triple("?a", "v:p", "?b")),
            keys: vec![],
            skip: None,
            limit: Some(TermOrVar::param("n")),
        }));
        let p = tm_ir::params([("n", Value::Int(-1))]);
        assert!(matches!(bind(&q, &p), Err(Error::InvalidQuery { .. })));
    }
}
