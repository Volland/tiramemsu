//! The error type shared by every Tiramemsu crate.

use std::path::PathBuf;
use std::time::Duration;

use crate::budget::ResultLimit;
use crate::exec::SqlError;
use crate::id::{Eid, ObjectId, Tag};

/// Result alias using [`Error`].
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// The position of a value in a write or lookup.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Position {
    /// Subject position.
    Subject,
    /// Predicate position.
    Predicate,
    /// Object position.
    Object,
    /// Volatile key.
    Key,
    /// A value outside a statement position (decoding, volatile value).
    Value,
}

/// The query language a [`Error::Parse`] failure belongs to.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Dialect {
    /// SPARQL 1.1 / 1.2 query or update text.
    Sparql,
    /// Cypher text.
    Cypher,
    /// A `tm_path` path expression.
    Path,
}

/// Where parsing failed: 1-based line and column, and the byte offset.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Span {
    /// 1-based line.
    pub line: usize,
    /// 1-based column.
    pub column: usize,
    /// Byte offset into the text.
    pub offset: usize,
}

/// Every failure of the store. A failed transaction leaves no trace.
///
/// The enum is `#[non_exhaustive]`, so match with a wildcard arm. Host failures
/// arrive as [`Error::Sqlite`] with a [`SqlError`] carrying the SQLite result code
/// (see [`Error::sql`]); your own failures inside a transaction body can be wrapped
/// with [`Error::custom`]. Some variants (`Parse`, `PathLimitExceeded`,
/// `MissingCapability`, ...) are produced only by the query crates above `tm-core`.
///
/// ```
/// use tm_core::Error;
///
/// let e = Error::custom("stop");
/// match e {
///     Error::Custom(_) => {}
///     _ => unreachable!(),
/// }
/// assert_eq!(Error::unsupported("SEALED").to_string(), "unsupported: SEALED");
/// ```
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A second live subject for a `sys:unique` predicate.
    #[error("unique violation on {p}: value {o} is already held by {existing}")]
    UniqueViolation {
        /// The unique predicate.
        p: ObjectId,
        /// The object value.
        o: ObjectId,
        /// The subject that already holds it.
        existing: ObjectId,
    },
    /// The object violates `sys:valueType` (or a flag's built-in value type).
    #[error("value type mismatch on {p}: expected {expected}, got {}", got.name())]
    ValueTypeMismatch {
        /// The predicate whose value type was violated.
        p: ObjectId,
        /// The expected type IRI.
        expected: ObjectId,
        /// The kind of the rejected object.
        got: Tag,
    },
    /// The subject's kind is not one of the predicate's `sys:subjectType` values.
    #[error("subject type mismatch on {p}: expected one of {expected:?}, got {}", got.name())]
    SubjectTypeMismatch {
        /// The predicate whose subject type was violated.
        p: ObjectId,
        /// The allowed tag IRIs, in eid order of their flag statements.
        expected: Vec<ObjectId>,
        /// The kind of the rejected subject.
        got: Tag,
    },
    /// A cascade set is larger than `max_cascade`.
    #[error("cascade from {root} exceeds the limit of {limit} statements")]
    CascadeLimitExceeded {
        /// The root retraction.
        root: Eid,
        /// The configured limit.
        limit: usize,
    },
    /// A path evaluation needed more search states than `path_max_states`.
    #[error("path search exceeds the limit of {limit} search states")]
    PathLimitExceeded {
        /// The configured limit.
        limit: usize,
    },
    /// The operation was cancelled through its [`CancelToken`](crate::budget::CancelToken).
    /// Read resources are released and a write is rolled back.
    #[error("operation cancelled")]
    Cancelled,
    /// The operation ran past the deadline of its budget. Read resources are
    /// released and a write is rolled back.
    #[error("operation exceeded its timeout of {} ms", timeout.as_millis())]
    DeadlineExceeded {
        /// The configured timeout.
        timeout: Duration,
    },
    /// No read connection became free within the reader acquisition timeout.
    #[error("no reader became available within {} ms", timeout.as_millis())]
    PoolTimeout {
        /// The configured reader timeout.
        timeout: Duration,
    },
    /// An operation produced more rows or decoded bytes than its budget allows. No
    /// partial result is returned.
    #[error("result exceeds the limit of {limit}")]
    ResultLimitExceeded {
        /// The budget that was exceeded.
        limit: ResultLimit,
    },
    /// `supersede` or `confirm` on a retracted or unknown eid.
    #[error("statement {0} is not live")]
    NotLive(Eid),
    /// A supersede patch is invalid (names `s`/`p`, empty interval, or no change).
    #[error("invalid patch: {0}")]
    InvalidPatch(String),
    /// A statement would use its own eid as subject or object.
    #[error("statement {0} would reference itself")]
    SelfReference(Eid),
    /// A user write uses a reserved `sys:` or `tm:` predicate.
    #[error("reserved namespace: {0}")]
    ReservedNamespace(String),
    /// A graph name that is not an IRI, a `NODE` or a `BNODE`.
    #[error("invalid graph name: {term}")]
    InvalidGraphName {
        /// The rejected term, rendered.
        term: String,
    },
    /// `CLEAR GRAPH` or `DROP GRAPH` on a graph with no membership and no declaration.
    #[error("graph not found: {graph}")]
    GraphNotFound {
        /// The graph, rendered.
        graph: String,
    },
    /// `CREATE GRAPH` on a graph that is already declared.
    #[error("graph already exists: {graph}")]
    GraphExists {
        /// The graph, rendered.
        graph: String,
    },
    /// A schema change is violated by live data.
    #[error("schema change conflicts with live statements {violating:?}")]
    SchemaConflict {
        /// The violating eids, ascending.
        violating: Vec<Eid>,
    },
    /// The file was written by a newer format.
    #[error("format version {found} is newer than the supported version {supported}")]
    FormatVersion {
        /// Version found in the file.
        found: i64,
        /// Highest version this build supports.
        supported: i64,
    },
    /// Query text that is not valid in its dialect (`lat.md/api#Errors`).
    #[error("{dialect:?} parse error{}: {msg}", span.map(|s| format!(" at {}:{}", s.line, s.column)).unwrap_or_default())]
    Parse {
        /// The language of the text.
        dialect: Dialect,
        /// Where it failed, when known.
        span: Option<Span>,
        /// What was wrong.
        msg: String,
    },
    /// A feature reserved for a later milestone.
    #[error("unsupported: {feature}")]
    Unsupported {
        /// The feature name, including its milestone.
        feature: String,
    },
    /// A runtime expression error: a type error, a division by zero, an unstorable value.
    #[error("{dialect:?} evaluation error: {msg}")]
    Eval {
        /// The query language.
        dialect: Dialect,
        /// What went wrong.
        msg: String,
    },
    /// `DELETE` of a node that still has live relationships (Cypher).
    #[error("cannot delete node {node}: it still has {} relationship(s)", relationships.len())]
    DeleteConnectedNode {
        /// The node.
        node: ObjectId,
        /// The live relationship statements that mention it.
        relationships: Vec<Eid>,
    },
    /// A structurally invalid query IR or plan, or a missing parameter.
    #[error("invalid query: {msg}")]
    InvalidQuery {
        /// What is wrong.
        msg: String,
    },
    /// The host lacks a capability the query engine requires (`functions`, `vtab`).
    #[error("the host lacks the `{capability}` capability required by the query engine")]
    MissingCapability {
        /// The missing capability name.
        capability: String,
    },
    /// A value of the wrong kind in a position, or an unknown term id.
    #[error("invalid term in {position:?} position: {reason}")]
    InvalidTerm {
        /// Where the value was used.
        position: Position,
        /// Why it was rejected.
        reason: String,
    },
    /// `assert`/`create` with an empty valid interval (`v_from >= v_to`).
    #[error("invalid valid-time interval [{v_from}, {v_to})")]
    InvalidInterval {
        /// Requested start.
        v_from: i64,
        /// Requested end.
        v_to: i64,
    },
    /// `upsert` on a predicate without `sys:unique true`.
    #[error("predicate {0} is not unique")]
    NotUniquePredicate(ObjectId),
    /// An id counter is used up: the next `NODE`, `BNODE`, `STMT` or `TX` number
    /// would pass 2⁴⁸ − 1 and spill into the origin bits. The transaction fails.
    #[error("id space exhausted for {}: counters stop at 2^48 - 1", kind.name())]
    IdSpaceExhausted {
        /// The kind whose counter is used up.
        kind: Tag,
    },
    /// A write was started from inside a running write on the same database.
    #[error("re-entrant write on the same database")]
    Reentrant,
    /// A write, or a second import session, was started while a bulk import
    /// session holds the database's write lease. Reads are unaffected; writes
    /// resume once the session finishes, is cancelled or is dropped.
    #[error("a bulk import session holds the write lease")]
    ImportInProgress,
    /// A SQLite file with user tables but no tiramemsu `meta` table.
    #[error("{0} is not a tiramemsu database")]
    ForeignFile(PathBuf),
    /// An error reported by the SQLite host.
    #[error("sqlite: {0}")]
    Sqlite(SqlError),
    /// The caller aborted the transaction body.
    #[error("{0}")]
    Custom(Box<dyn std::error::Error + Send + Sync>),
}

impl Error {
    /// Wraps a caller error as [`Error::Custom`].
    pub fn custom(e: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> Error {
        Error::Custom(e.into())
    }

    /// An [`Error::Parse`] of `dialect`.
    pub fn parse(dialect: Dialect, span: Option<Span>, msg: impl Into<String>) -> Error {
        Error::Parse {
            dialect,
            span,
            msg: msg.into(),
        }
    }

    /// An [`Error::InvalidQuery`] with `msg`.
    pub fn invalid_query(msg: impl Into<String>) -> Error {
        Error::InvalidQuery { msg: msg.into() }
    }

    /// An [`Error::Unsupported`] naming `feature`.
    pub fn unsupported(feature: impl Into<String>) -> Error {
        Error::Unsupported {
            feature: feature.into(),
        }
    }

    /// The host error, if this is one.
    pub fn sql(&self) -> Option<&SqlError> {
        match self {
            Error::Sqlite(e) => Some(e),
            _ => None,
        }
    }
}

impl From<SqlError> for Error {
    fn from(e: SqlError) -> Error {
        Error::Sqlite(e)
    }
}
