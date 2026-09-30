//! The path AST: the IR [`PathExpr`] itself (the planner hands it over without
//! printing and reparsing), plus the analyses the engine needs.

pub use tm_ir::PathExpr as PathAst;
use tm_ir::PathExpr;

/// True when the expression matches the empty word (zero hops): `p*`, `p?`,
/// `p{0,n}`, and sequences or alternations built from such parts.
pub fn nullable(e: &PathExpr) -> bool {
    match e {
        PathExpr::Pred(_) => false,
        PathExpr::Inverse(x) => nullable(x),
        PathExpr::Seq(xs) => xs.iter().all(nullable),
        PathExpr::Alt(xs) => xs.iter().any(nullable),
        PathExpr::ZeroOrMore(_) | PathExpr::ZeroOrOne(_) => true,
        PathExpr::OneOrMore(x) => nullable(x),
        PathExpr::Repeat { inner, min, .. } => *min == 0 || nullable(inner),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p() -> PathExpr {
        PathExpr::iri("urn:p")
    }

    // path-evaluation "Path expression operators": nullable on every operator
    #[test]
    fn nullable_every_operator() {
        assert!(!nullable(&p()));
        assert!(!nullable(&p().inverse()));
        assert!(nullable(&p().star()));
        assert!(!nullable(&p().plus()));
        assert!(nullable(&PathExpr::ZeroOrOne(Box::new(p()))));
        assert!(nullable(&PathExpr::Seq(vec![p().star(), p().star()])));
        assert!(!nullable(&PathExpr::Seq(vec![p().star(), p()])));
        assert!(nullable(&PathExpr::Alt(vec![p(), p().star()])));
        assert!(!nullable(&PathExpr::Alt(vec![p(), p()])));
        assert!(nullable(&p().star().plus()));
        let rep = |min, max| PathExpr::Repeat {
            inner: Box::new(p()),
            min,
            max,
        };
        assert!(nullable(&rep(0, Some(2))));
        assert!(!nullable(&rep(1, None)));
        assert!(nullable(&PathExpr::Repeat {
            inner: Box::new(p().star()),
            min: 3,
            max: None
        }));
    }
}
