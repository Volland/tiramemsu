//! Tiramemsu Cypher front end (`lat.md/query#Front Ends#Cypher`).
#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod ast;
pub mod error;
pub mod exec;
pub mod funcs;
pub mod parse;
pub mod program;
pub mod runner;
pub mod sema;
pub mod span;
pub mod value;
pub mod vocab;

pub use error::{CResult, CypherError};
pub use program::{compile, CompileCtx, CypherParams, CypherProgram};
pub use runner::{Rows, Runner};
pub use span::Span;
pub use value::{CypherResult, CypherValue, NodeValue, PathValue, RelValue};
pub use vocab::{Vocab, VocabExt};
