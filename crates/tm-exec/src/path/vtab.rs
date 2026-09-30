//! The `tm_path` table-valued function: `tm_path(start, path, mode, max_hops, view)`
//! returning `(start, "end", hops, path_json)`. The host binds this body to its
//! virtual-table API (`tm_core::ConnTableFunction`); the body reads through the
//! calling connection, so it sees the calling statement's snapshot.

use std::sync::Arc;

use tm_core::{
    ConnTableFunction, Error, Executor, HostRegistry, ObjectId, Result, SqlError, SqlValue,
};
use tm_ir::PathMode;

use super::engine::{PathEngine, PathRequest};
use super::view::parse_view;
use crate::native::{NativeKind, NativeOperator};

/// The SQL name.
pub const NAME: &str = "tm_path";

fn arg_err(arg: &str, msg: impl std::fmt::Display) -> Error {
    Error::Sqlite(SqlError::new(
        SqlError::ERROR,
        format!("{NAME}: {arg}: {msg}"),
    ))
}

/// Evaluates one call: the five arguments, then the pushed-down `"end"`.
// @lat: [[query#Physical Planning#Path Engine#tm_path]]
pub fn call(
    engine: &PathEngine,
    exec: &mut dyn Executor,
    args: &[SqlValue],
) -> Result<Vec<Vec<SqlValue>>> {
    let get = |i: usize| args.get(i).unwrap_or(&SqlValue::Null);
    let path = match get(1) {
        SqlValue::Text(t) => t.as_str(),
        SqlValue::Null => return Err(arg_err("path", "required")),
        _ => return Err(arg_err("path", "expected text")),
    };
    let mode = match get(2) {
        SqlValue::Null => PathMode::Reachability,
        SqlValue::Text(t) => t.parse().map_err(|m| arg_err("mode", m))?,
        _ => return Err(arg_err("mode", "expected text")),
    };
    let max_hops = match get(3) {
        SqlValue::Null => (mode == PathMode::Trail).then_some(engine.options().max_hops),
        SqlValue::Integer(n) if *n >= 0 => Some(u32::try_from(*n).unwrap_or(u32::MAX)),
        _ => return Err(arg_err("max_hops", "expected a non-negative integer")),
    };
    let view = match get(4) {
        SqlValue::Null => tm_core::ViewSpec::NOW,
        SqlValue::Text(t) => parse_view(t).map_err(|m| arg_err("view", m))?,
        _ => return Err(arg_err("view", "expected text")),
    };
    let start = match get(0) {
        SqlValue::Integer(n) => ObjectId::from_raw(*n),
        SqlValue::Null => {
            // an outer join without a match yields no rows; a malformed path still
            // fails, so mistakes do not hide behind an empty input
            check_path(exec, path)?;
            return Ok(Vec::new());
        }
        _ => return Err(arg_err("start", "expected an integer ObjectId")),
    };
    let end = get(5).as_i64().map(ObjectId::from_raw);
    let req = PathRequest {
        start,
        path,
        mode,
        max_hops,
        view,
        end,
    };
    let with_path = mode != PathMode::Reachability;
    let mut rows = Vec::new();
    engine
        .run(exec, &req, &mut |r| {
            rows.push(vec![
                SqlValue::Integer(r.start.raw()),
                SqlValue::Integer(r.end.raw()),
                SqlValue::Integer(i64::from(r.hops)),
                match (&r.path, with_path) {
                    (Some(p), true) => SqlValue::Text(p.to_json()),
                    _ => SqlValue::Null,
                },
            ]);
            Ok(true)
        })
        .map_err(parse_err)?;
    Ok(rows)
}

fn check_path(exec: &mut dyn Executor, path: &str) -> Result<()> {
    let mut lazy = super::engine::Lazy::new(exec);
    super::syntax::parse(path, &mut lazy)
        .map(|_| ())
        .map_err(parse_err)
}

fn parse_err(e: Error) -> Error {
    match e {
        Error::Parse { msg, span, .. } => arg_err(
            "path",
            format!(
                "{msg}{}",
                span.map(|s| format!(" at offset {}", s.offset))
                    .unwrap_or_default()
            ),
        ),
        other => other,
    }
}

/// The path operator: registers `tm_path` on every connection.
#[derive(Debug)]
pub struct PathOperator {
    engine: Arc<PathEngine>,
}

impl PathOperator {
    /// An operator over `engine`.
    pub fn new(engine: Arc<PathEngine>) -> PathOperator {
        PathOperator { engine }
    }

    /// The engine.
    pub fn engine(&self) -> &Arc<PathEngine> {
        &self.engine
    }
}

impl NativeOperator for PathOperator {
    fn kind(&self) -> NativeKind {
        NativeKind::Path
    }

    fn tvf_name(&self) -> &'static str {
        NAME
    }

    fn register(&self, host: &mut dyn HostRegistry) -> Result<()> {
        let engine = self.engine.clone();
        host.register_conn_table(ConnTableFunction {
            name: NAME.to_string(),
            args: ["start", "path", "mode", "max_hops", "view"]
                .map(String::from)
                .to_vec(),
            columns: ["start", "end", "hops", "path_json"]
                .map(String::from)
                .to_vec(),
            pushdown: vec!["end".to_string()],
            func: Arc::new(move |exec, args| call(&engine, exec, args)),
        })
    }

    fn engine(&self) -> Option<&Arc<PathEngine>> {
        Some(&self.engine)
    }
}
