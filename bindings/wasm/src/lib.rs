//! WebAssembly binding of tiramemsu, meant to run in a Web Worker.
//!
//! [`Worker`] is the binding's logic and builds on every target: it parses the
//! open configuration, opens a [`tiramemsu::Db`] on the [`tm_wasm::WasmHost`]
//! and serves the JSON bridge ([`tiramemsu_json::Database`]) with the calls
//! that cannot work in WebAssembly refused. On `wasm32-unknown-unknown` the
//! `js` module exposes it to JavaScript with wasm-bindgen as the class
//! `Database` (`open`, `call`, `capabilities`, `exportFile`, `close`) and the
//! functions `importFile` and `runtimeInfo`. See the README for a Worker
//! example.
//!
// @lat: [[bindings#WebAssembly]]

#[cfg(all(target_family = "wasm", target_os = "unknown"))]
pub mod js;

use std::sync::Arc;

use serde_json::{json, Value as J};
use tiramemsu::{Db, Error};
use tiramemsu_json::{BindError, Database, Res};
use tm_wasm::{Journal, Storage, WasmClock, WasmHost};

/// The parsed open configuration of a [`Worker`].
///
/// `{"storage": "memory" | "opfs" | "file", "path": "memory.db",
/// "journal": "rollback" | "wal", "queryEngine": true, "opfs": {"directory",
/// "capacity", "clearOnInit"}, "options": {...}}`. `journal` is required: the
/// WASM VFSes keep a rollback journal, and `"wal"` there fails with
/// `MissingCapability` instead of opening in another mode. `options` are the
/// JSON bridge's open options except `readers` and `readerTimeoutMs` (the WASM
/// host has no reader pool). `file` is the native file VFS (tests only).
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// Where the file lives.
    pub storage: Storage,
    /// The file name on that storage.
    pub path: String,
    /// The required journal mode.
    pub journal: Journal,
    /// Install the query engine (SPARQL, Cypher, paths); default true.
    pub query_engine: bool,
    /// The `opfs` object (directory, capacity, clearOnInit), when given.
    pub opfs: J,
    /// The bridge open options.
    pub options: J,
}

fn arg(msg: impl Into<String>) -> BindError {
    BindError::Arg(msg.into())
}

/// The operations a WASM worker refuses: budgets and bulk import sessions read
/// `std::time::Instant`, which `wasm32-unknown-unknown` does not provide, and a
/// worker is single-threaded, so nothing could cancel a running call.
fn refused(op: &str, args: &J) -> Option<BindError> {
    let what = if op == "cancel" || op.starts_with("import") {
        format!("{op} in the WASM binding")
    } else if args.get("budget").is_some_and(|b| !b.is_null()) {
        format!("budget on {op} in the WASM binding")
    } else {
        return None;
    };
    Some(BindError::Db(Error::unsupported(format!(
        "{what} (no std::time::Instant on wasm32-unknown-unknown, and a worker runs one call at a time)"
    ))))
}

impl Config {
    /// Parses a configuration object.
    ///
    /// # Errors
    ///
    /// `InvalidArgument` for a missing or unknown field or value.
    pub fn from_json(j: &J) -> Res<Config> {
        let o = j
            .as_object()
            .ok_or_else(|| arg("the configuration must be an object"))?;
        for k in o.keys() {
            if ![
                "storage",
                "path",
                "journal",
                "queryEngine",
                "opfs",
                "options",
            ]
            .contains(&k.as_str())
            {
                return Err(arg(format!("unknown configuration field {k:?}")));
            }
        }
        let storage = match o.get("storage").and_then(J::as_str) {
            Some("memory") => Storage::Memory,
            Some("opfs") => Storage::Opfs,
            Some("file") => Storage::File,
            _ => return Err(arg("storage must be \"memory\", \"opfs\" or \"file\"")),
        };
        let path = o
            .get("path")
            .and_then(J::as_str)
            .filter(|p| !p.is_empty())
            .ok_or_else(|| arg("path must be a non-empty string"))?
            .to_string();
        let journal = match o.get("journal").and_then(J::as_str) {
            Some("rollback") => Journal::Rollback,
            Some("wal") => Journal::Wal,
            _ => return Err(arg("journal must be \"rollback\" or \"wal\"")),
        };
        let query_engine = match o.get("queryEngine") {
            None | Some(J::Null) => true,
            Some(v) => v
                .as_bool()
                .ok_or_else(|| arg("queryEngine must be a boolean"))?,
        };
        let options = o.get("options").cloned().unwrap_or(J::Null);
        if let Some(opts) = options.as_object() {
            for k in ["readers", "readerTimeoutMs"] {
                if opts.contains_key(k) {
                    return Err(arg(format!(
                        "option {k} is not available: the WASM host has no reader pool"
                    )));
                }
            }
        }
        Ok(Config {
            storage,
            path,
            journal,
            query_engine,
            opfs: o.get("opfs").cloned().unwrap_or(J::Null),
            options,
        })
    }

    /// The host this configuration opens.
    pub fn host(&self) -> WasmHost {
        WasmHost::new(self.storage.clone(), self.journal)
    }
}

/// One open database in a worker: the JSON bridge on the WASM host.
#[derive(Debug)]
pub struct Worker {
    bridge: Database,
    host: WasmHost,
    path: String,
    query_engine: bool,
}

impl Worker {
    /// Opens the database `config` names. On OPFS the VFS must be installed
    /// first (the JavaScript `Database.open` does it).
    ///
    /// # Errors
    ///
    /// `InvalidArgument` for a bad configuration, `MissingCapability` for a
    /// journal mode or storage the runtime lacks, and the errors of
    /// [`Db::open_with_host`].
    pub fn open(config: &Config) -> Res<Worker> {
        let mut opts = Database::open_options(&config.options)?;
        opts.clock = Arc::new(WasmClock);
        opts.readers = 0;
        opts.query_engine = config.query_engine;
        let host = config.host();
        let db = Db::open_with_host(host.clone(), &config.path, opts)?;
        Ok(Worker {
            bridge: Database::from_db(db),
            host,
            path: config.path.clone(),
            query_engine: config.query_engine,
        })
    }

    /// Runs one bridge operation (`args` a JSON object), refusing budgets, bulk
    /// import sessions and `cancel`.
    ///
    /// # Errors
    ///
    /// `Unsupported` for a refused call, otherwise the bridge's errors.
    pub fn call(&self, op: &str, args: &J) -> Res<J> {
        if let Some(e) = refused(op, args) {
            return Err(e);
        }
        self.bridge.call(op, args)
    }

    /// [`Worker::call`] on JSON text, with errors as `{"code","message"}` text.
    pub fn call_text(&self, op: &str, args: &str) -> std::result::Result<String, String> {
        let parsed: J = if args.is_empty() {
            J::Null
        } else {
            serde_json::from_str(args).map_err(|e| {
                arg(format!("arguments are not JSON: {e}"))
                    .to_json()
                    .to_string()
            })?
        };
        self.call(op, &parsed)
            .map(|j| j.to_string())
            .map_err(|e| e.to_json().to_string())
    }

    /// The host's capabilities, storage and journal, and whether the query
    /// engine is installed, as JSON.
    pub fn capabilities(&self) -> J {
        let c = tiramemsu::Host::capabilities(&self.host);
        json!({
            "storage": self.host.storage().name(),
            "journal": match self.host.journal() {
                Journal::Wal => "wal",
                Journal::Rollback => "rollback",
            },
            "queryEngine": self.query_engine,
            "readerPool": c.reader_pool,
            "functions": c.functions,
            "vtab": c.vtab,
            "stat4": c.stat4,
            "fts5": c.fts5,
        })
    }

    /// The committed bytes of the database file (call between transactions).
    ///
    /// # Errors
    ///
    /// `Sqlite` (I/O) when the storage cannot read the file.
    pub fn export_file(&self) -> Res<Vec<u8>> {
        Ok(self.host.export_file(&self.path)?)
    }
}

/// Stores `bytes` (a SQLite database file) as the file `config` names, ready
/// for [`Worker::open`]; it must not exist yet.
///
/// # Errors
///
/// `Sqlite` (I/O) when the file exists or the bytes are not a database.
pub fn import_file(config: &Config, bytes: &[u8]) -> Res<()> {
    Ok(config.host().import_file(&config.path, bytes)?)
}

/// The running SQLite: version, compile options and probed capabilities.
pub fn runtime_info() -> J {
    let info = tm_wasm::runtime_info();
    let c = info.capabilities;
    json!({
        "sqliteVersion": info.sqlite_version,
        "compileOptions": info.compile_options,
        "capabilities": {
            "readerPool": c.reader_pool,
            "functions": c.functions,
            "vtab": c.vtab,
            "stat4": c.stat4,
            "fts5": c.fts5,
        },
    })
}
