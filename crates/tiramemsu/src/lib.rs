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

mod db;
mod pool;
mod view;

pub use db::{Db, OpenOptions};
pub use view::View;

pub use tm_core::{
    codec, read, value, vocab, AssertOpts, Asserted, Capabilities, Clock, Eid, Error, Event,
    Executor, Host, HostOptions, IntoObject, ManualClock, ObjectId, OnExisting, Op, Patch,
    PatchField, Position, PredicateSchema, Result, RetKind, SqlError, SqlValue, SystemClock, Tag,
    TimeRef, Triple, Tx, TxId, TxOptions, TxReport, TxSel, Valid, ValidSel, Value, ViewSpec,
};
pub use tm_exec::{
    ExecStats, Explain, LftjConfig, NativeKind, NativeOperator, PlannerOptions, QueryResult,
    RegionInfo, RegionKind, ResultValue, RouteNote,
};
pub use tm_ir as ir;
pub use tm_ir::{IrQuery, Params};
pub use tm_rusqlite::RusqliteHost;
