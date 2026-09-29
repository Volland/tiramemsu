//! Rows of the event log (`lat.md/time-model#Event Log`).

use crate::id::{Eid, TxId};
use crate::report::RetKind;

/// The operation of an event.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Op {
    /// A statement was inserted (`t_add`).
    Assert,
    /// A statement was retracted (`t_ret`).
    Retract,
}

/// One row of the `event` view.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Event {
    /// Transaction of the event.
    pub t: TxId,
    /// The statement.
    pub eid: Eid,
    /// Assert or retract.
    pub op: Op,
    /// Retraction kind (retract events only).
    pub kind: Option<RetKind>,
}
