//! Static analysis: value domains of SQL columns and GYO cyclicity of BGPs.
//!
//! Maybe-missing sets are tracked per compiled relation by the code generator
//! (`sqlgen::Col::mm`), seeded from the same rules as `tm_ir::validate::scope`.

use std::collections::BTreeSet;

use tm_ir::Var;

use crate::udf;

/// The static class of a computed (native SQLite) value.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum VClass {
    /// INTEGER.
    Int,
    /// REAL.
    Double,
    /// TEXT (a plain string).
    Str,
    /// 0 / 1.
    Bool,
    /// TEXT holding an IRI.
    Iri,
    /// A boxed literal (TEXT `lex SOH datatype-or-@lang`) from `STRDT`, `STRLANG`
    /// and `TIMEZONE`; ordered as a plain string.
    Lit,
    /// A number or a string, decided per value (sums, sample of computed values).
    Dynamic,
}

impl VClass {
    /// The class code passed to `tm_vkind` / `tm_vkey`.
    pub fn code(self) -> i64 {
        match self {
            VClass::Int | VClass::Double => udf::class::NUM,
            VClass::Str | VClass::Lit => udf::class::STR,
            VClass::Bool => udf::class::BOOL,
            VClass::Iri => udf::class::IRI,
            VClass::Dynamic => udf::class::DYNAMIC,
        }
    }

    /// True for numeric classes.
    pub fn numeric(self) -> bool {
        matches!(self, VClass::Int | VClass::Double)
    }
}

/// The static domain of a SQL column or expression.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Dom {
    /// An ObjectId.
    Term,
    /// A native SQLite value of a static class.
    Computed(VClass),
    /// A JSON array whose elements have the inner domain.
    List(Box<Dom>),
    /// An ObjectId, or a JSON array of ObjectIds (a multi-valued lookup).
    TermOrList,
    /// The `path_json` column of `tm_path`.
    PathJson,
}

/// GYO reduction: true when the hypergraph whose hyperedges are the variable sets
/// of the patterns is cyclic (a triangle is cyclic; chains and stars are not).
pub fn is_cyclic(edges: &[BTreeSet<Var>]) -> bool {
    let mut edges: Vec<BTreeSet<Var>> = edges.iter().filter(|e| !e.is_empty()).cloned().collect();
    loop {
        let mut changed = false;
        // remove vertices that occur in only one edge
        let all: Vec<Var> = edges.iter().flatten().cloned().collect();
        for e in edges.iter_mut() {
            let before = e.len();
            e.retain(|v| all.iter().filter(|x| *x == v).count() > 1);
            changed |= e.len() != before;
        }
        // remove edges that are empty or contained in another edge
        let mut i = 0;
        while i < edges.len() {
            let contained = edges[i].is_empty()
                || edges
                    .iter()
                    .enumerate()
                    .any(|(j, f)| j != i && edges[i].is_subset(f) && (edges[i] != *f || j < i));
            if contained {
                edges.remove(i);
                changed = true;
            } else {
                i += 1;
            }
        }
        if edges.is_empty() {
            return false;
        }
        if !changed {
            return true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(vs: &[&str]) -> BTreeSet<Var> {
        vs.iter().map(Var::new).collect()
    }

    #[test]
    fn triangle_is_cyclic() {
        assert!(is_cyclic(&[e(&["a", "b"]), e(&["b", "c"]), e(&["c", "a"])]));
    }

    #[test]
    fn chain_and_star_are_acyclic() {
        assert!(!is_cyclic(&[
            e(&["a", "b"]),
            e(&["b", "c"]),
            e(&["c", "d"])
        ]));
        assert!(!is_cyclic(&[
            e(&["x", "a"]),
            e(&["x", "b"]),
            e(&["x", "c"]),
            e(&["x", "d"])
        ]));
        assert!(!is_cyclic(&[e(&["a"])]));
        assert!(!is_cyclic(&[]));
        // a 4-cycle is cyclic, a triangle with a covering edge is not
        assert!(is_cyclic(&[
            e(&["a", "b"]),
            e(&["b", "c"]),
            e(&["c", "d"]),
            e(&["d", "a"])
        ]));
        assert!(!is_cyclic(&[
            e(&["a", "b"]),
            e(&["b", "c"]),
            e(&["c", "a"]),
            e(&["a", "b", "c"])
        ]));
        // duplicate edges
        assert!(!is_cyclic(&[e(&["a", "b"]), e(&["a", "b"])]));
    }
}
