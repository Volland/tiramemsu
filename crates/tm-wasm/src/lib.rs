#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

mod probe;
mod storage;

use std::path::Path;

use rusqlite::{Connection, OpenFlags};
use tm_core::{
    Capabilities, Clock, Error, Executor, Host, HostOptions, HostRegistry, Interrupt, Result,
    SqlValue,
};
use tm_rusqlite::{map_err, RusqliteExec};

pub use probe::{probe_capabilities, runtime_info, RuntimeInfo};
#[cfg(all(target_family = "wasm", target_os = "unknown"))]
pub use storage::{install_opfs, OpfsOptions};
pub use storage::{Journal, Storage};

/// The WASM SQLite host: opens [`WasmExec`] connections on one storage with one
/// journal policy.
///
/// The capabilities are probed from the running SQLite when the host is built
/// ([`probe_capabilities`]): `functions`, `vtab` and `fts5` are declared only if
/// registering and running them works, `stat4` only if `PRAGMA compile_options`
/// lists `ENABLE_STAT4`. `reader_pool` is never declared: the WASM VFSes have no
/// shared memory (so no WAL snapshots beside a writer) and the runtime's SQLite is
/// single-threaded, so the writer serves every read.
///
/// Opening verifies the journal mode with the running SQLite and fails with
/// `MissingCapability` instead of continuing in another mode ([`Journal`]).
///
/// ```
/// use tm_core::Host;
/// use tm_wasm::{Journal, Storage, WasmHost};
///
/// let host = WasmHost::new(Storage::default_for_target(), Journal::Rollback);
/// assert!(!host.capabilities().reader_pool);
/// assert!(host.capabilities().functions && host.capabilities().vtab);
/// ```
// @lat: [[architecture#WebAssembly Host]]
#[derive(Clone, Debug)]
pub struct WasmHost {
    storage: Storage,
    journal: Journal,
    caps: Capabilities,
}

impl WasmHost {
    /// A host on `storage` with `journal`, declaring the probed capabilities.
    pub fn new(storage: Storage, journal: Journal) -> WasmHost {
        WasmHost {
            storage,
            journal,
            caps: probe_capabilities(),
        }
    }

    /// Declares at most `allowed`: each capability stays only if it was probed and
    /// is set in `allowed`. Use it to run the `tm-core` tier on purpose (opening a
    /// query engine then fails with `MissingCapability`). It never adds a
    /// capability the runtime lacks.
    pub fn limit_capabilities(mut self, allowed: Capabilities) -> WasmHost {
        let c = self.caps;
        self.caps = Capabilities {
            reader_pool: c.reader_pool && allowed.reader_pool,
            functions: c.functions && allowed.functions,
            vtab: c.vtab && allowed.vtab,
            stat4: c.stat4 && allowed.stat4,
            fts5: c.fts5 && allowed.fts5,
        };
        self
    }

    /// The storage this host opens.
    pub fn storage(&self) -> &Storage {
        &self.storage
    }

    /// The journal policy this host enforces.
    pub fn journal(&self) -> Journal {
        self.journal
    }

    /// The committed bytes of the database file `path` on this host's storage, to
    /// download or to open with another host. Call it between transactions; a
    /// rollback-journal file holds exactly its committed state then.
    ///
    /// # Errors
    ///
    /// `Sqlite` (I/O) when the file is missing, `Unsupported` when the storage is
    /// not available on this target.
    pub fn export_file(&self, path: &str) -> Result<Vec<u8>> {
        storage::export(&self.storage, path)
    }

    /// Stores `bytes` (a SQLite database file, for example one written by the
    /// native host) as `path` on this host's storage, ready to open. The memory
    /// and OPFS VFSes rewrite a WAL-mode header to rollback mode, as they have no
    /// WAL; the pages are unchanged.
    ///
    /// # Errors
    ///
    /// `Sqlite` (I/O) when `path` exists or `bytes` is not a database file,
    /// `Unsupported` when the storage is not available on this target.
    pub fn import_file(&self, path: &str, bytes: &[u8]) -> Result<()> {
        storage::import(&self.storage, path, bytes)
    }

    fn open(&self, path: &Path, opts: &HostOptions) -> Result<WasmExec> {
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_URI;
        let conn = match storage::vfs_name(&self.storage)? {
            Some(vfs) => Connection::open_with_flags_and_vfs(path, flags, vfs.as_str()),
            None => Connection::open_with_flags(path, flags),
        }
        .map_err(map_err)?;
        conn.busy_timeout(opts.busy_timeout).map_err(map_err)?;
        conn.set_prepared_statement_cache_capacity(256);
        if self.caps.vtab {
            tm_rusqlite::register::register_defaults(&conn).map_err(map_err)?;
        }
        storage::apply_journal(&conn, &self.storage, self.journal)?;
        Ok(WasmExec {
            inner: RusqliteExec::from_connection(conn, self.caps),
            journal: self.journal,
        })
    }
}

impl Host for WasmHost {
    fn capabilities(&self) -> Capabilities {
        self.caps
    }

    fn open_writer(&self, path: &Path, opts: &HostOptions) -> Result<Box<dyn Executor>> {
        Ok(Box::new(self.open(path, opts)?))
    }

    fn open_reader(&self, _path: &Path, _opts: &HostOptions) -> Result<Box<dyn Executor>> {
        Err(Error::MissingCapability {
            capability: "reader_pool (the WASM host serves reads on the writer)".to_string(),
        })
    }
}

/// One connection of the WASM host.
///
/// Statements, transactions, savepoints, interrupts and registration run on the
/// `rusqlite` executor of `tm-rusqlite`, which compiles unchanged for
/// `wasm32-unknown-unknown`. This wrapper keeps the journal mode verified at open:
/// the engine's `PRAGMA journal_mode = WAL` is checked against the runtime under
/// [`Journal::Wal`] and left out under the explicit [`Journal::Rollback`].
pub struct WasmExec {
    inner: RusqliteExec,
    journal: Journal,
}

impl std::fmt::Debug for WasmExec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmExec")
            .field("journal", &self.journal)
            .finish()
    }
}

impl WasmExec {
    /// The underlying connection.
    pub fn connection(&self) -> &Connection {
        self.inner.connection()
    }
}

/// True for the engine's switch to WAL (`tm_core::storage::open`).
fn is_wal_switch(sql: &str) -> bool {
    let compact: String = sql
        .chars()
        .filter(|c| !c.is_whitespace() && *c != ';')
        .collect();
    compact.eq_ignore_ascii_case("PRAGMAjournal_mode=WAL")
}

impl Executor for WasmExec {
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }

    fn execute(&mut self, sql: &str, params: &[SqlValue]) -> Result<usize> {
        self.inner.execute(sql, params)
    }

    fn query(
        &mut self,
        sql: &str,
        params: &[SqlValue],
        row: &mut dyn FnMut(&[SqlValue]) -> Result<()>,
    ) -> Result<()> {
        self.inner.query(sql, params, row)
    }

    fn execute_batch(&mut self, sql: &str) -> Result<()> {
        if is_wal_switch(sql) {
            return match self.journal {
                // verified at open; check again that the runtime still reports it
                Journal::Wal => {
                    storage::expect_journal(self.inner.connection(), "wal", "the open database")
                }
                // the caller chose a rollback journal explicitly (same file format)
                Journal::Rollback => Ok(()),
            };
        }
        self.inner.execute_batch(sql)
    }

    fn begin_immediate(&mut self) -> Result<()> {
        self.inner.begin_immediate()
    }

    fn begin_read(&mut self) -> Result<()> {
        self.inner.begin_read()
    }

    fn commit(&mut self) -> Result<()> {
        self.inner.commit()
    }

    fn rollback(&mut self) -> Result<()> {
        self.inner.rollback()
    }

    fn savepoint(&mut self, name: &str) -> Result<()> {
        self.inner.savepoint(name)
    }

    fn rollback_to(&mut self, name: &str) -> Result<()> {
        self.inner.rollback_to(name)
    }

    fn release(&mut self, name: &str) -> Result<()> {
        self.inner.release(name)
    }

    fn set_interrupt(&mut self, interrupt: Option<Interrupt>) {
        self.inner.set_interrupt(interrupt);
    }

    fn registry(&mut self) -> Option<&mut dyn HostRegistry> {
        self.inner.registry()
    }
}

/// The wall clock of the WASM host: `Date.now()` on `wasm32-unknown-unknown`,
/// where `std::time::SystemTime` is unavailable, and the system clock elsewhere.
/// Pass it as `OpenOptions::clock` (or `StoreOptions::clock`) in a browser.
#[derive(Copy, Clone, Debug, Default)]
pub struct WasmClock;

impl Clock for WasmClock {
    fn now_ms(&self) -> i64 {
        #[cfg(all(target_family = "wasm", target_os = "unknown"))]
        {
            js_sys::Date::now() as i64
        }
        #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
        {
            tm_core::SystemClock.now_ms()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_the_engine_wal_switch() {
        assert!(is_wal_switch("PRAGMA journal_mode = WAL"));
        assert!(is_wal_switch("pragma journal_mode=wal;"));
        assert!(!is_wal_switch("PRAGMA journal_mode = DELETE"));
        assert!(!is_wal_switch("PRAGMA synchronous = NORMAL"));
    }
}
