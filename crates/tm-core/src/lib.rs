//! Tiramemsu core: the ObjectId codec, the term dictionary, the SQLite storage
//! format, the single-writer transaction engine, temporal views and the event log.
//!
//! `tm-core` reaches SQLite only through its own [`exec::Executor`] trait and
//! depends on no SQLite binding; hosts such as `tm-rusqlite` implement it.
#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod clock;
pub mod codec;
pub mod engine;
pub mod error;
pub mod event;
pub mod exec;
pub mod id;
pub mod read;
pub mod report;
pub mod storage;
pub mod term;
pub mod value;
pub mod view;
pub mod vocab;

pub use clock::{Clock, ManualClock, SystemClock};
pub use engine::{IntoObject, PredicateSchema, Store, StoreOptions, Tx};
pub use error::{Error, Position, Result};
pub use event::{Event, Op};
pub use exec::{Capabilities, Executor, Host, HostOptions, Params, SqlError, SqlValue};
pub use id::{Eid, ObjectId, Tag, TxId};
pub use report::{
    AssertOpts, Asserted, OnExisting, Patch, PatchField, RetKind, Triple, TxOptions, TxReport,
    Valid,
};
pub use term::{TermDict, TermReader};
pub use value::Value;
pub use view::{scan_predicates, TimeRef, TxSel, ValidSel, ViewSpec};
