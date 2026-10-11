//! The `tm_vector` table-valued function: `tm_vector(model, vector, view, graphs,
//! limit)` returning `(eid, similarity, rank, confidence)`, one row per hit in rank
//! order. The body is [`tm_core::vector::search`], the recall behind
//! `View::vector_search`, run on the calling connection so it sees the calling
//! statement's snapshot (including an open speculation). IR
//! [`tm_ir::VectorPattern`]s compile to a call.

use std::sync::Arc;

use tm_core::vector::{self, parse_vector, VectorQuery};
use tm_core::{ConnTableFunction, Error, Executor, Result, SqlError, SqlValue, TermReader, Value};

use crate::path::view::parse_view_arg;
use crate::text::parse_graphs;

/// The SQL name.
pub const NAME: &str = "tm_vector";

fn arg_err(arg: &str, msg: impl std::fmt::Display) -> Error {
    Error::Sqlite(SqlError::new(
        SqlError::ERROR,
        format!("{NAME}: {arg}: {msg}"),
    ))
}

/// Evaluates one call.
// @lat: [[query#Vector Recall#Query Languages]]
pub fn call(exec: &mut dyn Executor, args: &[SqlValue]) -> Result<Vec<Vec<SqlValue>>> {
    let get = |i: usize| args.get(i).unwrap_or(&SqlValue::Null);
    let model = match get(0) {
        SqlValue::Text(t) => t.clone(),
        SqlValue::Null => return Ok(Vec::new()),
        _ => return Err(arg_err("model", "expected an IRI")),
    };
    let vector = match get(1) {
        SqlValue::Text(t) => parse_vector(t).map_err(|m| arg_err("vector", m))?,
        SqlValue::Null => return Ok(Vec::new()),
        _ => return Err(arg_err("vector", "expected a JSON array of numbers")),
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
    let predicates = parse_graphs(get(5))?;
    let Some(model) = TermReader::encode(exec, &Value::iri(model.clone()))? else {
        return Err(Error::invalid_query(format!(
            "<{model}> is not a defined embedding model"
        )));
    };
    let q = VectorQuery {
        model: Some(model),
        vector,
        graphs,
        limit,
        predicates,
        ..VectorQuery::default()
    };
    let r = vector::search(exec, &spec, &q, &TermReader::new(64), false)?;
    Ok(r.hits
        .into_iter()
        .map(|h| {
            vec![
                SqlValue::Integer(h.eid.oid().raw()),
                SqlValue::Real(h.similarity),
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
        args: ["model", "vector", "view", "graphs", "limit", "predicates"]
            .map(String::from)
            .to_vec(),
        columns: ["eid", "similarity", "rank", "confidence"]
            .map(String::from)
            .to_vec(),
        pushdown: Vec::new(),
        func: Arc::new(call),
    }
}

#[cfg(test)]
mod tests {
    use tm_core::vector::parse_vector;

    #[test]
    fn vectors_parse_from_json_arrays() {
        assert_eq!(
            parse_vector("[1, -0.5, 2e-1]").unwrap(),
            vec![1.0, -0.5, 0.2]
        );
        assert_eq!(parse_vector(" [] ").unwrap(), Vec::<f32>::new());
        assert!(parse_vector("1, 2").is_err());
        assert!(parse_vector("[1, x]").is_err());
    }
}
