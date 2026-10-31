//! Tiramemsu ELPI (OpenSpec change `add-elpi-engine`, `lat.md/elpi.md`).
#![doc = include_str!("../README.md")]

mod engine;
pub mod host;
pub mod run;
pub mod scan;
pub mod term;
pub mod valid;

pub use host::{Reading, Seeds};
pub use run::{
    check, check_delta, conclude, query, validate, CheckResult, ConcludeResult, Conclusion, Cost,
    ElpiOptions, Part, QueryResult, Row, Severity, Source, Violation, PRELUDE,
};
pub use term::ElpiTerm;

/// True when this build links lean-elpi (cargo feature `native`).
pub const NATIVE: bool = cfg!(feature = "native");
