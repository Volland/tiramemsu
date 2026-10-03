//! The capability probe: what the running SQLite actually supports.
//!
//! Every capability is established by doing the thing on a scratch in-memory
//! connection, never from the target or a version number, so a runtime built
//! without virtual tables or FTS5 declares less and the facade refuses the query
//! engine with `MissingCapability` instead of degrading.

use rusqlite::functions::FunctionFlags;
use rusqlite::Connection;
use tm_core::Capabilities;

/// What the running SQLite reports and what the probe established.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeInfo {
    /// `sqlite_version()`.
    pub sqlite_version: String,
    /// `PRAGMA compile_options`.
    pub compile_options: Vec<String>,
    /// The probed capabilities (as [`probe_capabilities`]).
    pub capabilities: Capabilities,
}

/// Probes the running SQLite: `functions` if a registered scalar function runs,
/// `vtab` if the `rarray` eponymous virtual table answers a bound array, `fts5` if
/// an FTS5 table can be created and matched, `stat4` if `PRAGMA compile_options`
/// lists `ENABLE_STAT4`. `reader_pool` is never declared by the WASM host.
pub fn probe_capabilities() -> Capabilities {
    runtime_info().capabilities
}

/// The SQLite version, compile options and probed capabilities.
pub fn runtime_info() -> RuntimeInfo {
    let Ok(conn) = Connection::open_in_memory() else {
        return RuntimeInfo {
            sqlite_version: String::new(),
            compile_options: Vec::new(),
            capabilities: Capabilities::default(),
        };
    };
    let compile_options = compile_options(&conn);
    let capabilities = Capabilities {
        reader_pool: false,
        functions: probe_functions(&conn),
        vtab: probe_vtab(&conn),
        stat4: compile_options.iter().any(|o| o == "ENABLE_STAT4"),
        fts5: probe_fts5(&conn),
    };
    RuntimeInfo {
        sqlite_version: conn
            .query_row("SELECT sqlite_version()", [], |r| r.get(0))
            .unwrap_or_default(),
        compile_options,
        capabilities,
    }
}

fn compile_options(conn: &Connection) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(mut st) = conn.prepare("PRAGMA compile_options") {
        if let Ok(rows) = st.query_map([], |r| r.get::<_, String>(0)) {
            out.extend(rows.flatten());
        }
    }
    out
}

fn probe_functions(conn: &Connection) -> bool {
    let registered = conn.create_scalar_function(
        "tm_probe_fn",
        1,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| Ok(ctx.get::<i64>(0)? + 1),
    );
    registered.is_ok()
        && conn
            .query_row("SELECT tm_probe_fn(41)", [], |r| r.get::<_, i64>(0))
            .is_ok_and(|v| v == 42)
}

fn probe_vtab(conn: &Connection) -> bool {
    if tm_rusqlite::register::register_defaults(conn).is_err() {
        return false;
    }
    let values: rusqlite::vtab::array::Array = std::rc::Rc::new(
        [1i64, 2, 3]
            .into_iter()
            .map(rusqlite::types::Value::Integer)
            .collect(),
    );
    conn.query_row("SELECT sum(value) FROM rarray(?1)", [values], |r| {
        r.get::<_, i64>(0)
    })
    .is_ok_and(|v| v == 6)
}

fn probe_fts5(conn: &Connection) -> bool {
    conn.execute_batch(
        "CREATE VIRTUAL TABLE temp.tm_probe_fts USING fts5(x);
         INSERT INTO temp.tm_probe_fts(x) VALUES ('tiramisu layers');",
    )
    .is_ok()
        && conn
            .query_row(
                "SELECT count(*) FROM temp.tm_probe_fts WHERE tm_probe_fts MATCH 'layers'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .is_ok_and(|n| n == 1)
}
