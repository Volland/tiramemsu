//! Query-level semantic flags that encode the dialect differences.

/// How relationship patterns of one match group may bind statements.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum MatchMode {
    /// No constraint (SPARQL).
    #[default]
    Homomorphism,
    /// Two relationship patterns of one group never bind the same eid (Cypher).
    RelIsomorphism,
}

/// How missing values behave in joins.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum Missing {
    /// SPARQL: an unbound value is compatible with any value.
    #[default]
    Unbound,
    /// Cypher: NULL with three-valued logic; NULL never equals anything.
    Null3VL,
}

/// Whether parallel statements count once or once per eid.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum GraphSet {
    /// SPARQL: each visible distinct `(s, p, o)` matches once.
    #[default]
    SetOfTriples,
    /// Cypher: every visible statement matches.
    BagOfEids,
}

/// The three semantic flags of one query.
///
/// They encode every difference between SPARQL and Cypher that the executor
/// must know about. Start from [`Semantics::sparql`] or [`Semantics::cypher`]
/// and adjust with the `with_*` methods.
///
/// # Example
///
/// ```
/// use tm_ir::{GraphSet, Semantics};
///
/// // Cypher matching rules, but count each distinct triple once.
/// let s = Semantics::cypher().with_graph_set(GraphSet::SetOfTriples);
/// assert_eq!(s.graph_set, GraphSet::SetOfTriples);
/// ```
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Semantics {
    /// Pattern matching mode.
    pub match_mode: MatchMode,
    /// Missing-value semantics.
    pub missing: Missing,
    /// Set or bag of statements.
    pub graph_set: GraphSet,
}

impl Semantics {
    /// SPARQL: (`Homomorphism`, `Unbound`, `SetOfTriples`).
    pub fn sparql() -> Semantics {
        Semantics {
            match_mode: MatchMode::Homomorphism,
            missing: Missing::Unbound,
            graph_set: GraphSet::SetOfTriples,
        }
    }

    /// Cypher: (`RelIsomorphism`, `Null3VL`, `BagOfEids`).
    pub fn cypher() -> Semantics {
        Semantics {
            match_mode: MatchMode::RelIsomorphism,
            missing: Missing::Null3VL,
            graph_set: GraphSet::BagOfEids,
        }
    }

    /// A copy with another `graph_set`.
    pub fn with_graph_set(self, graph_set: GraphSet) -> Semantics {
        Semantics { graph_set, ..self }
    }

    /// A copy with another `match_mode`.
    pub fn with_match_mode(self, match_mode: MatchMode) -> Semantics {
        Semantics { match_mode, ..self }
    }

    /// A copy with another `missing`.
    pub fn with_missing(self, missing: Missing) -> Semantics {
        Semantics { missing, ..self }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // query-ir "Presets"
    #[test]
    fn presets() {
        let s = Semantics::sparql();
        assert_eq!(s.match_mode, MatchMode::Homomorphism);
        assert_eq!(s.missing, Missing::Unbound);
        assert_eq!(s.graph_set, GraphSet::SetOfTriples);
        let c = Semantics::cypher();
        assert_eq!(c.match_mode, MatchMode::RelIsomorphism);
        assert_eq!(c.missing, Missing::Null3VL);
        assert_eq!(c.graph_set, GraphSet::BagOfEids);
        assert_eq!(Semantics::default(), s);
        assert_eq!(
            s.with_graph_set(GraphSet::BagOfEids).graph_set,
            GraphSet::BagOfEids
        );
    }
}
