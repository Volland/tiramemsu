//! Execution parameters.

use std::collections::BTreeMap;

use tm_core::Value;

/// Named parameter values supplied at execution (`$name` → value).
pub type Params = BTreeMap<String, Value>;

/// Builds a parameter map from `(name, value)` pairs.
pub fn params<'a>(pairs: impl IntoIterator<Item = (&'a str, Value)>) -> Params {
    pairs
        .into_iter()
        .map(|(k, v)| (k.trim_start_matches('$').to_string(), v))
        .collect()
}
