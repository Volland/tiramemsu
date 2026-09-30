//! [`IrBuilder`]: fluent construction of patterns, joins and filters with a
//! default View, used by tests and front ends.

use tm_core::{Eid, ObjectId, Value};

use crate::expr::Expr;
use crate::op::{IrQuery, Op, PathPattern, TriplePattern};
use crate::path::{PathExpr, PathMode};
use crate::semantics::Semantics;
use crate::term::TermOrVar;
use crate::var::Var;
use crate::view::View;

/// Expands the CURIE prefixes `v:`, `sys:`, `tm:`, `xsd:` and `rdf:`; other text is
/// returned unchanged.
pub fn expand(text: &str) -> String {
    for (p, ns) in crate::display::PATH_PREFIXES {
        if let Some(local) = text.strip_prefix(p).and_then(|r| r.strip_prefix(':')) {
            if !local.starts_with("//") {
                return format!("{ns}{local}");
            }
        }
    }
    text.to_string()
}

/// Anything usable as a pattern position in the builder.
pub trait IntoTerm {
    /// Converts to a pattern position.
    fn into_term(self) -> TermOrVar;
}

impl IntoTerm for &str {
    /// `?x` is a variable, `$x` a parameter, `v:x` / `sys:x` / `tm:x` a CURIE,
    /// anything else an IRI.
    fn into_term(self) -> TermOrVar {
        if self.starts_with('?') {
            TermOrVar::Var(Var::new(self))
        } else if let Some(p) = self.strip_prefix('$') {
            TermOrVar::Param(p.to_string())
        } else {
            TermOrVar::Const(Value::Iri(expand(self)))
        }
    }
}

impl IntoTerm for &String {
    fn into_term(self) -> TermOrVar {
        self.as_str().into_term()
    }
}

impl IntoTerm for TermOrVar {
    fn into_term(self) -> TermOrVar {
        self
    }
}

impl IntoTerm for Value {
    fn into_term(self) -> TermOrVar {
        TermOrVar::Const(self)
    }
}

impl IntoTerm for &Value {
    fn into_term(self) -> TermOrVar {
        TermOrVar::Const(self.clone())
    }
}

impl IntoTerm for ObjectId {
    fn into_term(self) -> TermOrVar {
        TermOrVar::Id(self)
    }
}

impl IntoTerm for Eid {
    fn into_term(self) -> TermOrVar {
        TermOrVar::Id(self.oid())
    }
}

impl IntoTerm for Var {
    fn into_term(self) -> TermOrVar {
        TermOrVar::Var(self)
    }
}

/// Builds IR with a default View for every pattern it creates.
///
/// Pattern positions are plain strings: `?x` is a variable, `$x` a parameter,
/// `v:x` (also `sys:`, `tm:`, `xsd:`, `rdf:`) a CURIE and anything else an IRI.
/// Use [`IrBuilder::at`] to change the View of the patterns that follow.
///
/// # Example
///
/// ```
/// use tm_ir::builder::IrBuilder;
///
/// let b = IrBuilder::sparql();
/// let q = b.query(b.bgp(&[("?p", "v:worksAt", "?c"), ("?c", "v:locatedIn", "$city")]));
/// assert!(q.to_string().contains("locatedIn"));
/// ```
#[derive(Copy, Clone, Debug, Default)]
pub struct IrBuilder {
    /// The View given to new patterns.
    pub view: View,
    /// The flags given to built queries.
    pub semantics: Semantics,
}

impl IrBuilder {
    /// A builder with `view` as the default pattern View and the SPARQL preset.
    pub fn new(view: View) -> IrBuilder {
        IrBuilder {
            view,
            semantics: Semantics::sparql(),
        }
    }

    /// A `{Now, Unfiltered}` builder with the SPARQL preset.
    pub fn sparql() -> IrBuilder {
        IrBuilder::new(View::NOW)
    }

    /// A `{Now, Unfiltered}` builder with the Cypher preset.
    pub fn cypher() -> IrBuilder {
        IrBuilder {
            view: View::NOW,
            semantics: Semantics::cypher(),
        }
    }

    /// A copy with another default View.
    pub fn at(self, view: View) -> IrBuilder {
        IrBuilder { view, ..self }
    }

    /// A copy with other semantics.
    pub fn with(self, semantics: Semantics) -> IrBuilder {
        IrBuilder { semantics, ..self }
    }

    /// A triple pattern under the default View.
    pub fn t(&self, s: impl IntoTerm, p: impl IntoTerm, o: impl IntoTerm) -> TriplePattern {
        TriplePattern::new(s.into_term(), p.into_term(), o.into_term(), self.view)
    }

    /// A triple pattern operator under the default View.
    pub fn triple(&self, s: impl IntoTerm, p: impl IntoTerm, o: impl IntoTerm) -> Op {
        Op::Triple(self.t(s, p, o))
    }

    /// A join of triple patterns given as `(s, p, o)` text.
    pub fn bgp(&self, patterns: &[(&str, &str, &str)]) -> Op {
        Op::join(
            patterns
                .iter()
                .map(|(s, p, o)| self.triple(*s, *p, *o))
                .collect(),
        )
    }

    /// A path pattern under the default View.
    pub fn path(
        &self,
        start: impl IntoTerm,
        path: PathExpr,
        end: impl IntoTerm,
        mode: PathMode,
    ) -> Op {
        Op::Path(PathPattern {
            start: start.into_term(),
            end: end.into_term(),
            path,
            mode,
            max_hops: None,
            bind_path: None,
            view: self.view,
        })
    }

    /// `Filter(cond, input)`.
    pub fn filter(&self, input: Op, cond: Expr) -> Op {
        input.filter(cond)
    }

    /// The query of `root` with this builder's semantics.
    pub fn query(&self, root: Op) -> IrQuery {
        IrQuery::new(root, self.semantics)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_uses_the_default_view() {
        let b = IrBuilder::sparql().at(View::as_of_tx(3));
        let t = b.t("?a", "v:worksAt", "?c");
        assert_eq!(t.view, View::as_of_tx(3));
        assert_eq!(t.p, TermOrVar::Const(Value::iri("urn:tiramemsu:v:worksAt")));
        assert_eq!(
            b.t("?a", "$p", "http://ex/o").p,
            TermOrVar::Param("p".into())
        );
        let q = b.query(b.bgp(&[("?a", "v:p", "?b"), ("?b", "v:q", "?c")]));
        match &q.root {
            Op::Join(j) => assert_eq!(j.inputs.len(), 2),
            _ => panic!(),
        }
        assert_eq!(expand("sys:subject"), crate::vocab::SYS_SUBJECT);
        assert_eq!(expand("http://x"), "http://x");
    }
}
