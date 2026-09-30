//! Property paths (`path-lowering`). A path with `*`, `+` or `?` goes to the
//! native path operator with `REACH` semantics; a path of only `/`, `|`, `^` and
//! IRIs uses the SPARQL 1.1 translation to joins and unions of triple patterns, so
//! it needs no bound endpoint. Negated property sets are `Unsupported`.

use spargebra::algebra::PropertyPathExpression as P;
use spargebra::term::TermPattern;
use tm_core::{Result, Value};
use tm_ir::{Op, PathExpr, PathMode, PathPattern, TermOrVar, TriplePattern, View};

use super::Lowerer;
use crate::error::{unsupported, NEGATED_PROPERTY_SET, PROPERTY_PATH};

/// True when the path contains `*`, `+` or `?`.
fn recursive(p: &P) -> bool {
    match p {
        P::ZeroOrMore(_) | P::OneOrMore(_) | P::ZeroOrOne(_) => true,
        P::Reverse(x) => recursive(x),
        P::Sequence(a, b) | P::Alternative(a, b) => recursive(a) || recursive(b),
        P::NamedNode(_) | P::NegatedPropertySet(_) => false,
    }
}

/// The IR path expression of `p`.
fn expr(p: &P) -> Result<PathExpr> {
    Ok(match p {
        P::NamedNode(n) => PathExpr::iri(n.as_str()),
        P::Reverse(x) => expr(x)?.inverse(),
        P::Sequence(a, b) => seq_or_alt(expr(a)?, expr(b)?, true),
        P::Alternative(a, b) => seq_or_alt(expr(a)?, expr(b)?, false),
        P::ZeroOrMore(x) => PathExpr::ZeroOrMore(Box::new(expr(x)?)),
        P::OneOrMore(x) => PathExpr::OneOrMore(Box::new(expr(x)?)),
        P::ZeroOrOne(x) => PathExpr::ZeroOrOne(Box::new(expr(x)?)),
        P::NegatedPropertySet(_) => return Err(unsupported(NEGATED_PROPERTY_SET)),
    })
}

/// Flattens nested sequences (alternatives) into one node.
fn seq_or_alt(a: PathExpr, b: PathExpr, seq: bool) -> PathExpr {
    let mut items = Vec::new();
    for x in [a, b] {
        match x {
            PathExpr::Seq(xs) if seq => items.extend(xs),
            PathExpr::Alt(xs) if !seq => items.extend(xs),
            other => items.push(other),
        }
    }
    if seq {
        PathExpr::Seq(items)
    } else {
        PathExpr::Alt(items)
    }
}

impl Lowerer<'_> {
    /// Lowers `subject path object` under `view`.
    // @lat: [[query#Physical Planning#Path Engine#Path Lowering]]
    pub fn path(
        &mut self,
        subject: &TermPattern,
        path: &P,
        object: &TermPattern,
        view: View,
    ) -> Result<Op> {
        let s = self.simple_position(subject)?;
        let o = self.simple_position(object)?;
        if recursive(path) {
            return Ok(Op::Path(PathPattern {
                start: s,
                end: o,
                path: expr(path)?,
                mode: PathMode::Reachability,
                max_hops: None,
                bind_path: None,
                view,
            }));
        }
        self.expand(s, path, o, view)
    }

    /// The SPARQL 1.1 translation of a non-recursive path: a sequence joins on a
    /// fresh variable, an alternative is a union, an inverse swaps its ends.
    fn expand(&mut self, s: TermOrVar, p: &P, o: TermOrVar, view: View) -> Result<Op> {
        Ok(match p {
            P::NamedNode(n) => Op::Triple(TriplePattern::new(
                s,
                TermOrVar::Const(Value::Iri(n.as_str().to_string())),
                o,
                view,
            )),
            P::Reverse(x) => self.expand(o, x, s, view)?,
            P::Sequence(a, b) => {
                let mid = TermOrVar::Var(self.vars.fresh("p"));
                let left = self.expand(s, a, mid.clone(), view)?;
                let right = self.expand(mid, b, o, view)?;
                Op::join(vec![left, right])
            }
            P::Alternative(a, b) => {
                let left = self.expand(s.clone(), a, o.clone(), view)?;
                let right = self.expand(s, b, o, view)?;
                Op::union(vec![left, right])
            }
            P::NegatedPropertySet(_) => return Err(unsupported(NEGATED_PROPERTY_SET)),
            P::ZeroOrMore(_) | P::OneOrMore(_) | P::ZeroOrOne(_) => {
                unreachable!("recursive paths are routed to the path operator")
            }
        })
    }

    fn simple_position(&mut self, t: &TermPattern) -> Result<TermOrVar> {
        Ok(match t {
            TermPattern::NamedNode(n) => TermOrVar::Const(crate::terms::named_node(n)),
            TermPattern::Literal(l) => TermOrVar::Const(crate::terms::literal(l)?),
            TermPattern::Variable(v) => TermOrVar::Var(tm_ir::Var::new(v.as_str())),
            TermPattern::BlankNode(b) => TermOrVar::Var(self.bnode_var(b.as_str())),
            TermPattern::Triple(_) => return Err(unsupported(PROPERTY_PATH)),
        })
    }
}
