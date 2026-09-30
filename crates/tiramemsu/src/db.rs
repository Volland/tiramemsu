//! The database handle: one writer behind a mutex, a pool of readers.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tm_core::{
    storage, Capabilities, Clock, Error, Event, Executor, Host, HostOptions, ObjectId, Result,
    Store, StoreOptions, SystemClock, TermReader, TimeRef, Tx, TxOptions, TxReport, ViewSpec,
};
use tm_exec::{
    NativeKind, NativeOperator, OperatorRegistry, PathEngine, PathOperator, PathOptions,
    PlannerOptions, QueryEngine,
};
use tm_rusqlite::RusqliteHost;

use crate::pool::ReaderPool;
use crate::view::View;

/// Per-database tuning knobs, passed to [`Db::open`].
///
/// `OpenOptions::default()` suits most applications. Override `clock` to make
/// transaction instants deterministic in tests, `readers` to size the read pool,
/// and `path_max_hops` / `path_max_states` to bound path searches.
///
/// ```
/// # use tiramemsu::*;
/// # let dir = tempfile::tempdir().unwrap();
/// let opts = OpenOptions { readers: 2, path_max_states: 100_000, ..OpenOptions::default() };
/// let db = Db::open(dir.path().join("m.db"), opts)?;
/// assert_eq!(db.reader_count(), 2);
/// # Ok::<(), Error>(())
/// ```
#[derive(Clone)]
pub struct OpenOptions {
    /// Number of read-only connections (default 4). Ignored when the host has no
    /// reader pool.
    pub readers: usize,
    /// Clock for transaction instants (default: the system clock).
    pub clock: Arc<dyn Clock>,
    /// How long to wait for a lock before failing with `SQLITE_BUSY` (default 5 s).
    pub busy_timeout: Duration,
    /// Size of the term caches (default 16 384).
    pub term_cache_capacity: usize,
    /// Run `PRAGMA optimize` every this many commits, and after a commit that
    /// inserted at least this many statements (default 1000).
    pub optimize_every: u64,
    /// Query planner options (default: SQL routing, LFTJ off).
    pub planner: PlannerOptions,
    /// Open the query engine (default true). It needs the host capabilities
    /// `functions` and `vtab`; opening fails with `MissingCapability` on a host
    /// without them. `false` opens the `tm-core` tier only (no `execute_ir`).
    pub query_engine: bool,
    /// The hop cap of unbounded Cypher path patterns and the default `max_hops` of
    /// `tm_path` in `TRAIL` mode (default 15). Reaching it stops paths without an
    /// error.
    pub path_max_hops: u32,
    /// The bound on the search states of one path evaluation (default 1 000 000).
    /// Exceeding it fails with `PathLimitExceeded`.
    pub path_max_states: usize,
    /// Native operators registered on every connection (tests). A registered
    /// `Path` operator replaces the built-in `tm_path`.
    #[doc(hidden)]
    pub native_operators: Vec<Arc<dyn NativeOperator>>,
}

impl OpenOptions {
    /// Registers a native operator (the `tm_path` path operator of M3, or a test
    /// operator).
    #[doc(hidden)]
    pub fn with_native_operator(mut self, op: Arc<dyn NativeOperator>) -> OpenOptions {
        self.native_operators.push(op);
        self
    }
}

impl Default for OpenOptions {
    fn default() -> Self {
        OpenOptions {
            readers: 4,
            clock: Arc::new(SystemClock),
            busy_timeout: Duration::from_secs(5),
            term_cache_capacity: 16_384,
            optimize_every: 1000,
            planner: PlannerOptions::default(),
            query_engine: true,
            path_max_hops: 15,
            path_max_states: 1_000_000,
            native_operators: Vec::new(),
        }
    }
}

impl std::fmt::Debug for OpenOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenOptions")
            .field("readers", &self.readers)
            .field("busy_timeout", &self.busy_timeout)
            .field("term_cache_capacity", &self.term_cache_capacity)
            .field("optimize_every", &self.optimize_every)
            .field("planner", &self.planner)
            .field("query_engine", &self.query_engine)
            .field("path_max_hops", &self.path_max_hops)
            .field("path_max_states", &self.path_max_states)
            .finish()
    }
}

static NEXT_DB_ID: AtomicU64 = AtomicU64::new(1);

thread_local! {
    /// The databases whose writer this thread currently holds.
    static HELD: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
}

/// Marks the writer of one database as held by this thread.
struct HeldGuard(u64);

impl HeldGuard {
    fn acquire(id: u64) -> Result<HeldGuard> {
        HELD.with(|h| {
            let mut h = h.borrow_mut();
            if h.contains(&id) {
                return Err(Error::Reentrant);
            }
            h.push(id);
            Ok(HeldGuard(id))
        })
    }

    fn is_held(id: u64) -> bool {
        HELD.with(|h| h.borrow().contains(&id))
    }
}

impl Drop for HeldGuard {
    fn drop(&mut self) {
        HELD.with(|h| h.borrow_mut().retain(|x| *x != self.0));
    }
}

/// An open database: one writer connection behind a mutex and, when the host
/// declares `reader_pool`, a pool of read-only connections on the same WAL file.
pub struct Db {
    id: u64,
    writer: Mutex<Store>,
    pool: Option<ReaderPool>,
    terms: TermReader,
    engine: Option<Arc<QueryEngine>>,
    caps: Capabilities,
    path_max_hops: u32,
    path: PathBuf,
    clock: Arc<dyn Clock>,
}

impl std::fmt::Debug for Db {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Db").field("path", &self.path).finish()
    }
}

impl Db {
    /// Opens (creating or migrating) the database at `path` with the bundled
    /// `rusqlite` host. This is the usual entry point: it starts the single writer
    /// and the reader pool and installs the query engine.
    ///
    /// # Errors
    ///
    /// `FormatVersion` when the file was written by a newer format, `ForeignFile`
    /// when it is a SQLite database that is not a tiramemsu one, and `Sqlite` for
    /// I/O or locking failures.
    ///
    /// ```
    /// # use tiramemsu::*;
    /// # let dir = tempfile::tempdir().unwrap();
    /// let db = Db::open(dir.path().join("memory.db"), OpenOptions::default())?;
    /// assert!(db.now().triples(None, None, None)?.is_empty());
    /// # Ok::<(), Error>(())
    /// ```
    pub fn open(path: impl AsRef<Path>, opts: OpenOptions) -> Result<Db> {
        Db::open_with_host(RusqliteHost::new(), path, opts)
    }

    /// Opens the database at `path` through any executor [`Host`], for embedding on
    /// a SQLite other than the bundled one.
    ///
    /// # Errors
    ///
    /// As [`Db::open`], plus `MissingCapability` when `opts.query_engine` is set and
    /// the host lacks `functions` or `vtab`.
    pub fn open_with_host(
        host: impl Host,
        path: impl AsRef<Path>,
        opts: OpenOptions,
    ) -> Result<Db> {
        let path = path.as_ref();
        let engine = if opts.query_engine {
            // refuse before touching the file: no query is ever planned on a host
            // without `functions` and `vtab` (decision D22)
            tm_exec::host::check_capabilities(host.capabilities())?;
            let mut reg = OperatorRegistry::new();
            if !opts
                .native_operators
                .iter()
                .any(|o| o.kind() == NativeKind::Path)
            {
                let engine = Arc::new(PathEngine::new(PathOptions {
                    max_hops: opts.path_max_hops,
                    max_states: opts.path_max_states,
                    ..PathOptions::default()
                }));
                reg.add(Arc::new(PathOperator::new(engine)));
            }
            for op in &opts.native_operators {
                reg.add(op.clone());
            }
            Some(Arc::new(QueryEngine::new(
                opts.planner,
                reg,
                opts.term_cache_capacity,
            )))
        } else {
            None
        };
        let mut store = Store::open(
            &host,
            path,
            StoreOptions {
                clock: opts.clock.clone(),
                busy_timeout: opts.busy_timeout,
                term_cache_capacity: opts.term_cache_capacity,
                optimize_every: opts.optimize_every,
            },
        )?;
        let caps = store.capabilities();
        if let Some(e) = &engine {
            e.install(store.executor())?;
        }
        let pool = if caps.reader_pool && opts.readers > 0 {
            let hopts = HostOptions {
                busy_timeout: opts.busy_timeout,
            };
            let mut readers = Vec::with_capacity(opts.readers);
            for _ in 0..opts.readers {
                let mut r = host.open_reader(path, &hopts)?;
                storage::configure_reader(r.as_mut())?;
                if let Some(e) = &engine {
                    e.install(r.as_mut())?;
                }
                readers.push(r);
            }
            Some(ReaderPool::new(readers))
        } else {
            None
        };
        Ok(Db {
            id: NEXT_DB_ID.fetch_add(1, Ordering::Relaxed),
            writer: Mutex::new(store),
            pool,
            terms: TermReader::new(opts.term_cache_capacity),
            engine,
            caps,
            path_max_hops: opts.path_max_hops,
            path: path.to_path_buf(),
            clock: opts.clock.clone(),
        })
    }

    /// The database clock (`NOW()` in SPARQL).
    pub(crate) fn now_ms(&self) -> i64 {
        self.clock.now_ms()
    }

    /// The capabilities declared by the host, for crates that need one.
    pub fn capabilities(&self) -> Capabilities {
        self.caps
    }

    /// The hop cap of unbounded Cypher path patterns (`OpenOptions::path_max_hops`).
    pub fn path_max_hops(&self) -> u32 {
        self.path_max_hops
    }

    /// The database file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Number of read-only connections (0 when the writer serves reads).
    pub fn reader_count(&self) -> usize {
        self.pool.as_ref().map_or(0, ReaderPool::size)
    }

    /// How many times the writer ran `PRAGMA optimize` after a commit since opening.
    pub fn optimize_runs(&self) -> Result<u64> {
        let _held = HeldGuard::acquire(self.id)?;
        Ok(self.lock()?.optimize_runs())
    }

    fn lock(&self) -> Result<MutexGuard<'_, Store>> {
        Ok(self.writer.lock().unwrap_or_else(|p| p.into_inner()))
    }

    pub(crate) fn term_reader(&self) -> &TermReader {
        &self.terms
    }

    pub(crate) fn engine(&self) -> Option<&QueryEngine> {
        self.engine.as_deref()
    }

    /// Number of terms in the query engine's shared term cache (tests).
    #[doc(hidden)]
    pub fn term_cache_len(&self) -> usize {
        self.engine.as_ref().map_or(0, |e| e.term_cache().len())
    }

    /// True when the query engine's shared term cache holds `id` (tests).
    #[doc(hidden)]
    pub fn term_cache_contains(&self, id: ObjectId) -> bool {
        self.engine
            .as_ref()
            .is_some_and(|e| e.term_cache().contains(id))
    }

    /// Sets a hook run by every query between planning and its SQL statement
    /// (snapshot tests).
    #[doc(hidden)]
    pub fn set_query_hook(&self, f: Option<Arc<dyn Fn() + Send + Sync>>) {
        if let Some(e) = &self.engine {
            e.set_test_hook(f);
        }
    }

    /// Runs a read in one committed snapshot: on a reader, or on the writer (under
    /// its mutex) when the host has no reader pool.
    pub(crate) fn read_committed<R>(
        &self,
        f: impl FnOnce(&mut dyn Executor) -> Result<R>,
    ) -> Result<R> {
        match &self.pool {
            Some(pool) => pool.read(f),
            None => {
                if HeldGuard::is_held(self.id) {
                    return Err(Error::Reentrant);
                }
                let _held = HeldGuard::acquire(self.id)?;
                self.lock()?.read(f)
            }
        }
    }

    /// Runs a full `ANALYZE` on the writer. Worth calling once after a large bulk
    /// load; normal operation runs `PRAGMA optimize` on its own (`optimize_every`).
    ///
    /// # Errors
    ///
    /// `Reentrant` inside a running transaction, or `Sqlite`.
    pub fn optimize(&self) -> Result<()> {
        let _held = HeldGuard::acquire(self.id)?;
        self.lock()?.optimize()
    }

    /// Runs one transaction on the single writer and returns its report. This is the
    /// only way to write: assert, retract, supersede and the rest are methods of the
    /// [`Tx`] the closure receives.
    ///
    /// If the body or any operation fails, the whole transaction is rolled back and
    /// leaves no trace: no `tx` row, statement, term, volatile change or counter
    /// change. Ids seen inside a failed body may therefore be reissued; do not
    /// persist them. With `opts.dry_run`, every operation runs with full semantics,
    /// the report is returned and all effects are discarded (allocated ids are
    /// burned). Starting a write from inside another write or speculation on the
    /// same database fails with [`Error::Reentrant`].
    ///
    /// # Errors
    ///
    /// Whatever the operations raise (schema violations, `NotLive`,
    /// `CascadeLimitExceeded`, ...), an error returned by the closure, or `Sqlite`.
    ///
    /// ```
    /// # use tiramemsu::*;
    /// # let dir = tempfile::tempdir().unwrap();
    /// # let db = Db::open(dir.path().join("m.db"), OpenOptions::default())?;
    /// let v = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
    /// let report = db.transact(TxOptions::default(), |tx| {
    ///     tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?;
    ///     Ok(())
    /// })?;
    /// assert_eq!(report.t, TxId(1));
    ///
    /// // A failing body commits nothing, not even the first assert.
    /// let failed = db.transact(TxOptions::default(), |tx| {
    ///     tx.assert(v("bob"), v("worksAt"), v("acme"), Valid::ALWAYS)?;
    ///     Err(Error::custom("changed my mind"))
    /// });
    /// assert!(failed.is_err());
    /// assert_eq!(db.now().triples(None, None, None)?.len(), 1);
    /// # Ok::<(), Error>(())
    /// ```
    pub fn transact<F>(&self, opts: TxOptions, f: F) -> Result<TxReport>
    where
        F: FnOnce(&mut Tx<'_>) -> Result<()>,
    {
        let _held = HeldGuard::acquire(self.id)?;
        let engine = self.engine.clone();
        self.lock()?.transact(opts, move |tx| {
            if let Some(e) = engine {
                tx.set_extension(e);
            }
            f(tx)
        })
    }

    /// Speculation: applies `ops` hypothetically on the single writer, calls `query`
    /// with a now view that sees the uncommitted state, then discards everything.
    /// No transaction number is consumed and no event is logged; ids allocated
    /// inside are burned so they are never reissued. `query` is not called when
    /// `ops` fails.
    ///
    /// Use it for "what would happen if" questions; it holds the write lock, so keep
    /// it short. To preview only a report, use `TxOptions { dry_run: true, .. }`.
    ///
    /// # Errors
    ///
    /// The error of `ops` or of `query`, or `Reentrant` inside another write.
    ///
    /// ```
    /// # use tiramemsu::*;
    /// # let dir = tempfile::tempdir().unwrap();
    /// # let db = Db::open(dir.path().join("m.db"), OpenOptions::default())?;
    /// let v = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
    /// let seen = db.with(
    ///     |tx| { tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?; Ok(()) },
    ///     |view| Ok(view.triples(None, None, None)?.len()),
    /// )?;
    /// assert_eq!(seen, 1);
    /// assert!(db.now().triples(None, None, None)?.is_empty()); // nothing was kept
    /// # Ok::<(), Error>(())
    /// ```
    pub fn with<F, G, R>(&self, ops: F, query: G) -> Result<R>
    where
        F: FnOnce(&mut Tx<'_>) -> Result<()>,
        G: FnOnce(&View<'_>) -> Result<R>,
    {
        let _held = HeldGuard::acquire(self.id)?;
        let engine = self.engine.as_deref();
        self.lock()?.speculate(ops, |exec| {
            let cell: RefCell<&mut dyn Executor> = RefCell::new(exec);
            let view = View::on_writer(&cell, ViewSpec::NOW, engine);
            query(&view)
        })
    }

    /// The now view: live statements, valid time unfiltered. Views are cheap values;
    /// creating one does no I/O.
    pub fn now(&self) -> View<'_> {
        View::on_db(self, ViewSpec::NOW)
    }

    /// The as-of view at a transaction number or a wall-clock instant: what the
    /// database believed then. Exact for every past `t`, since nothing is deleted.
    /// An instant resolves to the last transaction at or before it, and to the empty
    /// view before the first.
    pub fn as_of(&self, at: TimeRef) -> View<'_> {
        View::on_db(self, ViewSpec::as_of(at))
    }

    /// The history view: every statement ever committed, with its real lifetime
    /// (`t_add`, `t_ret`, `ret_kind`). Use it for audits and "how did this change".
    pub fn history(&self) -> View<'_> {
        View::on_db(self, ViewSpec::history())
    }

    /// Every event with `t > since`, ordered by time, asserts before retracts, eid:
    /// the change log for replication, auditing and "what happened since I last looked".
    ///
    /// # Errors
    ///
    /// `Sqlite` on a read failure.
    pub fn events_since(&self, since: u64) -> Result<Vec<Event>> {
        self.now().events_since(since)
    }

    /// Runs raw SQL on the read path (diagnostics and tests).
    #[doc(hidden)]
    pub fn read_sql(&self, sql: &str) -> Result<Vec<Vec<tm_core::SqlValue>>> {
        self.read_committed(|e| e.rows(sql, &[]))
    }
}
