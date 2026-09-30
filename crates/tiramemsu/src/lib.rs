//! Tiramemsu: a bitemporal, never-forget triple store on one SQLite file.
//!
//! The facade exposes three handles: [`Db`] opens the file and runs transactions,
//! [`View`] is an immutable time selection that reads, and [`Tx`] holds the write
//! operations. Every statement has its own identity (eid), transaction time is
//! Datomic-style (`t`, instants, as-of, history, speculation), and valid time is an
//! immutable half-open interval. Nothing is ever deleted.
//!
//! ```no_run
//! use tiramemsu::{Db, OpenOptions, TxOptions, Valid, Value};
//! let db = Db::open("memory.db", OpenOptions::default())?;
//! let v = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
//! let report = db.transact(TxOptions::default(), |tx| {
//!     tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?;
//!     Ok(())
//! })?;
//! let rows = db.now().triples(None, None, None)?;
//! # Ok::<(), tiramemsu::Error>(())
//! ```
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
