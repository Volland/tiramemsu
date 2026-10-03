//! The `tm_text` table-valued function: `tm_text(query, mode, view, graphs, limit)`
//! returning `(eid, score, rank, confidence)`, one row per hit in rank order. The
//! body is [`tm_core::text::search`], the recall behind `View::text_search`, run on
//! the calling connection so it sees the calling statement's snapshot (including
//! an open speculation). IR [`tm_ir::TextPattern`]s compile to a call.

use std::sync::Arc;

use tm_core::text::{self, TextMode, TextQuery};
use tm_core::{
    ConnTableFunction, Error, Executor, ObjectId, Result, SqlError, SqlValue, TermReader,
};

use crate::path::view::parse_view_arg;

/// The SQL name.
pub const NAME: &str = "tm_text";

fn arg_err(arg: &str, msg: impl std::fmt::Display) -> Error {
    Error::Sqlite(SqlError::new(
        SqlError::ERROR,
        format!("{NAME}: {arg}: {msg}"),
    ))
}

/// The `graphs` argument: NULL (no filter), one graph id, or a JSON array of ids.
fn parse_graphs(v: &SqlValue) -> Result<Option<Vec<ObjectId>>> {
    let bad = || {
        arg_err(
            "graphs",
            "expected NULL, an integer ObjectId or a JSON array of them",
        )
    };
    match v {
        SqlValue::Null => Ok(None),
        SqlValue::Integer(n) => Ok(Some(vec![ObjectId::from_raw(*n)])),
        SqlValue::Text(t) => {
            let inner = t
                .trim()
                .strip_prefix('[')
                .and_then(|r| r.strip_suffix(']'))
                .ok_or_else(bad)?;
            if inner.trim().is_empty() {
                return Ok(Some(Vec::new()));
            }
            inner
                .split(',')
                .map(|x| x.trim().parse::<i64>().map(ObjectId::from_raw))
                .collect::<std::result::Result<Vec<_>, _>>()
                .map(Some)
                .map_err(|_| bad())
        }
        _ => Err(bad()),
    }
}

/// Evaluates one call.
// @lat: [[query#Text Recall]]
pub fn call(exec: &mut dyn Executor, args: &[SqlValue]) -> Result<Vec<Vec<SqlValue>>> {
    let get = |i: usize| args.get(i).unwrap_or(&SqlValue::Null);
    let query = match get(0) {
        SqlValue::Text(t) => t.clone(),
        SqlValue::Null => return Ok(Vec::new()),
        _ => return Err(arg_err("query", "expected text")),
    };
    let mode = match get(1) {
        SqlValue::Null => TextMode::All,
        SqlValue::Text(t) => {
            TextMode::from_name(t).ok_or_else(|| arg_err("mode", "expected all, any or phrase"))?
        }
        _ => return Err(arg_err("mode", "expected text")),
    };
    let spec = match get(2) {
        SqlValue::Null => tm_core::ViewSpec::NOW,
        SqlValue::Text(t) => parse_view_arg(t).map_err(|m| arg_err("view", m))?.0,
        _ => return Err(arg_err("view", "expected text")),
    };
    let graphs = parse_graphs(get(3))?;
    let limit = match get(4) {
        SqlValue::Null => None,
        SqlValue::Integer(n) if *n >= 0 => Some(*n as usize),
        _ => return Err(arg_err("limit", "expected a non-negative integer")),
    };
    let q = TextQuery {
        text: query,
        mode,
        graphs,
        limit,
        ..TextQuery::default()
    };
    let hits = text::search(exec, &spec, &q, &TermReader::new(64), false)?;
    Ok(hits
        .into_iter()
        .map(|h| {
            vec![
                SqlValue::Integer(h.eid.oid().raw()),
                SqlValue::Real(h.lexical),
                SqlValue::Integer(h.rank as i64),
                h.evidence.confidence.map_or(SqlValue::Null, SqlValue::Real),
            ]
        })
        .collect())
}

/// The table function registered on every connection by [`crate::host::install`].
pub fn table_function() -> ConnTableFunction {
    ConnTableFunction {
        name: NAME.to_string(),
        args: ["query", "mode", "view", "graphs", "limit"]
            .map(String::from)
            .to_vec(),
        columns: ["eid", "score", "rank", "confidence"]
            .map(String::from)
            .to_vec(),
        pushdown: Vec::new(),
        func: Arc::new(call),
    }
}
