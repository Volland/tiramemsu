//! Error constructors and the `Unsupported { feature }` names of design D10.

use tm_core::{Dialect, Error, Span};

/// `DESCRIBE` queries.
pub const DESCRIBE: &str = "DESCRIBE";
/// `SERVICE` with a variable or a non-time IRI (federation).
pub const SERVICE: &str = "SERVICE";
/// `GRAPH ?g`.
pub const GRAPH_VARIABLE: &str = "GRAPH variable";
/// A named graph: `FROM`, `GRAPH`, `WITH`, `USING` with a non-time IRI.
pub const NAMED_GRAPH: &str = "named graph";
/// Property paths beyond a single IRI or its inverse (interim, M3).
pub const PROPERTY_PATH: &str = "property path";
/// Custom aggregate functions.
pub const CUSTOM_AGGREGATE: &str = "custom aggregate";
/// `DISTINCT` with a sort key that is not projected.
pub const ORDER_BY_DISTINCT: &str = "ORDER BY non-projected variable with DISTINCT";
/// Two `FROM` clauses selecting the same time part differently.
pub const CONFLICTING_TIME: &str = "conflicting time selectors";
/// `?r rdf:reifies X` where `X` is not a triple term.
pub const REIFIES_WITHOUT_TRIPLE: &str = "rdf:reifies without triple term";
/// A variable predicate with a triple-term object.
pub const VARIABLE_PREDICATE_TRIPLE: &str = "variable predicate with triple term";
/// An update reifier that is not a statement.
pub const REIFIER_NOT_STATEMENT: &str = "reifier that is not a statement";
/// One update reifier for two triples.
pub const REIFIER_MANY: &str = "reifier of more than one triple";
/// An update on a view other than the plain current one.
pub const UPDATE_NON_CURRENT: &str = "update on a non-current view";
/// `LOAD`.
pub const LOAD: &str = "LOAD";
/// `CLEAR`.
pub const CLEAR: &str = "CLEAR";
/// `CREATE`.
pub const CREATE: &str = "CREATE";
/// `DROP`.
pub const DROP: &str = "DROP";
/// `ADD`.
pub const ADD: &str = "ADD";
/// `MOVE`.
pub const MOVE: &str = "MOVE";
/// `COPY`.
pub const COPY: &str = "COPY";

/// `Unsupported { feature }`.
pub fn unsupported(feature: impl Into<String>) -> Error {
    Error::Unsupported {
        feature: feature.into(),
    }
}

/// `Parse { dialect: Sparql, span, msg }`.
pub fn parse_error(span: Option<Span>, msg: impl Into<String>) -> Error {
    Error::Parse {
        dialect: Dialect::Sparql,
        span,
        msg: msg.into(),
    }
}

/// The error for a malformed `tm:` time IRI.
pub fn invalid_time_iri(iri: &str) -> Error {
    parse_error(
        None,
        format!("invalid time IRI <{iri}>: not a valid time IRI"),
    )
}

/// The error for a `tm:` IRI used as a `GRAPH` name.
pub fn time_iri_in_graph(iri: &str) -> Error {
    parse_error(
        None,
        format!("time IRI <{iri}> is not allowed in GRAPH; use SERVICE <{iri}> {{ … }}"),
    )
}
