//! Tiramemsu Datalog (OpenSpec change `add-datalog-engine`, `lat.md/datalog.md`).
#![doc = include_str!("../README.md")]

pub mod enforce;
pub mod eval;
pub mod interval;
pub mod magic;
pub mod materialize;
pub mod program;
pub mod rel;
pub mod run;
pub mod source;
pub mod syntax;
pub mod terms;

pub use eval::{Limit, Limits};
pub use program::{Analysis, StratumInfo};
pub use run::{
    check, explain, info, query, reason, reason_excluding, stored_program, validate, AccessMode,
    CheckResult, Conclusion, ConstraintReport, DatalogOptions, DatalogResult, Explain, Fact,
    FragmentProof, Premise, Program, ProgramInfo, Proof, ReasonResult, Row, Scope, Status,
    TimeMode, Violation,
};
pub use syntax::Severity;
pub use terms::Term;
