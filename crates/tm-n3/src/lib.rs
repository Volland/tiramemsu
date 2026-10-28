#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod reason;
mod rules;
mod term;
pub mod vocab;

pub use reason::{
    reason, Conclusion, Contradiction, Fact, Input, Limit, Limits, Premise, Proof, ReasonOptions,
    ReasonResult, Rules, Scope, Status, SupportKey, TimeMode,
};
pub use rules::{Monotonicity, RuleSet, RuleSetInfo};
pub use term::{from_n3, to_n3, value_of, Term};

/// The N3 reasoner this crate drives.
pub use eyeron;
