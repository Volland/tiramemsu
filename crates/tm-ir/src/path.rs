//! Path expressions and modes (the interface; the algorithm is `add-path-engine`).

use tm_core::Value;

/// A property-path expression.
#[derive(Clone, Debug, PartialEq)]
pub enum PathExpr {
    /// One predicate hop (an IRI; `sys:anyRelationship` is the Cypher wildcard).
    Pred(Value),
    /// `^p`: the hop backwards.
    Inverse(Box<PathExpr>),
    /// `a / b / …`.
    Seq(Vec<PathExpr>),
    /// `a | b | …`.
    Alt(Vec<PathExpr>),
    /// `p*`.
    ZeroOrMore(Box<PathExpr>),
    /// `p+`.
    OneOrMore(Box<PathExpr>),
    /// `p?`.
    ZeroOrOne(Box<PathExpr>),
    /// `p{min,max}`; `max = None` is unbounded.
    Repeat {
        /// The repeated path.
        inner: Box<PathExpr>,
        /// Minimum count.
        min: u32,
        /// Maximum count, `None` for unbounded.
        max: Option<u32>,
    },
}

impl PathExpr {
    /// A predicate hop on an IRI.
    pub fn iri(iri: impl Into<String>) -> PathExpr {
        PathExpr::Pred(Value::Iri(iri.into()))
    }

    /// `^self`.
    pub fn inverse(self) -> PathExpr {
        match self {
            PathExpr::Inverse(p) => *p,
            p => PathExpr::Inverse(Box::new(p)),
        }
    }

    /// `self*`.
    pub fn star(self) -> PathExpr {
        PathExpr::ZeroOrMore(Box::new(self))
    }

    /// `self+`.
    pub fn plus(self) -> PathExpr {
        PathExpr::OneOrMore(Box::new(self))
    }
}

/// How the path operator enumerates paths.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum PathMode {
    /// Endpoints only, set semantics (SPARQL).
    #[default]
    Reachability,
    /// No repeated relationship eid (Cypher).
    Trail,
    /// One shortest path per endpoint pair.
    AnyShortest,
    /// Every shortest path per endpoint pair.
    AllShortest,
}

impl PathMode {
    /// The `mode` argument text of `tm_path`.
    pub fn sql_name(self) -> &'static str {
        match self {
            PathMode::Reachability => "REACH",
            PathMode::Trail => "TRAIL",
            PathMode::AnyShortest => "ANY_SHORTEST",
            PathMode::AllShortest => "ALL_SHORTEST",
        }
    }
}
