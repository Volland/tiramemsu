//! Query budgets: per-operation deadlines, cancellation, reader timeouts and result
//! limits (OpenSpec change `add-query-budgets`).

use std::time::Duration;

use tm_core::budget::{self, CancelToken, Interrupt, Meter};
use tm_core::Result;

/// The bounds of one operation, for [`View::with_budget`](crate::View::with_budget),
/// [`Db::transact_budgeted`](crate::Db::transact_budgeted) and
/// [`Db::cypher_write_budgeted`](crate::Db::cypher_write_budgeted).
///
/// `QueryBudget::default()` bounds nothing, which is exactly the behaviour of the
/// calls without a budget. Every field is independent:
///
/// - `timeout` starts when the operation starts and covers waiting for a connection,
///   SQL execution and native work (path search); past it the operation fails with
///   `DeadlineExceeded`.
/// - `cancel` stops the operation with `Cancelled` once its token is cancelled from
///   any thread.
/// - `reader_timeout` bounds the wait for a read connection (overriding
///   `OpenOptions::reader_timeout`); past it the operation fails with `PoolTimeout`.
///   It is separate from `OpenOptions::busy_timeout`, SQLite's own lock wait.
/// - `max_rows` and `max_bytes` cap what the whole operation decodes, across every
///   statement it runs (provenance lookups, Cypher sub-queries, the `WHERE` of an
///   update); past either the operation fails with `ResultLimitExceeded` and returns
///   no partial result.
///
/// A stopped read releases its connection; a stopped write rolls back and commits
/// nothing. Build it with `..Default::default()` so fields added later keep their
/// defaults.
///
/// ```
/// # use tiramemsu::*;
/// # use std::time::Duration;
/// # let dir = tempfile::tempdir().unwrap();
/// # let db = Db::open(dir.path().join("m.db"), OpenOptions::default())?;
/// db.now().sparql("INSERT DATA { v:a v:p 1 , 2 , 3 }")?;
/// let budget = QueryBudget {
///     timeout: Some(Duration::from_secs(2)),
///     max_rows: Some(2),
///     ..Default::default()
/// };
/// let r = db.now().with_budget(&budget).sparql("SELECT ?o WHERE { v:a v:p ?o }");
/// assert!(matches!(r, Err(Error::ResultLimitExceeded { .. })));
///
/// let token = CancelToken::new();
/// let budget = QueryBudget { cancel: Some(token.clone()), ..Default::default() };
/// token.cancel(); // from any thread; here before the query starts
/// let r = db.now().with_budget(&budget).sparql("SELECT ?o WHERE { v:a v:p ?o }");
/// assert!(matches!(r, Err(Error::Cancelled)));
/// # Ok::<(), Error>(())
/// ```
#[derive(Clone, Debug, Default)]
pub struct QueryBudget {
    /// The longest the whole operation may take (`None`: no deadline).
    pub timeout: Option<Duration>,
    /// A token that cancels the operation from another thread.
    pub cancel: Option<CancelToken>,
    /// How long to wait for a read connection (`None`: `OpenOptions::reader_timeout`).
    pub reader_timeout: Option<Duration>,
    /// The most rows the operation may decode across all its statements.
    pub max_rows: Option<u64>,
    /// The most decoded result bytes (8 per cell plus string lengths) the operation
    /// may produce across all its statements.
    pub max_bytes: Option<u64>,
}

impl QueryBudget {
    /// Runs `f` as one operation under this budget: every read, query and write
    /// `f` makes on this thread draws on the same deadline and the same row and
    /// byte budget, as if it were one call. Use it to bound a sequence of calls
    /// (an agent step) as a whole. Inside an operation that already has a budget,
    /// `f` runs under that outer budget.
    ///
    /// ```
    /// # use tiramemsu::*;
    /// # let dir = tempfile::tempdir().unwrap();
    /// # let db = Db::open(dir.path().join("m.db"), OpenOptions::default())?;
    /// db.now().sparql("INSERT DATA { v:a v:p 1 , 2 }")?;
    /// let budget = QueryBudget { max_rows: Some(3), ..Default::default() };
    /// let r = budget.run(|| {
    ///     db.now().triples(None, None, None)?; // 2 rows
    ///     db.now().triples(None, None, None) // 2 more: over the budget of 3
    /// });
    /// assert!(matches!(r, Err(Error::ResultLimitExceeded { .. })));
    /// # Ok::<(), Error>(())
    /// ```
    pub fn run<R>(&self, f: impl FnOnce() -> Result<R>) -> Result<R> {
        run(Some(self), f)
    }

    /// The meter of one run of an operation: the deadline starts now.
    fn meter(&self) -> Meter {
        let mut m = Meter::new(Interrupt::new(self.timeout, self.cancel.clone()));
        m.reader_timeout = self.reader_timeout;
        m.max_rows = self.max_rows;
        m.max_bytes = self.max_bytes;
        m
    }
}

/// Runs one operation under `budget`. Without a budget, or inside an operation that
/// already runs under one (the outer budget covers the whole operation), `f` runs
/// as is.
// @lat: [[query#Query Budgets]]
pub(crate) fn run<R>(budget: Option<&QueryBudget>, f: impl FnOnce() -> Result<R>) -> Result<R> {
    match budget {
        Some(b) if !budget::active() => budget::scope(b.meter(), || {
            budget::check()?;
            f().map_err(budget::map_interrupt)
        }),
        _ => f(),
    }
}
