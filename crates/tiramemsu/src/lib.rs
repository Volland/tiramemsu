#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod budget;
mod bundle;
mod cypher;
mod db;
mod import;
mod pool;
mod saved;
mod sparql;
mod view;

pub use budget::QueryBudget;
pub use bundle::{BundleFormat, BUNDLE_FORMAT};
pub use cypher::TxCypher;
pub use db::{Db, OpenOptions};
pub use import::{BulkImport, ImportProgress, ImportSummary};
pub use saved::{
    AnswerStatus, CoverageReason, Invalidation, InvalidationCause, QueryLanguage, SavedAnswer,
    SavedQuery, SAVED_ANSWER_LAYOUT,
};
pub use sparql::{SparqlOptions, SparqlResult};
pub use view::{PathArgs, PathReport, View};

pub use tm_core::text;
pub use tm_core::{
    codec, read, value, vocab, AssertOpts, Asserted, CancelToken, Capabilities, Clock, Dialect,
    Eid, Error, Event, Executor, Host, HostOptions, Interrupt, IntoObject, ManualClock, ObjectId,
    OnExisting, Op, Patch, PatchField, Position, PredicateSchema, Result, ResultLimit, RetKind,
    Span, SqlError, SqlValue, SystemClock, Tag, TextEvidence, TextHit, TextMode, TextQuery,
    TimeRef, Triple, Tx, TxId, TxOptions, TxReport, TxSel, Valid, ValidSel, Value, ViewSpec,
};
pub use tm_core::{BTerm, Bundle, BundleStatement, ImportReport, ImportedStatement};
pub use tm_cypher as cypher_frontend;
pub use tm_cypher::{CypherParams, CypherResult, CypherValue};
pub use tm_exec::path::row::{Dir as PathDir, HopKind};
pub use tm_exec::{
    ExecStats, Explain, Hop, LftjConfig, NativeKind, NativeOperator, Path, PathRow, PlannerOptions,
    QueryResult, RegionInfo, RegionKind, ResultValue, RouteNote, TimeRespecting,
};
pub use tm_ir as ir;
pub use tm_ir::{IrQuery, Params, PathCompleteness, PathMode};
pub use tm_rusqlite::RusqliteHost;
pub use tm_sparql as sparql_frontend;
pub use tm_sparql::results::{ProvenanceGap, RdfTerm, RdfTriple, Solutions};
