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
    /// `set_embedding` with a vector that differs from the one already stored for
    /// the same model and statement. Embeddings are append-only: re-embed under a
    /// new model instead.
    #[error("statement {eid} already has a different embedding for model {model}")]
    EmbeddingConflict {
        /// The statement.
        eid: Eid,
        /// The model IRI.
        model: String,
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
    /// Text recall on a database whose derived text index cannot answer: it was
    /// never built (`OpenOptions::text_index` or `Db::rebuild_text_index`), or a
    /// host without FTS5 wrote string statements since it was last brought up to
    /// date. Graph history is unaffected; rebuilding the index fixes it.
    #[error("text index unavailable: {reason}")]
    TextIndexUnavailable {
        /// Why the index cannot answer.
        reason: String,
    },
    /// A saved-answer operation named an answer that was never saved, or was
    /// deleted.
    #[error("saved answer not found: {name}")]
    SavedAnswerNotFound {
        /// The name that was looked up.
        name: String,
    },
    /// N3 rule text that does not parse (facade feature `n3`).
    #[error("N3 parse error at line {line}: {message}")]
    N3Parse {
        /// The 1-based line of the error (0 when the parser gave no position).
        line: usize,
        /// What was wrong.
        message: String,
    },
    /// `Rules::Stored` names a rule set without a live `n3:source` statement in
    /// the reasoning view (facade feature `n3`).
    #[error("unknown rule set: {rule_set}")]
    UnknownRuleSet {
        /// The rule set, rendered.
        rule_set: String,
    },
    /// Materialization of a rule set that uses a non-monotonic builtin, a
    /// clock-reading builtin or `list:map` (untraced premises) (facade feature `n3`).
    #[error("rule set cannot be materialized: rule {rule} uses {builtin}")]
    N3NotMaterializable {
        /// The index of the first offending rule in its rule set.
        rule: usize,
        /// The builtin it uses, as a prefixed name (`log:notIncludes`,
        /// `time:localTime`, `list:map`).
        builtin: String,
    },
    /// An inference fuse (`{ … } => false`) fired during materialization;
    /// nothing was written (facade feature `n3`).
    #[error("inference fuse fired: rule {rule}")]
    N3Contradiction {
        /// The index of the fuse rule in its rule set.
        rule: usize,
    },
    /// Materialization with `require_complete` of a run that was incomplete (a
    /// limit was reached or a builtin failed); nothing was written (facade
    /// feature `n3`).
    #[error("materialization incomplete: limits {limits:?}, builtin errors {errors:?}")]
    N3Incomplete {
        /// The limits reached (`Segments`, `Iterations`, ...), ascending.
        limits: Vec<String>,
        /// The builtin failures the reasoner reported.
        errors: Vec<String>,
    },
    /// Datalog program text that does not parse (facade feature `datalog`).
    #[error("Datalog parse error at line {line}, column {column}: {message}")]
    DatalogParse {
        /// The 1-based line.
        line: usize,
        /// The 1-based column.
        column: usize,
        /// What was wrong.
        message: String,
    },
    /// A Datalog rule with a variable that no positive atom binds.
    #[error("unsafe Datalog rule {rule}: {message}")]
    DatalogUnsafe {
        /// The index of the rule (goals and constraints follow the rules).
        rule: usize,
        /// The variable and where it occurs.
        message: String,
    },
    /// A Datalog program with recursion through negation or aggregation.
    #[error("Datalog program cannot be stratified: {message}")]
    DatalogUnstratifiable {
        /// The relations of one cycle.
        message: String,
    },
    /// A reserved Datalog atom (`triple`, `valid`, ...) used as a rule head.
    #[error("reserved Datalog name in a head: {name}")]
    DatalogReserved {
        /// The reserved name.
        name: String,
    },
    /// A sometime operator (`past`, `future`) in a rule head.
    #[error("not a Horn rule: {message}")]
    DatalogNotHorn {
        /// The offending head.
        message: String,
    },
    /// A program that recurses through a metric temporal operator, run without
    /// a window.
    #[error("Datalog recursion through a temporal operator needs a window: {relation}")]
    DatalogUnboundedRecursion {
        /// A relation on the cycle.
        relation: String,
    },
    /// A program that recurses through value invention, run without
    /// `limits.max_new_values`.
    #[error("Datalog recursion through value invention needs max_new_values: {relation}")]
    DatalogUnboundedInvention {
        /// A relation on the cycle.
        relation: String,
    },
    /// A temporal operator in a program run with `TimeMode::Atemporal`.
    #[error("temporal operator in an atemporal Datalog run: {operator}")]
    DatalogTemporalOperatorInAtemporal {
        /// The operator.
        operator: String,
    },
    /// Two constraints of one program with the same name.
    #[error("duplicate Datalog constraint: {name}")]
    DatalogDuplicateConstraint {
        /// The name.
        name: String,
    },
    /// A Datalog run with `require_complete` that reached a limit.
    #[error("Datalog run incomplete: limits {limits:?}")]
    DatalogIncomplete {
        /// The limits reached (`Iterations`, `Facts`, ...), ascending.
        limits: Vec<String>,
    },
    /// `Program::Stored` names a program without a live `dl:source` statement in
    /// the view (facade feature `datalog`).
    #[error("unknown Datalog program: {program}")]
    UnknownProgram {
        /// The program, rendered.
        program: String,
    },
    /// Storing rules of one language on a node that holds live rules of the other.
    #[error("{node} already holds {existing} rules")]
    RuleLanguageConflict {
        /// The node, rendered.
        node: String,
        /// The language it already holds (`"N3"` or `"Datalog"`).
        existing: String,
    },
    /// A file that is neither a SQLite nor a redb database.
    #[error("{path} is not a database file this build can open (neither SQLite nor redb)")]
    UnknownFormat {
        /// The file.
        path: PathBuf,
    },
    /// An explicit backend choice that disagrees with the existing file.
    #[error("{path} is a {found} database, but the {expected} backend was requested")]
    BackendMismatch {
        /// The file.
        path: PathBuf,
        /// The requested backend.
        expected: String,
        /// The backend of the file.
        found: String,
    },
    /// `create` on a path that exists.
    #[error("{0} already exists")]
    AlreadyExists(PathBuf),
    /// A backend this build was compiled without.
    #[error("this build has no {backend} backend (enable the `{backend}` cargo feature)")]
    BackendNotCompiled {
        /// The backend.
        backend: String,
    },
    /// A SQLite file with user tables but no tiramemsu `meta` table.
    #[error("{0} is not a tiramemsu database")]
    ForeignFile(PathBuf),
    /// An error reported by the SQLite host.
    #[error("sqlite: {0}")]
    Sqlite(SqlError),
    /// An error reported by a storage backend on the storage trait (redb, the
    /// in-memory reference store): I/O, a damaged file, or a refused write such
    /// as a duplicate key.
    #[error("storage: {message}")]
    Storage {
        /// What failed.
        message: String,
    },
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

    /// An [`Error::Storage`] with `message`.
    pub fn storage(message: impl Into<String>) -> Error {
        Error::Storage {
            message: message.into(),
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
