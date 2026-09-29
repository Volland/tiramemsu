//! Planner statistics kept by the store (design D-18).

use crate::exec::Executor;

/// Counts commits and runs `PRAGMA optimize` when statistics may be stale.
#[derive(Clone, Debug)]
pub struct Stats {
    every: u64,
    since: u64,
    runs: u64,
}

impl Stats {
    /// A counter that optimises every `every` commits (and after bulk loads).
    pub fn new(every: u64) -> Stats {
        Stats {
            every: every.max(1),
            since: 0,
            runs: 0,
        }
    }

    /// How many times `PRAGMA optimize` ran after a commit since opening.
    pub fn runs(&self) -> u64 {
        self.runs
    }

    /// Called after every successful `COMMIT` of a real transaction (never for dry
    /// runs or speculation). Runs `PRAGMA optimize` on the `every`-th commit, or when
    /// the commit inserted at least `every` statements (a bulk load). Errors are
    /// ignored: the transaction has committed and statistics never change results.
    // @lat: [[query#Physical Planning#Join Ordering]]
    pub fn after_commit(&mut self, exec: &mut dyn Executor, inserted: usize) {
        self.since += 1;
        if self.since >= self.every || inserted as u64 >= self.every {
            self.since = 0;
            self.runs += 1;
            let _ = exec.execute_batch("PRAGMA optimize");
        }
    }
}

/// Runs a full `ANALYZE`.
pub fn analyze(exec: &mut dyn Executor) -> crate::error::Result<()> {
    exec.execute_batch("ANALYZE")
}

/// Runs `PRAGMA optimize=0x10002` (analyse tables that were never analysed).
pub fn optimize_at_open(exec: &mut dyn Executor) -> crate::error::Result<()> {
    exec.execute_batch("PRAGMA optimize=0x10002")
}
