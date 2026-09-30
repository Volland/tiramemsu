//! Python bindings for tiramemsu (PyO3): a thin native class over the JSON bridge.
//!
//! All operations are JSON-in / JSON-out via the `tiramemsu-json` bridge. The GIL is
//! released for every database call so Python threads can query in parallel.
//!
// @lat: [[bindings]]

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use tiramemsu_json::Database;

/// The native database handle exported to Python as `tiramemsu._native.Native`.
///
/// One method opens the database; one method runs any operation. The Python package in
/// `python/tiramemsu` wraps this with typed, idiomatic names.
#[pyclass(frozen, module = "tiramemsu._native")]
struct Native {
    db: Database,
}

#[pymethods]
impl Native {
    /// Opens (or creates) the database at `path`. `options` is a JSON object string
    /// (`{"readers": 4, "busyTimeoutMs": 5000, ...}`) or `None`.
    ///
    /// Raises `RuntimeError` with message `tiramemsu:<json>` on failure.
    #[new]
    #[pyo3(signature = (path, options = None))]
    fn new(py: Python<'_>, path: &str, options: Option<&str>) -> PyResult<Self> {
        let opts: serde_json::Value = match options {
            Some(s) if !s.is_empty() => serde_json::from_str(s).map_err(|e| {
                PyRuntimeError::new_err(format!(
                    "tiramemsu:{{\"code\":\"InvalidArgument\",\"message\":\"bad options: {e}\"}}",
                ))
            })?,
            _ => serde_json::Value::Null,
        };
        let db = py
            .detach(|| Database::open(path, &opts))
            .map_err(|e| PyRuntimeError::new_err(format!("tiramemsu:{}", e.to_json())))?;
        Ok(Native { db })
    }

    /// Calls one operation on the database. `args` is a JSON object string, or an empty
    /// string for operations that take no arguments. Returns a JSON string on success.
    ///
    /// Raises `RuntimeError` with message `tiramemsu:<json>` on failure, where `<json>`
    /// is `{"code": "...", "message": "..."}`.
    fn call(&self, py: Python<'_>, op: &str, args: &str) -> PyResult<String> {
        py.detach(|| self.db.call_text(op, args))
            .map_err(|e| PyRuntimeError::new_err(format!("tiramemsu:{e}")))
    }
}

#[pymodule]
#[pyo3(name = "_native")]
fn native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Native>()?;
    Ok(())
}
