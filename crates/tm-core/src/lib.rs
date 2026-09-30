#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod clock;
pub mod codec;
pub mod engine;
pub mod error;
pub mod event;
pub mod exec;
pub mod id;
pub mod mapping;
pub mod read;
pub mod report;
pub mod storage;
pub mod term;
pub mod value;
pub mod view;
pub mod vocab;

pub use clock::{Clock, ManualClock, SystemClock};
pub use engine::{IntoObject, PredicateSchema, Store, StoreOptions, Tx};
pub use error::{Dialect, Error, Position, Result, Span};
pub use event::{Event, Op};
pub use exec::{
    AggregateFunction, AggregateState, Capabilities, ConnTableFunction, ConnTableImpl, Executor,
    Host, HostOptions, HostRegistry, Params, ScalarFunction, SqlError, SqlValue, TableFunction,
};
pub use id::{Eid, ObjectId, Tag, TxId};
pub use mapping::Vocab;
pub use report::{
    AssertOpts, Asserted, OnExisting, Patch, PatchField, RetKind, Triple, TxOptions, TxReport,
    Valid,
};
pub use term::{TermDict, TermReader};
pub use value::Value;
pub use view::{scan_predicates, TimeRef, TxSel, ValidSel, ViewSpec};
