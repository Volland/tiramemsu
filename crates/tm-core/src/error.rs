//! The error type shared by every Tiramemsu crate.

use std::path::PathBuf;

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

/// Every failure of the store. A failed transaction leaves no trace.
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
    /// A cascade set is larger than `max_cascade`.
    #[error("cascade from {root} exceeds the limit of {limit} statements")]
    CascadeLimitExceeded {
        /// The root retraction.
        root: Eid,
        /// The configured limit.
        limit: usize,
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
    /// A feature reserved for a later milestone.
    #[error("unsupported: {feature}")]
    Unsupported {
        /// The feature name, including its milestone.
        feature: String,
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
    /// A write was started from inside a running write on the same database.
    #[error("re-entrant write on the same database")]
    Reentrant,
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
