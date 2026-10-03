//! Bulk import sessions with deferred statistics (OpenSpec change `add-bulk-import`).
//!
//! A session holds the database's write lease, commits each chunk as one ordinary
//! transaction, suppresses the per-commit statistics trigger, and refreshes the
//! planner statistics once when it is finished.

use std::ops::Deref;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tm_core::{Error, Result, Tx, TxId, TxOptions, TxReport};

use crate::budget::QueryBudget;
use crate::db::Db;

static NEXT_LEASE: AtomicU64 = AtomicU64::new(1);

/// What a bulk import session has done so far.
///
/// Committed rows, rejected chunks and maintenance time are counted apart: a
/// rejected chunk adds nothing to the row counts, and `maintenance` is the time
/// spent in the final analysis only.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportProgress {
    /// Chunks that committed.
    pub chunks: u64,
    /// Chunks that failed and were rolled back (they left no trace).
    pub rejected: u64,
    /// New statements committed by the chunks (`TxReport::asserted`).
    pub asserted: u64,
    /// Assertions that reused a live statement (`TxReport::existing`).
    pub existing: u64,
    /// Statements retracted by the chunks.
    pub retracted: u64,
    /// The transaction number of every committed chunk, in commit order.
    pub txs: Vec<TxId>,
    /// Time spent running chunks, committed or rejected.
    pub elapsed: Duration,
    /// Time spent in the final statistics refresh.
    pub maintenance: Duration,
}

impl ImportProgress {
    fn record(&mut self, report: &TxReport) {
        self.chunks += 1;
        self.asserted += report.asserted.len() as u64;
        self.existing += report.existing.len() as u64;
        self.retracted += report.retracted.len() as u64;
        self.txs.push(report.t);
    }
}

/// The outcome of [`BulkImport::finish`].
///
/// The data and the maintenance are reported apart: every chunk in
/// `progress.txs` is committed whatever happened to the final analysis. A failed
/// analysis is `maintenance_error`, never a rollback.
#[derive(Debug)]
pub struct ImportSummary {
    /// The session's final progress, `maintenance` included.
    pub progress: ImportProgress,
    /// True when the final full analysis succeeded.
    pub analyzed: bool,
    /// The error of the final analysis, if it failed. The chunks stay committed.
    pub maintenance_error: Option<Error>,
    /// True when statistics are still stale after the session (the analysis
    /// failed); the next ordinary commit or [`Db::optimize`] refreshes them.
    pub statistics_due: bool,
}

/// A borrowed or shared database handle.
#[derive(Debug)]
enum DbRef<'db> {
    Borrowed(&'db Db),
    Shared(Arc<Db>),
}

impl Deref for DbRef<'_> {
    type Target = Db;
    fn deref(&self) -> &Db {
        match self {
            DbRef::Borrowed(db) => db,
            DbRef::Shared(db) => db,
        }
    }
}

/// An opt-in bulk import session: chunks of writes, each one ordinary atomic
/// transaction, with planner statistics refreshed once at the end instead of after
/// every large commit.
///
/// - **Lease:** while the session lives it holds the database's write lease;
///   other writes (and a second session) fail with [`Error::ImportInProgress`].
///   Readers are unaffected and see the last committed chunk.
/// - **Chunks:** [`BulkImport::chunk`] runs the existing transaction engine, so a
///   failing chunk rolls back alone and earlier chunks stay committed. This is not
///   one atomic multi-chunk transaction.
/// - **Statistics:** chunks never trigger `ANALYZE`. [`BulkImport::finish`] runs one
///   full analysis and makes pooled readers reload the statistics.
/// - **Interruption:** [`BulkImport::cancel`] or dropping the session releases the
///   lease without discarding committed chunks and without running analysis; the
///   statistics are left due and the next ordinary commit refreshes them.
///
/// ```
/// # use tiramemsu::*;
/// # let dir = tempfile::tempdir().unwrap();
/// # let db = Db::open(dir.path().join("m.db"), OpenOptions::default())?;
/// let v = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
/// let mut import = db.bulk_import()?;
/// for chunk in 0..3 {
///     import.chunk(|tx| {
///         for i in 0..10 {
///             tx.assert(v(&format!("n{chunk}-{i}")), v("p"), Value::Int(i), Valid::ALWAYS)?;
///         }
///         Ok(())
///     })?;
/// }
/// let summary = import.finish();
/// assert_eq!(summary.progress.chunks, 3);
/// assert_eq!(summary.progress.asserted, 30);
/// assert!(summary.analyzed && summary.maintenance_error.is_none());
/// # Ok::<(), Error>(())
/// ```
// @lat: [[query#Bulk Import]]
#[derive(Debug)]
pub struct BulkImport<'db> {
    db: DbRef<'db>,
    lease: u64,
    progress: ImportProgress,
    open: bool,
}

impl Db {
    /// Starts a bulk import session on this database. See [`BulkImport`].
    ///
    /// # Errors
    ///
    /// `ImportInProgress` when another session holds the lease, `Reentrant`
    /// inside a running transaction.
    pub fn bulk_import(&self) -> Result<BulkImport<'_>> {
        BulkImport::begin(DbRef::Borrowed(self))
    }

    /// [`Db::bulk_import`] on a shared handle: the session owns a clone of the
    /// `Arc`, so it can be stored and moved between threads (bindings do this).
    ///
    /// # Errors
    ///
    /// As [`Db::bulk_import`].
    pub fn bulk_import_shared(self: &Arc<Db>) -> Result<BulkImport<'static>> {
        BulkImport::begin(DbRef::Shared(self.clone()))
    }
}

impl<'db> BulkImport<'db> {
    fn begin(db: DbRef<'db>) -> Result<BulkImport<'db>> {
        let lease = NEXT_LEASE.fetch_add(1, Ordering::Relaxed);
        db.acquire_import_lease(lease)?;
        Ok(BulkImport {
            db,
            lease,
            progress: ImportProgress::default(),
            open: true,
        })
    }

    /// Runs one chunk as one ordinary transaction under the session's lease and
    /// returns its report. Statistics are not analysed after the commit.
    ///
    /// # Errors
    ///
    /// Whatever [`Db::transact`] raises. The chunk is rolled back alone and counted
    /// in `rejected`; earlier chunks stay committed and the session stays usable.
    pub fn chunk<F>(&mut self, f: F) -> Result<TxReport>
    where
        F: FnOnce(&mut Tx<'_>) -> Result<()>,
    {
        self.chunk_with(TxOptions::default(), None, f)
    }

    /// [`BulkImport::chunk`] with transaction options and an optional budget that
    /// bounds this chunk like [`Db::transact_budgeted`]. A dry-run chunk commits
    /// nothing and is counted neither as committed nor as rejected.
    ///
    /// # Errors
    ///
    /// As [`BulkImport::chunk`], plus the budget's `Cancelled` and
    /// `DeadlineExceeded`.
    pub fn chunk_with<F>(
        &mut self,
        opts: TxOptions,
        budget: Option<&QueryBudget>,
        f: F,
    ) -> Result<TxReport>
    where
        F: FnOnce(&mut Tx<'_>) -> Result<()>,
    {
        let start = Instant::now();
        let lease = self.lease;
        let db = &*self.db;
        let r = crate::budget::run(budget, || db.transact_leased(opts, Some(lease), f));
        self.progress.elapsed += start.elapsed();
        match &r {
            Ok(report) if !opts.dry_run => self.progress.record(report),
            Ok(_) => {}
            Err(_) => self.progress.rejected += 1,
        }
        r
    }

    /// The progress so far.
    pub fn progress(&self) -> &ImportProgress {
        &self.progress
    }

    /// Ends the session: runs one full `ANALYZE`, makes pooled readers reload the
    /// statistics, and releases the lease. Never fails: a failed analysis is
    /// reported in [`ImportSummary::maintenance_error`] while every committed chunk
    /// stays committed.
    pub fn finish(mut self) -> ImportSummary {
        let start = Instant::now();
        let r = self.db.finish_import(self.lease);
        self.progress.maintenance = start.elapsed();
        self.open = false;
        let statistics_due = self.db.statistics_due().unwrap_or(true);
        ImportSummary {
            progress: std::mem::take(&mut self.progress),
            analyzed: r.is_ok(),
            maintenance_error: r.err(),
            statistics_due,
        }
    }

    /// Ends the session without analysis and returns its progress. Committed
    /// chunks are kept, the statistics are left due for the next ordinary commit
    /// (or [`Db::optimize`]), and ordinary writes can resume at once. Dropping the
    /// session does the same.
    pub fn cancel(mut self) -> ImportProgress {
        self.release();
        std::mem::take(&mut self.progress)
    }

    fn release(&mut self) {
        if self.open {
            self.open = false;
            self.db.release_import_lease(self.lease);
        }
    }
}

impl Drop for BulkImport<'_> {
    fn drop(&mut self) {
        // only the lease: dropping must never run the expensive analysis
        self.release();
    }
}
