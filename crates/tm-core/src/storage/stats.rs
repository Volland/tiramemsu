//! Planner statistics kept by the store (design D-18).

use crate::exec::Executor;

/// Counts commits and runs `PRAGMA optimize` when statistics may be stale.
#[derive(Clone, Debug)]
pub struct Stats {
    every: u64,
    since: u64,
    runs: u64,
    /// A bulk import session suppresses the per-commit trigger.
    deferred: bool,
    /// Commits have changed the data since statistics were last refreshed by a
    /// deferred session: the next commit runs the upkeep.
    due: bool,
}

impl Stats {
    /// A counter that optimises every `every` commits (and after bulk loads).
    pub fn new(every: u64) -> Stats {
        Stats {
            every: every.max(1),
            since: 0,
            runs: 0,
            deferred: false,
            due: false,
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
    ///
    /// While statistics are deferred (a bulk import session) no analysis runs: the
    /// commit only marks statistics due. Once deferral ends without a refresh, the
    /// next commit runs the upkeep.
    pub fn after_commit(&mut self, exec: &mut dyn Executor, inserted: usize) {
        if self.deferred {
            self.due = true;
            return;
        }
        self.since += 1;
        if self.due || self.since >= self.every || inserted as u64 >= self.every {
            self.since = 0;
            self.due = false;
            self.runs += 1;
            optimize(exec);
            refresh_readers(exec);
        }
    }

    /// Suppresses (`true`) or restores (`false`) the per-commit trigger. A bulk
    /// import session defers statistics and refreshes them once when it finishes.
    // @lat: [[query#Bulk Import]]
    pub fn set_deferred(&mut self, deferred: bool) {
        self.deferred = deferred;
    }

    /// True while the per-commit trigger is suppressed.
    pub fn deferred(&self) -> bool {
        self.deferred
    }

    /// True when deferred commits changed the data and no refresh has run since.
    pub fn due(&self) -> bool {
        self.due
    }

    /// Records a successful full analysis: statistics are no longer due.
    pub fn refreshed(&mut self) {
        self.due = false;
    }
}

/// Makes other connections reload the planner statistics. SQLite reads
/// `sqlite_stat1` / `sqlite_stat4` only when a connection loads the schema, and
/// `ANALYZE` does not bump the schema cookie, so pooled readers opened before the
/// first analysis would plan without statistics forever. Bumping the cookie makes
/// every reader reload the schema, and the statistics with it, at its next read.
/// Errors are ignored: statistics never change results.
pub fn refresh_readers(exec: &mut dyn Executor) {
    if let Ok(Some(v)) = exec.query_i64("PRAGMA schema_version", &[]) {
        let _ = exec.execute_batch(&format!("PRAGMA schema_version = {}", v + 1));
    }
}

/// Runs a full `ANALYZE`.
pub fn analyze(exec: &mut dyn Executor) -> crate::error::Result<()> {
    exec.execute_batch("ANALYZE")?;
    refresh_readers(exec);
    Ok(())
}

/// Runs `PRAGMA optimize=0x10002` (analyse tables that were never analysed) and,
/// when the file holds statements but no STAT4 samples yet, a full `ANALYZE`.
pub fn optimize_at_open(exec: &mut dyn Executor) -> crate::error::Result<()> {
    let _ = exec.execute_batch("PRAGMA optimize=0x10002");
    let has_rows = exec
        .query_i64("SELECT 1 FROM triple LIMIT 1", &[])
        .ok()
        .flatten()
        .is_some();
    let sampled = exec
        .query_i64(
            "SELECT 1 FROM sqlite_stat4 WHERE tbl = 'triple' LIMIT 1",
            &[],
        )
        .ok()
        .flatten()
        .is_some();
    if has_rows && !sampled {
        let _ = exec.execute_batch("ANALYZE");
    }
    Ok(())
}

/// Statistics upkeep at the trigger points: `PRAGMA optimize` decides which
/// tables are due, but it analyses with an analysis limit and leaves out the STAT4
/// samples that let the planner tell a 50-row predicate from an 18 000-row one
/// (measured: without them the plan started from the big pattern). So the trigger
/// runs a full `ANALYZE` (with STAT4), then the cheap `PRAGMA optimize`. Errors are
/// ignored: statistics never change results.
fn optimize(exec: &mut dyn Executor) {
    let _ = exec.execute_batch("ANALYZE");
    let _ = exec.execute_batch("PRAGMA optimize");
}
