//! Storage backends: detection from the file header, the `backend` and
//! `durability` options, and conversion between backends.

use std::io::Read;
use std::path::Path;

use tm_core::{Error, Result};

/// The storage engine of a database file (`OpenOptions::backend`).
///
/// `Auto` opens an existing file on whatever backend wrote it and creates new
/// files on SQLite. An explicit choice creates new files on that backend and
/// refuses an existing file of the other one with `BackendMismatch`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Backend {
    /// Detect existing files; create new ones on SQLite.
    #[default]
    Auto,
    /// One SQLite file in WAL mode with SQL query execution.
    Sqlite,
    /// One redb file with six live permutations and the native runner (cargo
    /// feature `redb`).
    Redb,
}

impl Backend {
    /// The lower-case name (`"auto"`, `"sqlite"`, `"redb"`).
    pub fn name(self) -> &'static str {
        match self {
            Backend::Auto => "auto",
            Backend::Sqlite => "sqlite",
            Backend::Redb => "redb",
        }
    }

    /// Parses a name (case-insensitive).
    pub fn from_name(s: &str) -> Option<Backend> {
        match s.to_ascii_lowercase().as_str() {
            "auto" => Some(Backend::Auto),
            "sqlite" => Some(Backend::Sqlite),
            "redb" => Some(Backend::Redb),
            _ => None,
        }
    }
}

impl std::fmt::Display for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// When commits become durable (`OpenOptions::durability`), on both backends.
///
/// A crash never leaves a damaged file in any mode; the modes differ in how many
/// of the newest committed transactions a crash may lose.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
// @tg: implements:: [[openspec:durability-modes#Durability option]] {since: "2026-10-27"}
pub enum Durability {
    /// SQLite `synchronous = NORMAL`; redb group commit (durable within
    /// `flush_interval` or `flush_commits`). The newest commits may be lost.
    #[default]
    Normal,
    /// Every commit is durable when it returns (SQLite `synchronous = FULL`).
    Immediate,
    /// Durable only on `Db::sync` and close (SQLite `synchronous = OFF`): for bulk
    /// loads and benchmarks.
    None,
}

impl Durability {
    /// The lower-case name.
    pub fn name(self) -> &'static str {
        match self {
            Durability::Normal => "normal",
            Durability::Immediate => "immediate",
            Durability::None => "none",
        }
    }

    /// Parses a name (case-insensitive).
    pub fn from_name(s: &str) -> Option<Durability> {
        match s.to_ascii_lowercase().as_str() {
            "normal" => Some(Durability::Normal),
            "immediate" => Some(Durability::Immediate),
            "none" => Some(Durability::None),
            _ => None,
        }
    }
}

const SQLITE_MAGIC: &[u8; 16] = b"SQLite format 3\0";
const REDB_MAGIC: [u8; 9] = [b'r', b'e', b'd', b'b', 0x1A, 0x0A, 0xA9, 0x0D, 0x0A];

/// What a path holds, from its first bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Found {
    /// No file, or an empty one.
    Nothing,
    /// A SQLite file.
    Sqlite,
    /// A redb file.
    Redb,
}

/// Reads the header of `path`.
// @lat: [[backends#Backend Detection and CLI]]
// @tg: implements:: [[openspec:backend-selection#Detection by file header]] {since: "2026-10-27"}
pub(crate) fn detect(path: &Path) -> Result<Found> {
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Found::Nothing),
        Err(e) => return Err(Error::storage(format!("{}: {e}", path.display()))),
    };
    let mut head = Vec::with_capacity(16);
    f.by_ref()
        .take(16)
        .read_to_end(&mut head)
        .map_err(|e| Error::storage(format!("{}: {e}", path.display())))?;
    if head.is_empty() {
        return Ok(Found::Nothing);
    }
    if head.as_slice() == SQLITE_MAGIC {
        return Ok(Found::Sqlite);
    }
    if head.len() >= REDB_MAGIC.len() && head[..REDB_MAGIC.len()] == REDB_MAGIC {
        return Ok(Found::Redb);
    }
    Err(Error::UnknownFormat {
        path: path.to_path_buf(),
    })
}

/// The backend to open `path` on, given the requested one.
// @tg: implements:: [[openspec:backend-selection#Explicit backend choice]] {since: "2026-10-27"}
pub(crate) fn resolve(path: &Path, requested: Backend) -> Result<Backend> {
    let found = detect(path)?;
    let actual = match found {
        Found::Nothing => match requested {
            Backend::Auto => default_backend(),
            b => b,
        },
        Found::Sqlite => Backend::Sqlite,
        Found::Redb => Backend::Redb,
    };
    if requested != Backend::Auto && found != Found::Nothing && requested != actual {
        return Err(Error::BackendMismatch {
            path: path.to_path_buf(),
            expected: requested.name().to_string(),
            found: actual.name().to_string(),
        });
    }
    Ok(actual)
}

/// The backend of a new file opened with `Auto`: SQLite, unless the environment
/// variable `TIRAMEMSU_DEFAULT_BACKEND` names another (`redb`). The variable lets
/// a whole test suite run on another backend without changing the tests.
fn default_backend() -> Backend {
    std::env::var("TIRAMEMSU_DEFAULT_BACKEND")
        .ok()
        .and_then(|v| Backend::from_name(&v))
        .filter(|b| *b != Backend::Auto)
        .unwrap_or(Backend::Sqlite)
}

/// The error for a backend this build does not have.
#[cfg_attr(feature = "redb", allow(dead_code))]
// @tg: implements:: [[openspec:backend-selection#Backend compiled out]] {since: "2026-10-27"}
pub(crate) fn not_compiled(b: Backend) -> Error {
    Error::BackendNotCompiled {
        backend: b.name().to_string(),
    }
}

/// Copies the database at `src` into a new file at `dst` on `backend`,
/// losslessly: the same statements with their eids, both clocks and retraction
/// kinds, the same terms, transactions, counters (burned ids included),
/// `pred_multi`, volatile values and embeddings. Derived indexes (the text index,
/// statistics) are rebuilt in the destination. The source is read in one
/// snapshot and never changed.
///
/// # Errors
///
/// `AlreadyExists` when `dst` exists, `UnknownFormat` for a source that is not a
/// database, `BackendNotCompiled` for a backend this build lacks.
///
/// ```
/// # use tiramemsu::*;
/// # let dir = tempfile::tempdir().unwrap();
/// let a = dir.path().join("a.db");
/// let db = Db::open(&a, OpenOptions::default())?;
/// db.transact(TxOptions::default(), |tx| {
///     tx.assert(Value::iri("urn:x:a"), Value::iri("urn:x:p"), Value::Int(1), Valid::ALWAYS)?;
///     Ok(())
/// })?;
/// drop(db);
/// let b = dir.path().join("b.db");
/// convert(&a, &b, Backend::Sqlite)?;
/// assert_eq!(Db::open(&b, OpenOptions::default())?.now().triples(None, None, None)?.len(), 1);
/// # Ok::<(), Error>(())
/// ```
// @lat: [[backends#Same Logical Database]]
// @tg: implements:: [[openspec:redb-storage#Lossless conversion]] {since: "2026-10-27"}
pub fn convert(src: &Path, dst: &Path, backend: Backend) -> Result<tm_store::copy::CopyReport> {
    if dst.exists() {
        return Err(Error::AlreadyExists(dst.to_path_buf()));
    }
    let target = match backend {
        Backend::Auto => Backend::Sqlite,
        b => b,
    };
    #[cfg(not(feature = "redb"))]
    if target == Backend::Redb {
        return Err(not_compiled(Backend::Redb));
    }
    match detect(src)? {
        Found::Nothing => Err(Error::UnknownFormat {
            path: src.to_path_buf(),
        }),
        Found::Sqlite => {
            let mut store = sqlite_store(src)?;
            store.read(|exec| {
                let reader = tm_core::sqlstore::SqlStore::new(exec);
                write_to(&reader, dst, target)
            })
        }
        #[cfg(feature = "redb")]
        Found::Redb => {
            let store = tm_redb::RedbStore::open(src, tm_redb::RedbOptions::default())?;
            let reader = tm_store::Backend::read(&store)?;
            write_to(&*reader, dst, target)
        }
        #[cfg(not(feature = "redb"))]
        Found::Redb => Err(not_compiled(Backend::Redb)),
    }
}

fn sqlite_store(path: &Path) -> Result<tm_core::Store> {
    tm_core::Store::open(
        &tm_rusqlite::RusqliteHost::new(),
        path,
        tm_core::StoreOptions::default(),
    )
}

fn write_to(
    src: &dyn tm_store::ReadStore,
    dst: &Path,
    target: Backend,
) -> Result<tm_store::copy::CopyReport> {
    let text = src.meta("text_index")?.unwrap_or(0) > 0;
    match target {
        #[cfg(feature = "redb")]
        Backend::Redb => {
            let store = tm_redb::RedbStore::create(dst, tm_redb::RedbOptions::default())?;
            let engine = tm_store::Engine::open(
                std::sync::Arc::new(store),
                tm_store::EngineOptions::default(),
            )?;
            let backend = engine.backend().clone();
            let mut w = backend.write()?;
            let rep = match tm_store::copy::copy(src, &mut *w) {
                Ok(r) => r,
                Err(e) => {
                    let _ = w.abort();
                    return Err(e);
                }
            };
            w.commit()?;
            if text {
                let mut w = backend.write()?;
                tm_store::text::rebuild(&mut *w)?;
                w.commit()?;
            }
            let _ = engine;
            Ok(rep)
        }
        _ => {
            let mut store = sqlite_store(dst)?;
            let rep = store.derived_write(|exec| {
                let mut writer = tm_core::sqlstore::SqlStore::new(exec);
                tm_store::copy::copy(src, &mut writer)
            })?;
            if text && store.capabilities().fts5 {
                store.rebuild_text_index()?;
            }
            Ok(rep)
        }
    }
}
