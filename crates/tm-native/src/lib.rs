#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod bgp;
pub mod exec;
pub mod expr;
pub mod funcs;
pub mod path;
pub mod rel;
pub mod source;
pub mod value;

pub use exec::{execute, explain, Exec};
