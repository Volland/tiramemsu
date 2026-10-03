// The README is the crate documentation, and its tour runs as doctests against
// the full facade; a reduced build documents its features instead.
#![cfg_attr(all(feature = "sparql", feature = "cypher"), doc = include_str!("../README.md"))]
#![cfg_attr(
    not(all(feature = "sparql", feature = "cypher")),
    doc = "Tiramemsu: a bitemporal, never-forget triple store on SQLite, built with a \
           reduced feature set.\n\n\
           The cargo features of this crate are `exec` (the shared query engine: IR, \
           planner, paths, LFTJ), `sparql` and `cypher` (the query front ends, each \
           implying `exec`); the default enables both front ends. Without them the \
           core tier remains: [`Db`], [`View`] reads, transactions, fact bundles with \
           [`BundleFormat`], text recall, conflict review, bulk import and budgets. \
           See the README's *Cargo features* section for the full table."
)]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod budget;
mod bundle;
#[cfg(feature = "cypher")]
mod cypher;
mod db;
mod import;
mod pool;
mod review;
#[cfg(all(feature = "sparql", feature = "cypher"))]
mod saved;
#[cfg(feature = "sparql")]
mod sparql;
mod view;

pub use budget::QueryBudget;
pub use bundle::{BundleFormat, BUNDLE_FORMAT};
#[cfg(feature = "cypher")]
pub use cypher::TxCypher;
pub use db::{Db, OpenOptions};
pub use import::{BulkImport, ImportProgress, ImportSummary};
pub use review::{BundlePreview, PreviewScope};
#[cfg(all(feature = "sparql", feature = "cypher"))]
pub use saved::{
    AnswerStatus, CoverageReason, Invalidation, InvalidationCause, QueryLanguage, SavedAnswer,
    SavedQuery, SAVED_ANSWER_LAYOUT,
};
#[cfg(feature = "sparql")]
pub use sparql::{SparqlOptions, SparqlResult};
pub use view::View;
#[cfg(feature = "exec")]
pub use view::{PathArgs, PathReport};

pub use tm_core::rdf::{RdfTerm, RdfTriple};
pub use tm_core::text;
pub use tm_core::{
    codec, conflict, read, value, vocab, AssertOpts, Asserted, CancelToken, Capabilities, Clock,
    Dialect, Eid, Error, Event, Executor, Host, HostOptions, Interrupt, IntoObject, ManualClock,
    ObjectId, OnExisting, Op, Patch, PatchField, Position, PredicateSchema, Result, ResultLimit,
    RetKind, Span, SqlError, SqlValue, SystemClock, Tag, TextEvidence, TextHit, TextMode,
    TextQuery, TimeRef, Triple, Tx, TxId, TxOptions, TxReport, TxSel, Valid, ValidSel, Value,
    ViewSpec,
};
pub use tm_core::{BTerm, Bundle, BundleStatement, IdUsage, ImportReport, ImportedStatement};
pub use tm_core::{Conflict, ConflictEvidence, ConflictQuery, ConflictValue};
pub use tm_rusqlite::RusqliteHost;

#[cfg(feature = "cypher")]
pub use tm_cypher as cypher_frontend;
#[cfg(feature = "cypher")]
pub use tm_cypher::{CypherParams, CypherResult, CypherValue};
#[cfg(feature = "exec")]
pub use tm_exec::path::row::{Dir as PathDir, HopKind};
#[cfg(feature = "exec")]
pub use tm_exec::{
    ExecStats, Explain, Hop, LftjConfig, LftjOperator, NativeKind, NativeOperator, Path, PathRow,
    PlannerOptions, QueryResult, RegionInfo, RegionKind, ResultValue, RouteNote, TimeRespecting,
};
#[cfg(feature = "exec")]
pub use tm_ir as ir;
#[cfg(feature = "exec")]
pub use tm_ir::{IrQuery, Params, PathCompleteness, PathMode};
#[cfg(feature = "sparql")]
pub use tm_sparql as sparql_frontend;
#[cfg(feature = "sparql")]
pub use tm_sparql::results::{ProvenanceGap, Solutions};
