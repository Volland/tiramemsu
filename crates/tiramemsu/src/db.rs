//! The database handle: one writer behind a mutex, a pool of readers.

use std::cell::RefCell;
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, TryLockError};
use std::time::{Duration, Instant};

use tm_core::{
    budget, storage, Capabilities, Clock, Error, Event, Executor, Host, HostOptions, Result, Store,
    StoreOptions, SystemClock, TermReader, TimeRef, TxHandle as Tx, TxOptions, TxReport, ViewSpec,
};
#[cfg(feature = "exec")]
use tm_exec::{
    LftjOperator, NativeKind, NativeOperator, OperatorRegistry, PathEngine, PathOperator,
    PathOptions, PlannerOptions,
};
use tm_rusqlite::RusqliteHost;
use tm_store::dict::TermCache;

use crate::backend::{self, Backend, Durability};
use crate::budget::QueryBudget;
use crate::pool::ReaderPool;
use crate::view::View;

/// The query engine a database installs (`OpenOptions::query_engine`).
#[cfg(feature = "exec")]
pub(crate) use tm_exec::QueryEngine as Engine;

/// Without the `exec` feature there is no query engine: the type has no values, so
/// every `Option<&Engine>` is `None`.
#[cfg(not(feature = "exec"))]
pub(crate) enum Engine {}

#[cfg(not(feature = "exec"))]
impl Engine {
    fn install(&self, _: &mut dyn Executor) -> Result<()> {
        match *self {}
    }
}

/// What a database file holds ([`Db::summary`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Summary {
    /// The backend that wrote the file.
    pub backend: Backend,
    /// The storage format version recorded in the file.
    pub format_version: Option<i64>,
    /// Statements not retracted.
    pub live_statements: u64,
    /// Every statement ever asserted, retracted ones included.
    pub statements: u64,
    /// Dictionary terms (inline values such as numbers are not counted).
    pub terms: u64,
    /// Committed transactions.
    pub transactions: u64,
    /// The size of the main file in bytes.
    pub file_size: u64,
}

/// Per-database tuning knobs, passed to [`Db::open`].
///
/// `OpenOptions::default()` suits most applications. Override `clock` to make
/// transaction instants deterministic in tests, `readers` to size the read pool,
/// and `path_max_hops` / `path_max_states` to bound path searches.
///
/// The query-engine fields (`planner`, `query_engine`, `path_max_hops`,
/// `path_max_states`) exist only with the `exec` cargo feature (implied by
/// `sparql` and `cypher`). Build the options with `..OpenOptions::default()`, so
/// that the same code compiles whichever features are enabled.
///
/// ```
/// # use tiramemsu::*;
/// # let dir = tempfile::tempdir().unwrap();
/// let opts = OpenOptions { readers: 2, text_index: true, ..OpenOptions::default() };
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
    #[cfg(feature = "exec")]
    /// Query planner options (default: SQL routing, LFTJ off). Setting
    /// `planner.lftj.enabled` installs the native cyclic-join operator
    /// ([`LftjOperator`], the `tm_lftj` table function) and lets the planner route
    /// pure cyclic BGPs to it when the estimate reaches
    /// `planner.lftj.min_rows_estimate`; see [`LftjConfig`](crate::LftjConfig).
    pub planner: PlannerOptions,
    #[cfg(feature = "exec")]
    /// Open the query engine (default true). It needs the host capabilities
    /// `functions` and `vtab`; opening fails with `MissingCapability` on a host
    /// without them. `false` opens the `tm-core` tier only (no `execute_ir`).
    pub query_engine: bool,
    #[cfg(feature = "exec")]
    /// The hop cap of unbounded Cypher path patterns and the default `max_hops` of
    /// `tm_path` in `TRAIL` mode (default 15). Reaching it stops paths without an
    /// error.
    pub path_max_hops: u32,
    #[cfg(feature = "exec")]
    /// The bound on the search states of one path evaluation (default 1 000 000).
    /// Exceeding it fails with `PathLimitExceeded`.
    pub path_max_states: usize,
    /// How long a read waits for a free read connection before failing with
    /// `PoolTimeout` (default `None`: wait as long as it takes). A
    /// [`QueryBudget::reader_timeout`] overrides it per operation. It bounds only
    /// the wait for a connection; `busy_timeout` is SQLite's own lock wait.
    pub reader_timeout: Option<Duration>,
    /// Build the derived text index at open when it is not current (default
    /// false), so that [`View::text_search`] and the query-language text recall
    /// can run. Once built, the index is kept current by every writer with FTS5,
    /// whatever this option says. Ignored on a host without FTS5, where text
    /// recall fails with `MissingCapability`. See [`Db::rebuild_text_index`].
    pub text_index: bool,
    /// The storage backend of a new file (default `Auto`: SQLite). An existing
    /// file is opened on the backend that wrote it; an explicit choice that
    /// disagrees fails with `BackendMismatch`. See [`Backend`].
    pub backend: Backend,
    /// When commits become durable (default `Normal`). See [`Durability`].
    pub durability: Durability,
    /// redb only: the longest a `Normal` commit stays non-durable (default 50 ms).
    pub flush_interval: Duration,
    /// redb only: a `Normal` commit after this many non-durable ones is durable
    /// (default 1000).
    pub flush_commits: u64,
    /// The bound on the Datalog constraint checks of every commit (default 50 ms
    /// and 100 000 seed rows); `TxOptions::constraint_budget` overrides it.
    pub constraint_budget: tm_core::ConstraintBudget,
    /// redb only: the input rows above which a join step of the native runner
    /// runs in parallel morsels (default 4096); `0` disables parallelism.
    #[cfg(feature = "exec")]
    pub parallel_threshold: usize,
    /// Native operators registered on every connection (tests). A registered
    /// `Path` operator replaces the built-in `tm_path`.
    #[doc(hidden)]
    #[cfg(feature = "exec")]
    pub native_operators: Vec<Arc<dyn NativeOperator>>,
}

#[cfg(feature = "exec")]
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
            #[cfg(feature = "exec")]
            planner: PlannerOptions::default(),
            #[cfg(feature = "exec")]
            query_engine: true,
            #[cfg(feature = "exec")]
            path_max_hops: 15,
            #[cfg(feature = "exec")]
            path_max_states: 1_000_000,
            reader_timeout: None,
            text_index: false,
            backend: Backend::Auto,
            durability: Durability::Normal,
            flush_interval: Duration::from_millis(50),
            flush_commits: 1000,
            constraint_budget: tm_core::ConstraintBudget::default(),
            #[cfg(feature = "exec")]
            parallel_threshold: tm_exec::DEFAULT_PARALLEL_THRESHOLD,
            #[cfg(feature = "exec")]
            native_operators: Vec::new(),
        }
    }
}

impl std::fmt::Debug for OpenOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut d = f.debug_struct("OpenOptions");
        d.field("readers", &self.readers)
            .field("busy_timeout", &self.busy_timeout)
            .field("term_cache_capacity", &self.term_cache_capacity)
            .field("optimize_every", &self.optimize_every);
        #[cfg(feature = "exec")]
        d.field("planner", &self.planner)
            .field("query_engine", &self.query_engine)
            .field("path_max_hops", &self.path_max_hops)
            .field("path_max_states", &self.path_max_states);
        d.field("reader_timeout", &self.reader_timeout)
            .field("text_index", &self.text_index)
            .field("backend", &self.backend)
            .field("durability", &self.durability)
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

/// The single writer: the SQL engine on a SQLite connection, or the
/// storage-trait engine on a key-value store.
pub(crate) enum Writer {
    Sql(Store),
    #[cfg_attr(not(feature = "redb"), allow(dead_code))]
    Kv(tm_store::Engine),
}

impl Writer {
    /// The SQL store, or `MissingCapability("sql")` on a key-value backend.
    pub(crate) fn sql(&mut self) -> Result<&mut Store> {
        match self {
            Writer::Sql(s) => Ok(s),
            Writer::Kv(_) => Err(Error::MissingCapability {
                capability: "sql".to_string(),
            }),
        }
    }

    fn optimize_runs(&self) -> u64 {
        match self {
            Writer::Sql(s) => s.optimize_runs(),
            Writer::Kv(_) => 0,
        }
    }

    fn optimize(&mut self) -> Result<()> {
        match self {
            Writer::Sql(s) => s.optimize(),
            // redb keeps exact statistics in every write transaction
            Writer::Kv(_) => Ok(()),
        }
    }

    fn enable_text_index(&mut self) -> Result<bool> {
        match self {
            Writer::Sql(s) => s.enable_text_index(),
            Writer::Kv(e) => e.enable_text_index(),
        }
    }

    fn rebuild_text_index(&mut self) -> Result<u64> {
        match self {
            Writer::Sql(s) => s.rebuild_text_index(),
            Writer::Kv(e) => e.rebuild_text_index(),
        }
    }

    fn statistics_due(&self) -> bool {
        match self {
            Writer::Sql(s) => s.statistics_due(),
            Writer::Kv(_) => false,
        }
    }

    fn defer_statistics(&mut self, deferred: bool) {
        if let Writer::Sql(s) = self {
            s.defer_statistics(deferred);
        }
    }

    /// One transaction whose body receives the backend-neutral handle.
    fn transact(
        &mut self,
        opts: TxOptions,
        f: impl FnOnce(&mut Tx<'_>) -> Result<()>,
    ) -> Result<TxReport> {
        match self {
            Writer::Sql(s) => s.transact(opts, |tx| f(&mut Tx::new(tx))),
            Writer::Kv(e) => e.transact(opts, |tx| f(&mut Tx::new(tx))),
        }
    }
}

/// The key-value side of a database: the backend handle for readers and the
/// readers' shared term cache.
pub(crate) struct Kv {
    pub(crate) store: Arc<dyn tm_store::Backend>,
    #[cfg(feature = "redb")]
    pub(crate) redb: Arc<tm_redb::RedbStore>,
    pub(crate) terms: TermCache,
}

/// An open database: one writer connection behind a mutex and, when the host
/// declares `reader_pool`, a pool of read-only connections on the same WAL file.
pub struct Db {
    id: u64,
    /// Declared before `writer` so the readers close first: the last connection
    /// to close checkpoints the WAL into the file and removes it only when it can
    /// write, so a closed database is one self-contained file (which the WASM
    /// host imports without its `-wal`).
    pool: Option<ReaderPool>,
    writer: Mutex<Writer>,
    /// The key-value backend (`None` on SQLite).
    kv: Option<Kv>,
    backend: Backend,
    /// Release generation of `writer`, so a bounded wait sleeps until a release
    /// instead of polling.
    released: Released,
    terms: TermReader,
    engine: Option<Arc<Engine>>,
    caps: Capabilities,
    #[cfg(feature = "exec")]
    path_max_hops: u32,
    /// `OpenOptions::reader_timeout`, also for reads served by the writer.
    reader_timeout: Option<Duration>,
    path: PathBuf,
    #[cfg_attr(not(all(feature = "sparql", feature = "cypher")), allow(dead_code))]
    clock: Arc<dyn Clock>,
    /// The bulk import session holding the write lease (0: none).
    import_lease: AtomicU64,
    #[cfg_attr(not(feature = "datalog"), allow(dead_code))]
    constraint_budget: tm_core::ConstraintBudget,
}

/// How often a bounded writer wait polls cancellation.
const POLL: Duration = Duration::from_millis(10);

/// A counter bumped on every writer release, with a condition variable to wait
/// for the next bump.
#[derive(Default)]
struct Released {
    generation: Mutex<u64>,
    signal: Condvar,
}

impl Released {
    fn generation(&self) -> u64 {
        *self.generation.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn bump(&self) {
        *self.generation.lock().unwrap_or_else(|p| p.into_inner()) += 1;
        self.signal.notify_all();
    }

    /// Waits until the generation passes `seen` or `wait` elapses.
    fn wait_past(&self, seen: u64, wait: Duration) {
        let g = self.generation.lock().unwrap_or_else(|p| p.into_inner());
        let _ = self
            .signal
            .wait_timeout_while(g, wait, |g| *g == seen)
            .unwrap_or_else(|p| p.into_inner());
    }
}

/// The locked writer; releasing it wakes bounded waiters.
pub(crate) struct WriterGuard<'a> {
    guard: Option<MutexGuard<'a, Writer>>,
    released: &'a Released,
}

impl<'a> WriterGuard<'a> {
    fn new(guard: MutexGuard<'a, Writer>, released: &'a Released) -> Self {
        WriterGuard {
            guard: Some(guard),
            released,
        }
    }
}

impl std::ops::Deref for WriterGuard<'_> {
    type Target = Writer;
    fn deref(&self) -> &Writer {
        self.guard.as_ref().expect("held until drop")
    }
}

impl std::ops::DerefMut for WriterGuard<'_> {
    fn deref_mut(&mut self) -> &mut Writer {
        self.guard.as_mut().expect("held until drop")
    }
}

impl Drop for WriterGuard<'_> {
    fn drop(&mut self) {
        // unlock first, then wake, so a woken waiter finds the mutex free
        drop(self.guard.take());
        self.released.bump();
    }
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
        let path = path.as_ref();
        match backend::resolve(path, opts.backend)? {
            Backend::Redb => Db::open_redb(path, opts, false),
            _ => Db::open_sqlite(RusqliteHost::new(), path, opts),
        }
    }

    /// Creates a new database at `path` on `backend` (`Auto` means SQLite).
    ///
    /// # Errors
    ///
    /// `AlreadyExists` when `path` exists, `BackendNotCompiled` for a backend
    /// this build lacks, and the errors of [`Db::open`].
    ///
    /// ```
    /// # use tiramemsu::*;
    /// # let dir = tempfile::tempdir().unwrap();
    /// let path = dir.path().join("new.db");
    /// let db = Db::create(&path, Backend::Sqlite, OpenOptions::default())?;
    /// assert_eq!(db.backend(), Backend::Sqlite);
    /// drop(db);
    /// assert!(matches!(Db::create(&path, Backend::Sqlite, OpenOptions::default()), Err(Error::AlreadyExists(_))));
    /// # Ok::<(), Error>(())
    /// ```
    // @tg: implements:: [[openspec:backend-selection#Creating a database]] {since: "2026-10-27"}
    pub fn create(path: impl AsRef<Path>, backend: Backend, opts: OpenOptions) -> Result<Db> {
        let path = path.as_ref();
        if path.exists() {
            return Err(Error::AlreadyExists(path.to_path_buf()));
        }
        match backend {
            Backend::Redb => Db::open_redb(path, opts, true),
            _ => Db::open_sqlite(RusqliteHost::new(), path, opts),
        }
    }

    /// The backend this database runs on.
    pub fn backend(&self) -> Backend {
        self.backend
    }

    #[cfg(feature = "redb")]
    fn open_redb(path: &Path, opts: OpenOptions, create: bool) -> Result<Db> {
        let ropts = tm_redb::RedbOptions {
            durability: match opts.durability {
                Durability::Normal => tm_redb::Durability::Normal,
                Durability::Immediate => tm_redb::Durability::Immediate,
                Durability::None => tm_redb::Durability::None,
            },
            flush_interval: opts.flush_interval,
            flush_commits: opts.flush_commits,
            ..Default::default()
        };
        let redb = Arc::new(if create || !path.exists() {
            tm_redb::RedbStore::create(path, ropts)?
        } else {
            tm_redb::RedbStore::open(path, ropts)?
        });
        let store: Arc<dyn tm_store::Backend> = redb.clone();
        let query_engine = build_engine(&opts);
        let mut engine = tm_store::Engine::open(
            store.clone(),
            tm_store::EngineOptions {
                clock: opts.clock.clone(),
                term_cache_capacity: opts.term_cache_capacity,
            },
        )?;
        if opts.text_index {
            engine.enable_text_index()?;
        }
        Ok(Db {
            id: NEXT_DB_ID.fetch_add(1, Ordering::Relaxed),
            writer: Mutex::new(Writer::Kv(engine)),
            kv: Some(Kv {
                store,
                redb,
                terms: TermCache::new(opts.term_cache_capacity),
            }),
            backend: Backend::Redb,
            released: Released::default(),
            pool: None,
            terms: TermReader::new(1),
            engine: query_engine,
            caps: Capabilities {
                fts5: true,
                ..Capabilities::default()
            },
            #[cfg(feature = "exec")]
            path_max_hops: opts.path_max_hops,
            reader_timeout: opts.reader_timeout,
            path: path.to_path_buf(),
            clock: opts.clock.clone(),
            import_lease: AtomicU64::new(0),
            constraint_budget: opts.constraint_budget,
        })
    }

    #[cfg(not(feature = "redb"))]
    fn open_redb(_path: &Path, _opts: OpenOptions, _create: bool) -> Result<Db> {
        Err(backend::not_compiled(Backend::Redb))
    }

    /// The key-value backend, if this database has one.
    pub(crate) fn kv(&self) -> Option<&Kv> {
        self.kv.as_ref()
    }

    /// The redb store behind a redb database (layout tests and `tm info`).
    #[doc(hidden)]
    #[cfg(feature = "redb")]
    pub fn redb_store(&self) -> Option<&Arc<tm_redb::RedbStore>> {
        self.kv.as_ref().map(|k| &k.redb)
    }

    /// The number of the last committed transaction (0 before the first).
    ///
    /// # Errors
    ///
    /// A storage error.
    pub fn last_t(&self) -> Result<u64> {
        match &self.kv {
            Some(k) => tm_store::read::last_t(&*k.store.read()?),
            None => self.read_committed(|e| tm_core::read::last_t(e)),
        }
    }

    /// What the file holds: backend, format version, statement, term and
    /// transaction counts and its size on disk (`tm info`).
    ///
    /// # Errors
    ///
    /// A storage error.
    pub fn summary(&self) -> Result<Summary> {
        let file_size = std::fs::metadata(&self.path).map_or(0, |m| m.len());
        let transactions = self.last_t()?;
        let (format_version, live_statements, statements, terms) = match &self.kv {
            Some(k) => {
                let s = k.store.read()?;
                let all = tm_store::Pattern::new(None, None, None);
                let terms = match s.term_count()? {
                    Some(n) => n,
                    None => return Err(Error::storage("the store cannot count its terms")),
                };
                (
                    s.meta("format_version")?,
                    s.count(all, tm_store::Scope::Live, u64::MAX)?,
                    s.count(all, tm_store::Scope::All, u64::MAX)?,
                    terms,
                )
            }
            None => {
                let rows = self.read_committed(|e| {
                    e.rows(
                        "SELECT (SELECT value FROM meta WHERE key = 'format_version'), \
                         (SELECT count(*) FROM triple WHERE t_ret IS NULL), \
                         (SELECT count(*) FROM triple), (SELECT count(*) FROM term)",
                        &[],
                    )
                })?;
                let n = |i: usize| rows[0][i].as_i64().unwrap_or(0).max(0) as u64;
                (rows[0][0].as_i64(), n(1), n(2), n(3))
            }
        };
        Ok(Summary {
            backend: self.backend(),
            format_version,
            live_statements,
            statements,
            terms,
            transactions,
            file_size,
        })
    }

    /// Makes every committed transaction durable: a durable commit on redb, a
    /// full WAL checkpoint on SQLite.
    ///
    /// # Errors
    ///
    /// `Reentrant` inside a running transaction, or a storage error.
    pub fn sync(&self) -> Result<()> {
        let _held = HeldGuard::acquire(self.id)?;
        #[cfg(feature = "redb")]
        if let Some(kv) = &self.kv {
            return kv.redb.sync();
        }
        let mut w = self.lock()?;
        w.sql()?
            .executor()
            .execute_batch("PRAGMA wal_checkpoint(FULL);")
    }

    /// Reclaims free space in a redb file. A no-op returning false on SQLite.
    ///
    /// # Errors
    ///
    /// `Storage` while another view or transaction is open.
    pub fn compact(&self) -> Result<bool> {
        let _held = HeldGuard::acquire(self.id)?;
        let _w = self.lock()?;
        #[cfg(feature = "redb")]
        if let Some(kv) = &self.kv {
            return kv.redb.compact();
        }
        Ok(false)
    }

    /// Opens the database at `path` through any executor [`Host`], for embedding on
    /// a SQLite other than the bundled one.
    ///
    /// # Errors
    ///
    /// As [`Db::open`], plus `MissingCapability` when `opts.query_engine` is set and
    /// the host lacks `functions` or `vtab`. Without the `exec` feature no engine
    /// is installed and only the host's required set is used.
    pub fn open_with_host(
        host: impl Host,
        path: impl AsRef<Path>,
        opts: OpenOptions,
    ) -> Result<Db> {
        Db::open_sqlite(host, path.as_ref(), opts)
    }

    fn open_sqlite(host: impl Host, path: &Path, opts: OpenOptions) -> Result<Db> {
        let engine = open_engine(&host, &opts)?;

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
        match opts.durability {
            Durability::Normal => {}
            Durability::Immediate => store
                .executor()
                .execute_batch("PRAGMA synchronous = FULL;")?,
            Durability::None => store
                .executor()
                .execute_batch("PRAGMA synchronous = OFF;")?,
        }
        if opts.text_index && caps.fts5 {
            store.enable_text_index()?;
        }
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
            Some(ReaderPool::new(readers, opts.reader_timeout))
        } else {
            None
        };
        Ok(Db {
            id: NEXT_DB_ID.fetch_add(1, Ordering::Relaxed),
            writer: Mutex::new(Writer::Sql(store)),
            kv: None,
            backend: Backend::Sqlite,
            released: Released::default(),
            pool,
            terms: TermReader::new(opts.term_cache_capacity),
            engine,
            caps,
            #[cfg(feature = "exec")]
            path_max_hops: opts.path_max_hops,
            reader_timeout: opts.reader_timeout,
            path: path.to_path_buf(),
            clock: opts.clock.clone(),
            import_lease: AtomicU64::new(0),
            constraint_budget: opts.constraint_budget,
        })
    }

    /// The database clock (`NOW()` in SPARQL).
    #[cfg_attr(not(all(feature = "sparql", feature = "cypher")), allow(dead_code))]
    pub(crate) fn now_ms(&self) -> i64 {
        self.clock.now_ms()
    }

    /// The capabilities declared by the host, for crates that need one.
    pub fn capabilities(&self) -> Capabilities {
        self.caps
    }

    /// The hop cap of unbounded Cypher path patterns (`OpenOptions::path_max_hops`).
    #[cfg(feature = "exec")]
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

    fn lock(&self) -> Result<WriterGuard<'_>> {
        let g = self.writer.lock().unwrap_or_else(|p| p.into_inner());
        Ok(WriterGuard::new(g, &self.released))
    }

    /// Locks the writer. Under an operation budget the wait ends with the budget's
    /// stop conditions and, for a read served by the writer, with the reader
    /// timeout (`PoolTimeout`); otherwise it is the plain blocking lock. The bounded
    /// wait sleeps on the release signal and wakes at least every 10 ms to poll
    /// cancellation, like the reader pool.
    fn lock_bounded(&self, read: bool) -> Result<WriterGuard<'_>> {
        let interrupt = budget::interrupt();
        let timeout = if read {
            budget::reader_timeout().or(self.reader_timeout)
        } else {
            None
        };
        if interrupt.is_none() && timeout.is_none() {
            return self.lock();
        }
        let start = Instant::now();
        loop {
            let seen = self.released.generation();
            match self.writer.try_lock() {
                Ok(g) => return Ok(WriterGuard::new(g, &self.released)),
                Err(TryLockError::Poisoned(p)) => {
                    return Ok(WriterGuard::new(p.into_inner(), &self.released))
                }
                Err(TryLockError::WouldBlock) => {}
            }
            if let Some(i) = &interrupt {
                i.check()?;
            }
            let mut wait = POLL;
            if let Some(timeout) = timeout {
                let elapsed = start.elapsed();
                if elapsed >= timeout {
                    return Err(Error::PoolTimeout { timeout });
                }
                wait = wait.min(timeout - elapsed);
            }
            self.released.wait_past(seen, wait);
        }
    }

    pub(crate) fn term_reader(&self) -> &TermReader {
        &self.terms
    }

    #[cfg_attr(not(all(feature = "sparql", feature = "cypher")), allow(dead_code))]
    pub(crate) fn engine(&self) -> Option<&Engine> {
        self.engine.as_deref()
    }

    /// Number of terms in the query engine's shared term cache (tests).
    #[doc(hidden)]
    #[cfg(feature = "exec")]
    pub fn term_cache_len(&self) -> usize {
        self.engine.as_ref().map_or(0, |e| e.term_cache().len())
    }

    /// True when the query engine's shared term cache holds `id` (tests).
    #[doc(hidden)]
    #[cfg(feature = "exec")]
    pub fn term_cache_contains(&self, id: tm_core::ObjectId) -> bool {
        self.engine
            .as_ref()
            .is_some_and(|e| e.term_cache().contains(id))
    }

    /// Sets a hook run by every query between planning and its SQL statement
    /// (snapshot tests).
    #[doc(hidden)]
    #[cfg(feature = "exec")]
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
                armed(&mut *self.lock_bounded(true)?, |store| store.sql()?.read(f))
            }
        }
    }

    /// Runs a full `ANALYZE` on the writer. Worth calling once after a large bulk
    /// load; normal operation runs `PRAGMA optimize` on its own (`optimize_every`).
    /// A [`BulkImport`](crate::BulkImport) session does this once when finished.
    ///
    /// # Errors
    ///
    /// `Reentrant` inside a running transaction, or `Sqlite`.
    pub fn optimize(&self) -> Result<()> {
        let _held = HeldGuard::acquire(self.id)?;
        self.lock()?.optimize()
    }

    /// Builds the derived text index unless it is current, and indexes what a
    /// host without FTS5 wrote since it last was (the `OpenOptions::text_index`
    /// step, for a database that is already open). Returns true when it built the
    /// whole index. Touches no graph row and consumes no transaction number.
    ///
    /// # Errors
    ///
    /// `MissingCapability("fts5")` on a host without FTS5, `ImportInProgress`
    /// while a bulk import session holds the write lease, `Reentrant` inside a
    /// running transaction.
    pub fn enable_text_index(&self) -> Result<bool> {
        let _held = HeldGuard::acquire(self.id)?;
        if self.import_active() {
            return Err(Error::ImportInProgress);
        }
        self.lock()?.enable_text_index()
    }

    /// Drops and rebuilds the derived text index from every statement, live and
    /// retracted, in one write transaction, and returns the number of indexed
    /// string values. History is untouched: no `triple`, `term` or `tx` row is
    /// changed or added, and recall afterwards returns what it returned before.
    /// Use it to repair or upgrade the index; it also builds a missing one.
    ///
    /// # Errors
    ///
    /// As [`Db::enable_text_index`].
    ///
    /// ```
    /// # use tiramemsu::*;
    /// # let dir = tempfile::tempdir().unwrap();
    /// let db = Db::open(dir.path().join("m.db"), OpenOptions::default())?;
    /// let v = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
    /// db.transact(TxOptions::default(), |tx| {
    ///     tx.assert(v("alice"), v("note"), Value::str("met at the Lisbon offsite"), Valid::ALWAYS)?;
    ///     Ok(())
    /// })?;
    /// assert_eq!(db.rebuild_text_index()?, 1);
    /// let hits = db.now().text_search(&TextQuery::new("lisbon"))?;
    /// assert_eq!(hits[0].text, "met at the Lisbon offsite");
    /// # Ok::<(), Error>(())
    /// ```
    // @lat: [[storage#Text Index]]
    pub fn rebuild_text_index(&self) -> Result<u64> {
        let _held = HeldGuard::acquire(self.id)?;
        if self.import_active() {
            return Err(Error::ImportInProgress);
        }
        self.lock()?.rebuild_text_index()
    }

    /// Runs `f` on the writer in one write transaction for derived records (saved
    /// answers): no transaction number, no event. Refused while a bulk import
    /// holds the write lease and inside a running transaction.
    #[cfg_attr(not(all(feature = "sparql", feature = "cypher")), allow(dead_code))]
    pub(crate) fn derived_write<R>(
        &self,
        f: impl FnOnce(&mut dyn Executor) -> Result<R>,
    ) -> Result<R> {
        let _held = HeldGuard::acquire(self.id)?;
        if self.import_active() {
            return Err(Error::ImportInProgress);
        }
        armed(&mut *self.lock_bounded(false)?, |store| {
            store.sql()?.derived_write(f)
        })
    }

    /// True when commits of a bulk import session changed the data and the
    /// planner statistics have not been refreshed since (the session was cancelled
    /// or dropped, or its final analysis failed). The next ordinary commit or
    /// [`Db::optimize`] refreshes them; stale statistics change plans, never results.
    ///
    /// # Errors
    ///
    /// `Reentrant` inside a running transaction.
    pub fn statistics_due(&self) -> Result<bool> {
        let _held = HeldGuard::acquire(self.id)?;
        Ok(self.lock()?.statistics_due())
    }

    /// True while a [`BulkImport`](crate::BulkImport) session holds the write lease.
    pub fn import_active(&self) -> bool {
        self.import_lease.load(Ordering::Acquire) != 0
    }

    /// Takes the write lease for import session `lease`.
    pub(crate) fn acquire_import_lease(&self, lease: u64) -> Result<()> {
        if HeldGuard::is_held(self.id) {
            return Err(Error::Reentrant);
        }
        self.import_lease
            .compare_exchange(0, lease, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(|_| Error::ImportInProgress)
    }

    /// Gives the write lease of session `lease` back, without touching the writer.
    pub(crate) fn release_import_lease(&self, lease: u64) {
        let _ = self
            .import_lease
            .compare_exchange(lease, 0, Ordering::AcqRel, Ordering::Acquire);
    }

    /// Ends session `lease`: one full analysis with the reader refresh, then the
    /// lease is released whatever the analysis returned.
    pub(crate) fn finish_import(&self, lease: u64) -> Result<()> {
        let r = (|| {
            let _held = HeldGuard::acquire(self.id)?;
            let mut store = self.lock()?;
            store.defer_statistics(false);
            store.optimize()
        })();
        self.release_import_lease(lease);
        r
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
        self.transact_leased(opts, None, f)
    }

    /// [`Db::transact`] on behalf of import session `lease` (`None`: an ordinary
    /// write). The lease is checked under the writer lock: while a session holds
    /// it, only that session's chunks write, and their commits defer statistics.
    pub(crate) fn transact_leased<F>(
        &self,
        opts: TxOptions,
        lease: Option<u64>,
        f: F,
    ) -> Result<TxReport>
    where
        F: FnOnce(&mut Tx<'_>) -> Result<()>,
    {
        let _held = HeldGuard::acquire(self.id)?;
        #[cfg(feature = "exec")]
        let engine = self.engine.clone();
        let mut guard = self.lock_bounded(false)?;
        let holder = self.import_lease.load(Ordering::Acquire);
        if holder != 0 && Some(holder) != lease {
            return Err(Error::ImportInProgress);
        }
        if lease.is_some() && holder == 0 {
            // the session was released (cannot happen through `BulkImport`)
            return Err(Error::ImportInProgress);
        }
        guard.defer_statistics(lease.is_some());
        #[cfg(feature = "datalog")]
        let notes: std::cell::RefCell<Option<crate::datalog_enforce::Notes>> = Default::default();
        #[cfg(feature = "datalog")]
        let (notes_ref, limit) = (
            &notes,
            opts.constraint_budget.unwrap_or(self.constraint_budget),
        );
        #[cfg(feature = "elpi")]
        let elpi_notes: std::cell::RefCell<Option<tm_elpi_store::Notes>> = Default::default();
        #[cfg(feature = "elpi")]
        let (elpi_ref, elpi_limit) = (
            &elpi_notes,
            opts.constraint_budget.unwrap_or(self.constraint_budget),
        );
        let report = armed(&mut guard, |store| {
            store.transact(opts, move |tx| {
                #[cfg(feature = "exec")]
                if let Some(e) = engine {
                    tx.set_extension(e);
                }
                f(tx)?;
                // Datalog constraints of enforced programs (add-datalog-constraints)
                #[cfg(feature = "datalog")]
                {
                    let n = crate::datalog_enforce::commit_check(tx, opts.dry_run, limit)?;
                    *notes_ref.borrow_mut() = Some(n);
                }
                // ELPI constraints of enforced programs (add-elpi-programs)
                #[cfg(feature = "elpi")]
                {
                    let n = tm_elpi_store::commit_check(tx, opts.dry_run, elpi_limit)?;
                    *elpi_ref.borrow_mut() = Some(n);
                }
                if budget::active() {
                    // the last point a budget stops the write; past it the
                    // bookkeeping and COMMIT run without the interrupt
                    budget::check()?;
                    if !tx.on_store() {
                        tx.read_with(|e| {
                            e.set_interrupt(None);
                            Ok(())
                        })?;
                    }
                }
                Ok(())
            })
        });
        #[cfg(feature = "datalog")]
        let report = report.map(|mut r| {
            if let Some(n) = notes.take() {
                r.constraint_checks = n.checks;
                r.seed_rows = n.seed_rows;
                r.constraint_violations = n.violations;
            }
            r
        });
        #[cfg(feature = "elpi")]
        let report = report.map(|mut r| {
            if let Some(n) = elpi_notes.take() {
                r.constraint_checks += n.checks;
                r.seed_rows += n.seed_rows;
                r.constraint_violations.extend(n.violations);
            }
            r
        });
        report
    }

    /// [`Db::transact`] as one operation bounded by `budget`. The deadline and the
    /// cancellation token cover waiting for the writer and every statement of the
    /// body, and are checked once more when the body returns; a stopped transaction
    /// rolls back and leaves no trace. The row and byte budgets cover the queries
    /// the body runs (Cypher through `TxCypher`, feature `cypher`).
    ///
    /// # Errors
    ///
    /// Those of [`Db::transact`], plus `Cancelled`, `DeadlineExceeded` and
    /// `ResultLimitExceeded`.
    ///
    /// ```
    /// # use tiramemsu::*;
    /// # use std::time::Duration;
    /// # let dir = tempfile::tempdir().unwrap();
    /// # let db = Db::open(dir.path().join("m.db"), OpenOptions::default())?;
    /// let v = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
    /// let budget = QueryBudget { timeout: Some(Duration::from_millis(20)), ..Default::default() };
    /// let r = db.transact_budgeted(TxOptions::default(), &budget, |tx| {
    ///     tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?;
    ///     std::thread::sleep(Duration::from_millis(40)); // past the deadline
    ///     Ok(())
    /// });
    /// assert!(matches!(r, Err(Error::DeadlineExceeded { .. })));
    /// assert!(db.now().triples(None, None, None)?.is_empty()); // rolled back
    /// # Ok::<(), Error>(())
    /// ```
    // @lat: [[query#Query Budgets]]
    pub fn transact_budgeted<F>(
        &self,
        opts: TxOptions,
        budget: &QueryBudget,
        f: F,
    ) -> Result<TxReport>
    where
        F: FnOnce(&mut Tx<'_>) -> Result<()>,
    {
        crate::budget::run(Some(budget), || self.transact(opts, f))
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
        let mut w = self.lock()?;
        match &mut *w {
            Writer::Sql(store) => store.speculate(
                |tx| {
                    let mut tx = Tx::new(tx);
                    ops(&mut tx)?;
                    #[cfg(feature = "datalog")]
                    crate::datalog_enforce::commit_check(&mut tx, false, self.constraint_budget)?;
                    #[cfg(feature = "elpi")]
                    tm_elpi_store::commit_check(&mut tx, false, self.constraint_budget)?;
                    Ok(())
                },
                |exec| {
                    let cell: RefCell<&mut dyn Executor> = RefCell::new(exec);
                    let view = View::on_writer(&cell, ViewSpec::NOW, engine);
                    query(&view)
                },
            ),
            Writer::Kv(e) => e.speculate(
                |tx| {
                    let mut tx = Tx::new(tx);
                    ops(&mut tx)?;
                    #[cfg(feature = "datalog")]
                    crate::datalog_enforce::commit_check(&mut tx, false, self.constraint_budget)?;
                    #[cfg(feature = "elpi")]
                    tm_elpi_store::commit_check(&mut tx, false, self.constraint_budget)?;
                    Ok(())
                },
                |store| {
                    let view = View::on_store(store, ViewSpec::NOW, engine);
                    query(&view)
                },
            ),
        }
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

/// Runs `f` on the writer with the current operation's stop conditions installed on
/// its connection (when there are any), removing them afterwards, also when `f`
/// panics.
fn armed<R>(store: &mut Writer, f: impl FnOnce(&mut Writer) -> Result<R>) -> Result<R> {
    let Some(interrupt) = budget::interrupt() else {
        return f(store);
    };
    let Writer::Sql(sql) = store else {
        // the storage-trait engine has no statement to interrupt; budgets stop
        // it between operations
        return f(store);
    };
    sql.executor().set_interrupt(Some(interrupt));
    let r = catch_unwind(AssertUnwindSafe(|| f(&mut *store)));
    if let Writer::Sql(sql) = store {
        sql.executor().set_interrupt(None);
    }
    match r {
        Ok(r) => r,
        Err(panic) => resume_unwind(panic),
    }
}

/// The query engine of `opts`: `None` when `opts.query_engine` is off. Refused
/// with `MissingCapability` before the file is touched on a host without
/// `functions` and `vtab`.
#[cfg(feature = "exec")]
fn open_engine(host: &impl Host, opts: &OpenOptions) -> Result<Option<Arc<Engine>>> {
    if !opts.query_engine {
        return Ok(None);
    }
    // refuse before touching the file: no query is ever planned on a host
    // without `functions` and `vtab` (decision D22)
    tm_exec::host::check_capabilities(host.capabilities())?;
    Ok(build_engine(opts))
}

/// The query engine of `opts` (`None` when `opts.query_engine` is off). On a
/// key-value backend it prepares queries and the native runner executes them.
#[cfg(feature = "exec")]
fn build_engine(opts: &OpenOptions) -> Option<Arc<Engine>> {
    if !opts.query_engine {
        return None;
    }
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
    // the cyclic-join operator is installed only when LFTJ routing is on
    if opts.planner.lftj.enabled {
        reg.add(Arc::new(LftjOperator));
    }
    for op in &opts.native_operators {
        reg.add(op.clone());
    }
    let engine = Engine::new(opts.planner, reg, opts.term_cache_capacity)
        .with_parallel_threshold(opts.parallel_threshold);
    // goal calls from SPARQL and Cypher (`add-datalog-query-calls`)
    #[cfg(feature = "datalog")]
    let engine = engine.with_datalog_provider(Arc::new(crate::datalog_call::Provider {
        clock: opts.clock.clone(),
    }));
    // ELPI goal calls (`add-elpi-query-calls`)
    #[cfg(feature = "elpi")]
    let engine = engine.with_elpi_provider(Arc::new(crate::elpi_call::Provider {
        clock: opts.clock.clone(),
    }));
    Some(Arc::new(engine))
}

/// Without the `exec` feature a redb database has no query engine.
#[cfg(all(not(feature = "exec"), feature = "redb"))]
fn build_engine(_opts: &OpenOptions) -> Option<Arc<Engine>> {
    None
}

/// Without the `exec` feature no engine is ever installed.
#[cfg(not(feature = "exec"))]
fn open_engine(_host: &impl Host, _opts: &OpenOptions) -> Result<Option<Arc<Engine>>> {
    Ok(None)
}
