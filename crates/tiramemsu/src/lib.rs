#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod cypher;
mod db;
mod pool;
mod sparql;
mod view;

pub use cypher::TxCypher;
pub use db::{Db, OpenOptions};
pub use sparql::SparqlResult;
pub use view::View;

pub use tm_core::{
    codec, read, value, vocab, AssertOpts, Asserted, Capabilities, Clock, Dialect, Eid, Error,
    Event, Executor, Host, HostOptions, IntoObject, ManualClock, ObjectId, OnExisting, Op, Patch,
    PatchField, Position, PredicateSchema, Result, RetKind, Span, SqlError, SqlValue, SystemClock,
    Tag, TimeRef, Triple, Tx, TxId, TxOptions, TxReport, TxSel, Valid, ValidSel, Value, ViewSpec,
};
pub use tm_cypher as cypher_frontend;
pub use tm_cypher::{CypherParams, CypherResult, CypherValue};
pub use tm_exec::path::row::{Dir as PathDir, HopKind};
pub use tm_exec::{
    ExecStats, Explain, Hop, LftjConfig, NativeKind, NativeOperator, Path, PathRow, PlannerOptions,
    QueryResult, RegionInfo, RegionKind, ResultValue, RouteNote,
};
pub use tm_ir as ir;
pub use tm_ir::{IrQuery, Params, PathMode};
pub use tm_rusqlite::RusqliteHost;
pub use tm_sparql as sparql_frontend;
pub use tm_sparql::results::{RdfTerm, RdfTriple, Solutions};
