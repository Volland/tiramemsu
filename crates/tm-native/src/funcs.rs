//! The SQL helper functions of `tm-exec`, called directly: the same bodies the
//! SQL route registers on its connections.

use std::collections::HashMap;
use std::sync::OnceLock;

use tm_core::exec::ScalarImpl;
use tm_core::{AggregateState, Error, Result, SqlValue};

struct Registry {
    scalars: HashMap<String, ScalarImpl>,
    aggs: HashMap<String, tm_core::AggregateFunction>,
}

fn registry() -> &'static Registry {
    static R: OnceLock<Registry> = OnceLock::new();
    R.get_or_init(|| {
        let mut scalars = HashMap::new();
        for f in tm_exec::udf::scalar_functions()
            .into_iter()
            .chain(tm_exec::udf_fn::functions())
        {
            scalars.insert(f.name.clone(), f.func.clone());
        }
        let aggs = tm_exec::udf::aggregate_functions()
            .into_iter()
            .map(|a| (a.name.clone(), a))
            .collect();
        Registry { scalars, aggs }
    })
}

/// Calls the scalar function `name`.
pub fn call(name: &str, args: &[SqlValue]) -> Result<SqlValue> {
    let f = registry()
        .scalars
        .get(name)
        .ok_or_else(|| Error::invalid_query(format!("no function {name}")))?;
    f(args).map_err(sql_error)
}

/// The error a SQLite statement reports when a function fails, as the SQL route
/// sees it.
pub fn sql_error(message: String) -> Error {
    Error::Sqlite(tm_core::SqlError {
        code: tm_core::SqlError::ERROR,
        extended_code: tm_core::SqlError::ERROR,
        message,
    })
}

/// A new state of the aggregate function `name`.
pub fn aggregate(name: &str) -> Result<Box<dyn AggregateState>> {
    let a = registry()
        .aggs
        .get(name)
        .ok_or_else(|| Error::invalid_query(format!("no aggregate {name}")))?;
    Ok((a.init)())
}
