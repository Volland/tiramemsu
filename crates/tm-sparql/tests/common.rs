//! Helpers for the lowering tests.
#![allow(dead_code)]

use tm_core::Error;
use tm_ir::View;
use tm_sparql::env::Env;
use tm_sparql::{prepare, Prepared};

pub fn env() -> Env {
    Env::new(View::NOW)
}

/// The IR text of `text` (panics on error).
pub fn ir(text: &str) -> String {
    match prepare(text, &env()).unwrap_or_else(|e| panic!("{text}: {e}")) {
        Prepared::Query(p) => p.query.to_string(),
        Prepared::Update(_) => panic!("not a query: {text}"),
    }
}

/// The error of `text`.
pub fn err(text: &str) -> Error {
    match prepare(text, &env()) {
        Ok(_) => panic!("expected an error for {text}"),
        Err(e) => e,
    }
}

#[track_caller]
pub fn unsupported(text: &str, feature: &str) {
    match err(text) {
        Error::Unsupported { feature: f } => assert_eq!(f, feature, "{text}"),
        other => panic!("{text}: expected Unsupported({feature}), got {other:?}"),
    }
}
