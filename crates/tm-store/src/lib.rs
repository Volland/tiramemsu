#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod conflict;
pub mod contract;
pub mod copy;
pub mod dict;
pub mod engine;
pub mod evidence;
pub mod mem;
pub mod read;
pub mod search;
pub mod store;
pub mod text;

pub use engine::{Engine, EngineOptions, IntoObject, PredicateSchema, Tx};
pub use read::View;
pub use store::{
    Backend, EmbeddingRow, Pattern, ReadStore, RowIter, Scope, StmtRow, TermRow, TextDoc,
    VolatileRow, WriteStore, WriteTxn,
};
