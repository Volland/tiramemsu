//! Error constructors and the `Unsupported { feature }` names of design D10.

use tm_core::{Dialect, Error, Span};

/// `DESCRIBE` queries.
pub const DESCRIBE: &str = "DESCRIBE";
/// `SERVICE` with a variable or a non-time IRI (federation).
pub const SERVICE: &str = "SERVICE";
/// `GRAPH ?g { SELECT … }`: a subquery hides the graph of its patterns.
pub const GRAPH_SUBQUERY: &str = "GRAPH ?g over a subquery";
/// `GRAPH ?g { … }` whose block has neither a triple pattern nor a property path
/// that the graph selection applies to.
pub const GRAPH_WITHOUT_PATTERN: &str = "GRAPH ?g without a triple pattern";
/// Property paths inside a triple-term position (not a path endpoint).
pub const PROPERTY_PATH: &str = "property path";
/// Negated property sets `!p` and `!(p|^q)`.
pub const NEGATED_PROPERTY_SET: &str = "negated property sets";
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

/// `Unsupported { feature }`: the error every construct outside the v1 subset
/// returns. `feature` is one of the constants of this module or a function name
/// such as `"MD5"`, so callers can match on it.
///
/// # Example
///
/// ```
/// use tm_core::Error;
/// use tm_sparql::error::{unsupported, DESCRIBE};
///
/// assert!(matches!(unsupported(DESCRIBE), Error::Unsupported { feature } if feature == "DESCRIBE"));
/// ```
pub fn unsupported(feature: impl Into<String>) -> Error {
    Error::Unsupported {
        feature: feature.into(),
    }
}

/// `Parse { dialect: Sparql, span, msg }`: invalid text or a static error
/// (undeclared prefix, malformed `tm:` IRI). `span` is a line and column when
/// the parser reported one.
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
