//! Basic graph patterns: triple terms, reifiers and annotations become eid-bound
//! triple patterns (design D6). The `rdf:reifies` desugaring assumptions of
//! spargebra live here and nowhere else.

use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern as SpTriple};
use tm_core::Value;
use tm_ir::{Expr, Op, TermOrVar, TriplePattern, Values, Var, View};

use super::Lowerer;
use crate::error::{unsupported, REIFIES_WITHOUT_TRIPLE, VARIABLE_PREDICATE_TRIPLE};
use crate::terms;
use tm_core::Result;

const RDF_REIFIES: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies";

/// The zero-row relation (a reifier constant that is not a statement).
pub fn empty_op() -> Op {
    Op::Values(Values {
        vars: Vec::new(),
        rows: Vec::new(),
    })
}

/// True for the zero-row relation built by [`empty_op`].
pub fn is_empty_op(op: &Op) -> bool {
    matches!(op, Op::Values(v) if v.vars.is_empty() && v.rows.is_empty())
}

/// What a reifier position denotes.
enum Reifier {
    /// Binds this eid variable.
    Var(Var),
    /// A fixed statement: bind a fresh variable and require equality.
    Stmt(Value),
    /// Not a statement: the pattern matches nothing.
    Nothing,
}

#[derive(Default)]
struct Items {
    triples: Vec<TriplePattern>,
    filters: Vec<Expr>,
    empty: bool,
}

impl Lowerer<'_> {
    /// Lowers one basic graph pattern read under `view`.
    pub fn bgp(&mut self, patterns: &[&SpTriple], view: View) -> Result<Op> {
        let mut items = Items::default();
        for p in patterns {
            self.one_pattern(p, view, &mut items)?;
        }
        if items.empty {
            return Ok(empty_op());
        }
        let mut triples = std::mem::take(&mut items.triples);
        eliminate_redundant(&mut triples);
        let ops: Vec<Op> = triples.into_iter().map(Op::Triple).collect();
        let mut op = if ops.len() == 1 {
            ops.into_iter().next().expect("one")
        } else {
            Op::join(ops)
        };
        for f in items.filters {
            op = op.filter(f);
        }
        Ok(op)
    }

    fn one_pattern(&mut self, p: &SpTriple, view: View, items: &mut Items) -> Result<()> {
        let is_reifies =
            matches!(&p.predicate, NamedNodePattern::NamedNode(n) if n.as_str() == RDF_REIFIES);
        if is_reifies {
            let TermPattern::Triple(t) = &p.object else {
                return Err(unsupported(REIFIES_WITHOUT_TRIPLE));
            };
            let target = self.reifier(&p.subject);
            return self.triple_term(t, target, view, items);
        }
        let s = self.position(&p.subject, view, items)?;
        let pred = match &p.predicate {
            NamedNodePattern::NamedNode(n) => TermOrVar::Const(Value::Iri(n.as_str().to_string())),
            NamedNodePattern::Variable(v) => TermOrVar::Var(Var::new(v.as_str())),
        };
        if matches!(p.object, TermPattern::Triple(_)) && matches!(pred, TermOrVar::Var(_)) {
            return Err(unsupported(VARIABLE_PREDICATE_TRIPLE));
        }
        let o = self.position(&p.object, view, items)?;
        items.triples.push(TriplePattern::new(s, pred, o, view));
        Ok(())
    }

    /// A pattern position. A triple term becomes a fresh eid variable and the
    /// pattern that binds it.
    fn position(&mut self, t: &TermPattern, view: View, items: &mut Items) -> Result<TermOrVar> {
        Ok(match t {
            TermPattern::NamedNode(n) => TermOrVar::Const(terms::named_node(n)),
            TermPattern::Literal(l) => TermOrVar::Const(terms::literal(l)?),
            TermPattern::Variable(v) => TermOrVar::Var(Var::new(v.as_str())),
            TermPattern::BlankNode(b) => TermOrVar::Var(self.bnode_var(b.as_str())),
            TermPattern::Triple(tp) => {
                let e = self.vars.fresh("e");
                self.triple_term(tp, Reifier::Var(e.clone()), view, items)?;
                TermOrVar::Var(e)
            }
        })
    }

    fn reifier(&mut self, t: &TermPattern) -> Reifier {
        match t {
            TermPattern::Variable(v) => Reifier::Var(Var::new(v.as_str())),
            TermPattern::BlankNode(b) => Reifier::Var(self.bnode_var(b.as_str())),
            TermPattern::NamedNode(n) => match terms::named_node(n) {
                v @ Value::Stmt(_) => Reifier::Stmt(v),
                _ => Reifier::Nothing,
            },
            _ => Reifier::Nothing,
        }
    }

    /// Emits the pattern of the triple term `tp` bound to `target`.
    fn triple_term(
        &mut self,
        tp: &SpTriple,
        target: Reifier,
        view: View,
        items: &mut Items,
    ) -> Result<()> {
        let eid = match target {
            Reifier::Var(v) => v,
            Reifier::Stmt(value) => {
                let e = self.vars.fresh("e");
                items.filters.push(Expr::SameTerm(
                    Box::new(Expr::Var(e.clone())),
                    Box::new(Expr::Const(value)),
                ));
                e
            }
            Reifier::Nothing => {
                items.empty = true;
                return Ok(());
            }
        };
        let s = self.position(&tp.subject, view, items)?;
        let pred = match &tp.predicate {
            NamedNodePattern::NamedNode(n) => TermOrVar::Const(Value::Iri(n.as_str().to_string())),
            NamedNodePattern::Variable(v) => TermOrVar::Var(Var::new(v.as_str())),
        };
        let o = self.position(&tp.object, view, items)?;
        let mut t = TriplePattern::new(s, pred, o, view);
        t.eid = Some(eid);
        items.triples.push(t);
        Ok(())
    }
}

/// Drops a plain `(s, p, o)` pattern when the same list has an eid-bound pattern
/// with the same terms and view: the eid pattern implies the triple.
fn eliminate_redundant(triples: &mut Vec<TriplePattern>) {
    let bound: Vec<TriplePattern> = triples
        .iter()
        .filter(|t| t.eid.is_some())
        .cloned()
        .collect();
    triples.retain(|t| {
        t.eid.is_some()
            || !bound
                .iter()
                .any(|b| b.s == t.s && b.p == t.p && b.o == t.o && b.view == t.view)
    });
}
