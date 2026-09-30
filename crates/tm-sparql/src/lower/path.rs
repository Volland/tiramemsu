//! Property paths (interim, design D5): a single IRI or its inverse is a triple
//! pattern, every other form is `Unsupported("property path")` until
//! `add-path-engine` (M3) replaces this file.

use spargebra::algebra::PropertyPathExpression;
use spargebra::term::TermPattern;
use tm_core::{Result, Value};
use tm_ir::{Op, TermOrVar, TriplePattern, View};

use super::Lowerer;
use crate::error::{unsupported, PROPERTY_PATH};

impl Lowerer<'_> {
    /// Lowers `subject path object` under `view`.
    // M3 HOOK: replace the `Unsupported` arm with a `PathPattern { mode: Reachability,
    // view, .. }` built from `path`.
    pub fn path(
        &mut self,
        subject: &TermPattern,
        path: &PropertyPathExpression,
        object: &TermPattern,
        view: View,
    ) -> Result<Op> {
        let (pred, swap) = match path {
            PropertyPathExpression::NamedNode(p) => (p, false),
            PropertyPathExpression::Reverse(inner) => match &**inner {
                PropertyPathExpression::NamedNode(p) => (p, true),
                _ => return Err(unsupported(PROPERTY_PATH)),
            },
            _ => return Err(unsupported(PROPERTY_PATH)),
        };
        let s = self.simple_position(subject)?;
        let o = self.simple_position(object)?;
        let (s, o) = if swap { (o, s) } else { (s, o) };
        Ok(Op::Triple(TriplePattern::new(
            s,
            TermOrVar::Const(Value::Iri(pred.as_str().to_string())),
            o,
            view,
        )))
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
