//! Parameter substitution: every `$name` becomes the constant supplied at
//! execution, before any SQL runs. A referenced but missing parameter fails with
//! `InvalidQuery` naming it.

use tm_core::{Result, Value};
use tm_ir::validate::check_count;
use tm_ir::{Expr, IrQuery, Op, Params, TermOrVar};

use crate::error::invalid;

struct Binder<'a> {
    params: &'a Params,
}

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

    fn op(&self, op: &Op) -> Result<Op> {
        let b = |x: &Op| self.op(x).map(Box::new);
        Ok(match op {
            Op::Triple(t) => {
                let mut t = t.clone();
                t.s = self.term(&t.s)?;
                t.p = self.term(&t.p)?;
                t.o = self.term(&t.o)?;
                Op::Triple(t)
            }
            Op::Path(p) => {
                let mut p = p.clone();
                p.start = self.term(&p.start)?;
                p.end = self.term(&p.end)?;
                Op::Path(p)
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
        root: Binder { params }.op(&q.root)?,
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
