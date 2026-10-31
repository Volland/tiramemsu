//! Tiramemsu ELPI persistence (OpenSpec changes `add-elpi-programs` and
//! `add-elpi-materialization`, `lat.md/elpi.md`).
#![doc = include_str!("../README.md")]

pub mod enforce;
pub mod plan;
pub mod programs;

pub use enforce::{commit_check, Notes};
pub use plan::{plan, Plan, PlanItem};
pub use programs::{enforce, info, resolve, resolve_text, store_program, ProgramInfo};

use tm_core::sqlstore::SqlStore;
use tm_core::store::ReadStore;
use tm_core::{Result, TxHandle};

/// Runs `f` on the transaction's own state, on either engine.
pub(crate) fn on_tx<R>(
    tx: &mut TxHandle<'_>,
    f: impl Fn(&dyn ReadStore) -> Result<R>,
) -> Result<R> {
    if let Some(r) = tx.read_store(&f) {
        return r;
    }
    tx.read_with(|e| f(&SqlStore::new(e)))
}
