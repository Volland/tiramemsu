//! The `tm_recall` table-valued function: `tm_recall(text, mode, model, vector,
//! view, graphs, limit, expand)` returning `(eid, score, rank, via)`, one row per
//! result statement in rank order. The body is [`tm_core::recall::recall`], the
//! hybrid recall behind `View::recall`, run on the calling connection so it sees
//! the calling statement's snapshot. IR [`tm_ir::RecallPattern`]s compile to a call.

use std::sync::Arc;

use tm_core::recall::{self, RecallQuery};
use tm_core::text::{TextMode, TextQuery};
use tm_core::vector::{parse_vector, VectorQuery};
use tm_core::{ConnTableFunction, Error, Executor, Result, SqlError, SqlValue, TermReader, Value};

use crate::path::view::parse_view_arg;
use crate::text::parse_graphs;

/// The SQL name.
pub const NAME: &str = "tm_recall";

fn arg_err(arg: &str, msg: impl std::fmt::Display) -> Error {
    Error::Sqlite(SqlError::new(
        SqlError::ERROR,
        format!("{NAME}: {arg}: {msg}"),
    ))
}

fn opt_text(v: &SqlValue, arg: &str) -> Result<Option<String>> {
    match v {
        SqlValue::Null => Ok(None),
        SqlValue::Text(t) => Ok(Some(t.clone())),
        _ => Err(arg_err(arg, "expected text")),
    }
}

/// Evaluates one call.
// @lat: [[query#Hybrid Recall#Query Languages]]
pub fn call(exec: &mut dyn Executor, args: &[SqlValue]) -> Result<Vec<Vec<SqlValue>>> {
    let get = |i: usize| args.get(i).unwrap_or(&SqlValue::Null);
    let text = opt_text(get(0), "text")?;
    let mode = match get(1) {
        SqlValue::Null => TextMode::All,
        SqlValue::Text(t) => {
            TextMode::from_name(t).ok_or_else(|| arg_err("mode", "expected all, any or phrase"))?
        }
        _ => return Err(arg_err("mode", "expected text")),
    };
    let model = opt_text(get(2), "model")?;
    let vector = opt_text(get(3), "vector")?
        .map(|v| parse_vector(&v).map_err(|m| arg_err("vector", m)))
        .transpose()?;
    let spec = match get(4) {
        SqlValue::Null => tm_core::ViewSpec::NOW,
        SqlValue::Text(t) => parse_view_arg(t).map_err(|m| arg_err("view", m))?.0,
        _ => return Err(arg_err("view", "expected text")),
    };
    let graphs = parse_graphs(get(5))?;
    let limit = match get(6) {
        SqlValue::Null => None,
        SqlValue::Integer(n) if *n >= 0 => Some(*n as usize),
        _ => return Err(arg_err("limit", "expected a non-negative integer")),
    };
    let expand = match get(7) {
        SqlValue::Null => 0,
        SqlValue::Integer(n) if *n >= 0 => *n as usize,
        _ => return Err(arg_err("expand", "expected a non-negative integer")),
    };
    let text = text.map(|t| TextQuery {
        text: t,
        mode,
        graphs: graphs.clone(),
        ..TextQuery::default()
    });
    let vector = match (model, vector) {
        (Some(m), Some(v)) => {
            let Some(id) = TermReader::encode(exec, &Value::iri(m.clone()))? else {
                return Err(Error::invalid_query(format!(
                    "<{m}> is not a defined embedding model"
                )));
            };
            Some(VectorQuery {
                graphs,
                ..VectorQuery::new(id, v)
            })
        }
        (None, None) => None,
        _ => {
            return Err(Error::invalid_query(
                "recall's vector channel needs both a model and a vector",
            ))
        }
    };
    let q = RecallQuery {
        text,
        vector,
        limit,
        expand,
        ..RecallQuery::default()
    };
    let hits = recall::recall(exec, &spec, &q, &TermReader::new(64), false)?;
    Ok(hits
        .into_iter()
        .map(|h| {
            vec![
                SqlValue::Integer(h.eid.oid().raw()),
                SqlValue::Real(h.score),
                SqlValue::Integer(h.rank as i64),
                h.via
                    .map_or(SqlValue::Null, |v| SqlValue::Integer(v.oid().raw())),
            ]
        })
        .collect())
}

/// The table function registered on every connection by [`crate::host::install`].
pub fn table_function() -> ConnTableFunction {
    ConnTableFunction {
        name: NAME.to_string(),
        args: [
            "text", "mode", "model", "vector", "view", "graphs", "limit", "expand",
        ]
        .map(String::from)
        .to_vec(),
        columns: ["eid", "score", "rank", "via"].map(String::from).to_vec(),
        pushdown: Vec::new(),
        func: Arc::new(call),
    }
}
