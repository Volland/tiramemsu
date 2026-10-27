#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod budget;
pub mod bundle;
pub mod clock;
pub mod codec;
pub mod conflict;
pub mod engine;
pub mod error;
pub mod event;
pub mod exec;
pub mod handle;
pub mod id;
pub mod mapping;
pub mod rdf;
pub mod read;
pub mod recall;
pub mod report;
pub mod sqlstore;
pub mod storage;
pub mod store;
pub mod term;
pub mod text;
pub mod value;
pub mod vector;
pub mod view;
pub mod vocab;

pub use budget::{CancelToken, Interrupt, ResultLimit};
pub use bundle::{BTerm, Bundle, BundleSource, BundleStatement, ImportReport, ImportedStatement};
pub use clock::{Clock, ManualClock, SystemClock};
pub use conflict::{Conflict, ConflictEvidence, ConflictQuery, ConflictValue};
pub use engine::{IntoObject, PredicateSchema, Store, StoreOptions, Tx};
pub use error::{Dialect, Error, Position, Result, Span};
pub use event::{Event, Op};
pub use exec::{
    AggregateFunction, AggregateState, Capabilities, ConnTableFunction, ConnTableImpl, Executor,
    Host, HostOptions, HostRegistry, Params, ScalarFunction, SqlError, SqlValue, TableFunction,
};
pub use handle::{IntoObj, Obj, TxHandle, TxOps};
pub use id::{Eid, ObjectId, Tag, TxId};
pub use mapping::Vocab;
pub use recall::{RecallHit, RecallQuery};
pub use report::{
    AssertOpts, Asserted, IdUsage, OnExisting, Patch, PatchField, RetKind, Triple, TxOptions,
    TxReport, Valid,
};
pub use term::{TermDict, TermReader};
pub use text::{TextEvidence, TextHit, TextMode, TextQuery};
pub use value::Value;
pub use vector::{EmbeddingModel, Metric, VectorEncoding, VectorHit, VectorQuery, VectorResult};
pub use view::{scan_predicates, TimeRef, TxSel, ValidSel, ViewSpec};
