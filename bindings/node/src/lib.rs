// Node.js bindings for tiramemsu: a bitemporal, never-forget triple store on SQLite.
//
// One native class `Native` with `constructor(path, options?)` and
// `call(op, args) -> string`. The TypeScript wrapper in `lib/index.ts` gives
// these operations typed, idiomatic names.
//
// @lat: [[bindings]]

use napi::{Error, Result, Status};
use napi_derive::napi;
use tiramemsu_json::{BindError, Database};

fn bind_err(e: BindError) -> Error {
    Error::new(Status::GenericFailure, format!("tiramemsu:{}", e.to_json()))
}

fn arg_err(e: impl std::fmt::Display) -> Error {
    Error::new(Status::GenericFailure, e.to_string())
}

/// The native handle: one open database. Construct with a file path and an optional
/// JSON options string; call operations by name with a JSON argument string.
#[napi]
pub struct Native {
    db: Database,
}

#[napi]
impl Native {
    /// Opens (creating if needed) the database at `path`.
    /// `options` may be a JSON object with `readers`, `busyTimeoutMs`,
    /// `termCacheCapacity`, `optimizeEvery`, `pathMaxHops` and `pathMaxStates`.
    #[napi(constructor)]
    pub fn new(path: String, options: Option<String>) -> Result<Self> {
        let opts: serde_json::Value = options
            .as_deref()
            .map(|s| serde_json::from_str(s).map_err(arg_err))
            .transpose()?
            .unwrap_or(serde_json::Value::Null);
        let db = Database::open(&path, &opts).map_err(bind_err)?;
        Ok(Native { db })
    }

    /// Runs one operation; `args` is a JSON object (empty string is treated as no args).
    /// Returns a JSON string on success, or throws with message `tiramemsu:<error-json>`
    /// where `<error-json>` is `{"code","message"}`.
    #[napi]
    pub fn call(&self, op: String, args: String) -> Result<String> {
        self.db
            .call_text(&op, &args)
            .map_err(|e| Error::new(Status::GenericFailure, format!("tiramemsu:{e}")))
    }
}
