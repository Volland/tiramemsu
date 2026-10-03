//! The JavaScript surface (wasm-bindgen). Errors are thrown as `Error` objects
//! whose message is `tiramemsu:{"code","message"}`, as in the Node.js binding.

use serde_json::Value as J;
use tiramemsu_json::BindError;
use wasm_bindgen::prelude::*;

use crate::{Config, Worker};

fn thrown(e: BindError) -> JsError {
    JsError::new(&format!("tiramemsu:{}", e.to_json()))
}

fn config(text: &str) -> Result<Config, JsError> {
    let j: J = serde_json::from_str(text).map_err(|e| {
        thrown(BindError::Arg(format!(
            "the configuration is not JSON: {e}"
        )))
    })?;
    Config::from_json(&j).map_err(thrown)
}

/// Installs the OPFS VFS when `config` uses OPFS (dedicated Worker only).
async fn prepare(config: &Config) -> Result<(), JsError> {
    if config.storage != tm_wasm::Storage::Opfs {
        return Ok(());
    }
    let o = &config.opfs;
    let mut opts = tm_wasm::OpfsOptions::default();
    if let Some(d) = o.get("directory").and_then(J::as_str) {
        opts.directory = d.to_string();
    }
    if let Some(c) = o.get("capacity").and_then(J::as_u64) {
        opts.capacity = c as u32;
    }
    if let Some(c) = o.get("clearOnInit").and_then(J::as_bool) {
        opts.clear_on_init = c;
    }
    tm_wasm::install_opfs(&opts)
        .await
        .map_err(|e| thrown(BindError::Db(e)))
}

/// An open tiramemsu database inside a worker.
#[wasm_bindgen(js_name = Database)]
pub struct JsDatabase {
    worker: Option<Worker>,
}

#[wasm_bindgen(js_class = Database)]
impl JsDatabase {
    /// Opens (creating if needed) the database a JSON configuration names:
    /// `{"storage": "memory" | "opfs", "path", "journal": "rollback" | "wal",
    /// "queryEngine", "opfs": {...}, "options": {...}}`. Resolves to the
    /// database; every later call is synchronous.
    pub async fn open(config_json: String) -> Result<JsDatabase, JsError> {
        let config = config(&config_json)?;
        prepare(&config).await?;
        let worker = Worker::open(&config).map_err(thrown)?;
        Ok(JsDatabase {
            worker: Some(worker),
        })
    }

    fn worker(&self) -> Result<&Worker, JsError> {
        self.worker
            .as_ref()
            .ok_or_else(|| thrown(BindError::Arg("the database is closed".to_string())))
    }

    /// Runs one JSON-bridge operation; `args` is a JSON object (or empty).
    /// Returns the JSON result text.
    pub fn call(&self, op: &str, args: &str) -> Result<String, JsError> {
        self.worker()?
            .call_text(op, args)
            .map_err(|e| JsError::new(&format!("tiramemsu:{e}")))
    }

    /// The host's capabilities, storage and journal as JSON text.
    pub fn capabilities(&self) -> Result<String, JsError> {
        Ok(self.worker()?.capabilities().to_string())
    }

    /// The committed bytes of the database file, to download or to open with the
    /// native host. Call it between transactions.
    #[wasm_bindgen(js_name = exportFile)]
    pub fn export_file(&self) -> Result<Vec<u8>, JsError> {
        self.worker()?.export_file().map_err(thrown)
    }

    /// Closes the database; later calls throw.
    pub fn close(&mut self) {
        self.worker = None;
    }
}

/// Stores the bytes of a SQLite database file (for example one written by the
/// native host) as the file a configuration names, before `Database.open`.
#[wasm_bindgen(js_name = importFile)]
pub async fn import_file(config_json: String, bytes: Vec<u8>) -> Result<(), JsError> {
    let config = config(&config_json)?;
    prepare(&config).await?;
    crate::import_file(&config, &bytes).map_err(thrown)
}

/// The running SQLite's version, compile options and probed capabilities as JSON
/// text.
#[wasm_bindgen(js_name = runtimeInfo)]
pub fn runtime_info() -> String {
    crate::runtime_info().to_string()
}
