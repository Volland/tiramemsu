//! Storage (which SQLite VFS holds the file) and the journal policy.

use rusqlite::Connection;
use tm_core::{Error, Result, SqlError};

/// `SQLITE_IOERR`, the code of a failed VFS import or export.
const IOERR: i32 = 10;

fn io(msg: impl Into<String>) -> Error {
    Error::Sqlite(SqlError::new(IOERR, msg))
}

/// Where the WASM host keeps database files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Storage {
    /// The memory VFS of `sqlite-wasm-rs` (`memvfs`): files live in the WASM
    /// instance and vanish with it. Any context (window, worker, Node.js);
    /// `wasm32-unknown-unknown` only.
    Memory,
    /// The Origin Private File System through the sync-access-handle pool VFS
    /// (`opfs-sahpool`): durable, dedicated Web Worker only, after
    /// `install_opfs` (both `wasm32-unknown-unknown` only).
    Opfs,
    /// The platform's file VFS: native targets only. It runs this host's journal
    /// policy and probe against real files, which is how the host is tested
    /// beside `tm-rusqlite` (and how a crash mid-write is simulated).
    File,
}

impl Storage {
    /// The storage name used in errors (`memory`, `opfs`, `file`).
    pub fn name(&self) -> &'static str {
        match self {
            Storage::Memory => "memory",
            Storage::Opfs => "opfs",
            Storage::File => "file",
        }
    }

    /// [`Storage::Memory`] on `wasm32-unknown-unknown`, [`Storage::File`] elsewhere.
    pub fn default_for_target() -> Storage {
        if cfg!(all(target_family = "wasm", target_os = "unknown")) {
            Storage::Memory
        } else {
            Storage::File
        }
    }
}

/// The journal mode the host must obtain from the runtime, verified at open.
///
/// The format contract of the native host is WAL. The WASM VFSes have no shared
/// memory, so the runtime keeps a rollback journal: under `Wal` opening such a
/// storage fails with `MissingCapability` naming the mode the runtime kept.
/// `Rollback` is the explicit opt-in to a rollback (`DELETE`) journal: the
/// database file format is the same, a hot journal is rolled back on the next
/// open, and a native host switches the file back to WAL when it opens it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Journal {
    /// `journal_mode = WAL`, as the native host; fails where the runtime cannot.
    Wal,
    /// `journal_mode = DELETE`, a rollback journal (no concurrent readers).
    Rollback,
}

impl Journal {
    fn mode(self) -> &'static str {
        match self {
            Journal::Wal => "wal",
            Journal::Rollback => "delete",
        }
    }
}

/// Sets the journal mode of `journal` on `conn` and checks what the runtime kept.
pub(crate) fn apply_journal(conn: &Connection, storage: &Storage, journal: Journal) -> Result<()> {
    let want = journal.mode();
    let got: String = conn
        .query_row(&format!("PRAGMA journal_mode = {want}"), [], |r| r.get(0))
        .map_err(tm_rusqlite::map_err)?;
    check(&got, want, &format!("{} storage", storage.name()))
}

/// Checks that `conn` reports journal mode `want`.
pub(crate) fn expect_journal(conn: &Connection, want: &str, what: &str) -> Result<()> {
    let got: String = conn
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .map_err(tm_rusqlite::map_err)?;
    check(&got, want, what)
}

fn check(got: &str, want: &str, what: &str) -> Result<()> {
    if got.eq_ignore_ascii_case(want) {
        return Ok(());
    }
    let hint = if want == "wal" {
        "; open with Journal::Rollback to accept a rollback journal"
    } else {
        ""
    };
    Err(Error::MissingCapability {
        capability: format!(
            "journal_mode={want} on {what} (the runtime kept journal_mode={got}{hint})"
        ),
    })
}

#[cfg(all(target_family = "wasm", target_os = "unknown"))]
mod wasm {
    use super::*;
    use sqlite_wasm_rs::{MemVfsUtil, WasmOsCallback};
    use sqlite_wasm_vfs::sahpool::{self, OpfsSAHPoolCfg, OpfsSAHPoolUtil};
    use std::cell::RefCell;

    /// The VFS name of the memory storage.
    pub(super) const MEMVFS: &str = "memvfs";

    thread_local! {
        static MEM: MemVfsUtil<WasmOsCallback> = MemVfsUtil::new();
        static OPFS: RefCell<Option<(String, OpfsSAHPoolUtil)>> = const { RefCell::new(None) };
    }

    /// Options of the OPFS storage.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct OpfsOptions {
        /// The OPFS directory holding the pool (default `.tiramemsu`).
        pub directory: String,
        /// The number of files the pool holds; each database needs its file and
        /// its journal (default 6).
        pub capacity: u32,
        /// Empties the pool at install (a scratch database; default false).
        pub clear_on_init: bool,
    }

    impl Default for OpfsOptions {
        fn default() -> Self {
            OpfsOptions {
                directory: ".tiramemsu".to_string(),
                capacity: 6,
                clear_on_init: false,
            }
        }
    }

    /// Installs the OPFS sync-access-handle pool VFS. Call it once in a dedicated
    /// Web Worker before opening a [`Storage::Opfs`] database; it acquires the
    /// pool's handles asynchronously, after which every database call is
    /// synchronous. A second call keeps the first installation.
    ///
    /// # Errors
    ///
    /// `MissingCapability` outside a dedicated worker or without OPFS.
    pub async fn install_opfs(opts: &OpfsOptions) -> Result<()> {
        let cfg = OpfsSAHPoolCfg {
            vfs_name: "tiramemsu-opfs".to_string(),
            directory: opts.directory.clone(),
            clear_on_init: opts.clear_on_init,
            initial_capacity: opts.capacity,
        };
        let util = sahpool::install::<WasmOsCallback>(&cfg, false)
            .await
            .map_err(|e| Error::MissingCapability {
                capability: format!("opfs storage ({e})"),
            })?;
        OPFS.with(|o| *o.borrow_mut() = Some((cfg.vfs_name, util)));
        Ok(())
    }

    pub(super) fn opfs_vfs() -> Result<String> {
        OPFS.with(|o| o.borrow().as_ref().map(|(n, _)| n.clone()))
            .ok_or_else(|| Error::MissingCapability {
                capability: "opfs storage (the OPFS VFS is not installed: call install_opfs \
                             in a dedicated Web Worker first)"
                    .to_string(),
            })
    }

    pub(super) fn ensure_memvfs() {
        MEM.with(|_| ());
    }

    pub(super) fn export(storage: &Storage, path: &str) -> Result<Vec<u8>> {
        match storage {
            Storage::Memory => MEM.with(|m| m.export_db(path).map_err(|e| io(e.to_string()))),
            Storage::Opfs => {
                opfs_vfs()?;
                OPFS.with(|o| {
                    let o = o.borrow();
                    let (_, util) = o.as_ref().expect("checked");
                    util.export_db(path).map_err(|e| io(e.to_string()))
                })
            }
            Storage::File => Err(unavailable(storage)),
        }
    }

    pub(super) fn import(storage: &Storage, path: &str, bytes: &[u8]) -> Result<()> {
        match storage {
            Storage::Memory => {
                MEM.with(|m| m.import_db(path, bytes).map_err(|e| io(e.to_string())))
            }
            Storage::Opfs => {
                opfs_vfs()?;
                OPFS.with(|o| {
                    let o = o.borrow();
                    let (_, util) = o.as_ref().expect("checked");
                    if util.exists(path).unwrap_or(false) {
                        return Err(io(format!("{path} file already exists")));
                    }
                    util.import_db(path, bytes).map_err(|e| io(e.to_string()))
                })
            }
            Storage::File => Err(unavailable(storage)),
        }
    }
}

#[cfg(all(target_family = "wasm", target_os = "unknown"))]
pub use wasm::{install_opfs, OpfsOptions};

/// `Unsupported` for a storage this target does not have.
fn unavailable(storage: &Storage) -> Error {
    let target = if cfg!(all(target_family = "wasm", target_os = "unknown")) {
        "wasm32 (no OS file system; use Storage::Memory or Storage::Opfs)"
    } else {
        "a native target (the memory and OPFS VFSes exist only on wasm32-unknown-unknown; use Storage::File)"
    };
    Error::unsupported(format!("{} storage on {target}", storage.name()))
}

/// The VFS to open `storage` with (`None`: the platform default).
pub(crate) fn vfs_name(storage: &Storage) -> Result<Option<String>> {
    #[cfg(all(target_family = "wasm", target_os = "unknown"))]
    {
        match storage {
            Storage::Memory => {
                wasm::ensure_memvfs();
                Ok(Some(wasm::MEMVFS.to_string()))
            }
            Storage::Opfs => Ok(Some(wasm::opfs_vfs()?)),
            Storage::File => Err(unavailable(storage)),
        }
    }
    #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
    {
        match storage {
            Storage::File => Ok(None),
            _ => Err(unavailable(storage)),
        }
    }
}

pub(crate) fn export(storage: &Storage, path: &str) -> Result<Vec<u8>> {
    #[cfg(all(target_family = "wasm", target_os = "unknown"))]
    {
        wasm::export(storage, path)
    }
    #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
    {
        match storage {
            Storage::File => std::fs::read(path).map_err(|e| io(format!("{path}: {e}"))),
            _ => Err(unavailable(storage)),
        }
    }
}

pub(crate) fn import(storage: &Storage, path: &str, bytes: &[u8]) -> Result<()> {
    #[cfg(all(target_family = "wasm", target_os = "unknown"))]
    {
        wasm::import(storage, path, bytes)
    }
    #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
    {
        match storage {
            Storage::File => {
                if bytes.len() < 100 || !bytes.starts_with(b"SQLite format 3\0") {
                    return Err(io(format!("{path}: not a SQLite database file")));
                }
                let mut f = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(path)
                    .map_err(|e| io(format!("{path}: {e}")))?;
                std::io::Write::write_all(&mut f, bytes).map_err(|e| io(format!("{path}: {e}")))
            }
            _ => Err(unavailable(storage)),
        }
    }
}
