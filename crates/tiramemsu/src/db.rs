//! The database handle: one writer behind a mutex, a pool of readers.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tm_core::{
    storage, Capabilities, Clock, Error, Event, Executor, Host, HostOptions, Result, Store,
    StoreOptions, SystemClock, TermReader, TimeRef, Tx, TxOptions, TxReport, ViewSpec,
};
use tm_rusqlite::RusqliteHost;

use crate::pool::ReaderPool;
use crate::view::View;

/// Per-database tuning knobs (`lat.md/api#Open Options`).
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
}

impl Default for OpenOptions {
    fn default() -> Self {
        OpenOptions {
            readers: 4,
            clock: Arc::new(SystemClock),
            busy_timeout: Duration::from_secs(5),
            term_cache_capacity: 16_384,
            optimize_every: 1000,
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
    caps: Capabilities,
    path: PathBuf,
}

impl std::fmt::Debug for Db {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Db").field("path", &self.path).finish()
    }
}

impl Db {
    /// Opens (creating or migrating) the database at `path` with the `rusqlite` host.
    pub fn open(path: impl AsRef<Path>, opts: OpenOptions) -> Result<Db> {
        Db::open_with_host(RusqliteHost::new(), path, opts)
    }

    /// Opens the database at `path` through any executor host.
    pub fn open_with_host(
        host: impl Host,
        path: impl AsRef<Path>,
        opts: OpenOptions,
    ) -> Result<Db> {
        let path = path.as_ref();
        let store = Store::open(
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
        let pool = if caps.reader_pool && opts.readers > 0 {
            let hopts = HostOptions {
                busy_timeout: opts.busy_timeout,
            };
            let mut readers = Vec::with_capacity(opts.readers);
            for _ in 0..opts.readers {
                let mut r = host.open_reader(path, &hopts)?;
                storage::configure_reader(r.as_mut())?;
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
            caps,
            path: path.to_path_buf(),
        })
    }

    /// The capabilities declared by the host, for crates that need one.
    pub fn capabilities(&self) -> Capabilities {
        self.caps
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

    /// Runs a full `ANALYZE` on the writer.
    pub fn optimize(&self) -> Result<()> {
        let _held = HeldGuard::acquire(self.id)?;
        self.lock()?.optimize()
    }

    /// Runs one transaction on the single writer and returns its report.
    ///
    /// If the body or any operation fails, the whole transaction is rolled back and
    /// leaves no trace: no `tx` row, statement, term, volatile change or counter
    /// change. Ids seen inside a failed body may therefore be reissued; do not
    /// persist them. With `opts.dry_run`, every operation runs with full semantics,
    /// the report is returned and all effects are discarded (allocated ids are
    /// burned). Starting a write from inside another write or speculation on the
    /// same database fails with [`Error::Reentrant`].
    pub fn transact<F>(&self, opts: TxOptions, f: F) -> Result<TxReport>
    where
        F: FnOnce(&mut Tx<'_>) -> Result<()>,
    {
        let _held = HeldGuard::acquire(self.id)?;
        self.lock()?.transact(opts, f)
    }

    /// Speculation: applies `ops` hypothetically on the single writer, calls `query`
    /// with a now view that sees the uncommitted state, then discards everything.
    /// No transaction number is consumed and no event is logged; ids allocated
    /// inside are burned so they are never reissued. `query` is not called when
    /// `ops` fails.
    pub fn with<F, G, R>(&self, ops: F, query: G) -> Result<R>
    where
        F: FnOnce(&mut Tx<'_>) -> Result<()>,
        G: FnOnce(&View<'_>) -> Result<R>,
    {
        let _held = HeldGuard::acquire(self.id)?;
        self.lock()?.speculate(ops, |exec| {
            let cell: RefCell<&mut dyn Executor> = RefCell::new(exec);
            let view = View::on_writer(&cell, ViewSpec::NOW);
            query(&view)
        })
    }

    /// The now view (live statements, valid time unfiltered).
    pub fn now(&self) -> View<'_> {
        View::on_db(self, ViewSpec::NOW)
    }

    /// The as-of view at a transaction number or a wall-clock instant.
    pub fn as_of(&self, at: TimeRef) -> View<'_> {
        View::on_db(self, ViewSpec::as_of(at))
    }

    /// The history view (every statement ever committed, with its lifetime).
    pub fn history(&self) -> View<'_> {
        View::on_db(self, ViewSpec::history())
    }

    /// Every event with `t > since`, ordered by time, asserts before retracts, eid.
    pub fn events_since(&self, since: u64) -> Result<Vec<Event>> {
        self.now().events_since(since)
    }

    /// Runs raw SQL on the read path (diagnostics and tests).
    #[doc(hidden)]
    pub fn read_sql(&self, sql: &str) -> Result<Vec<Vec<tm_core::SqlValue>>> {
        self.read_committed(|e| e.rows(sql, &[]))
    }
}
