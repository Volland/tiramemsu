//! The backend benchmark harness behind `tm bench` and `bench/backends`.
//!
//! Seeded generators ([`gen`]) produce the same triples for every engine; the
//! driver ([`suite::run`]) loads them, checks that every engine returns the same
//! results for a query before timing it, and records medians, p95, sizes and
//! throughput with the run's inputs; [`report::markdown`] turns a results file
//! into tables and derived strengths and weaknesses. The engines here are
//! tiramemsu on SQLite (with and without LFTJ) and on redb; `bench/backends`
//! adds Oxigraph, which needs RocksDB and so stays out of the workspace.

pub mod engine;
pub mod gen;
#[cfg(feature = "n3")]
pub mod reason;
pub mod report;
pub mod suite;

pub use engine::{Engine, Rows, Tiramemsu, ViewSel, WriteOp, TIMEOUT};
pub use suite::{check, run, Config, Dataset, Entry, Failure, Mismatch, Results};

use std::path::Path;

use tiramemsu::Backend;

/// The tiramemsu engines for a dataset: SQLite and redb, and SQLite with LFTJ
/// for the cyclic graphs.
pub fn tiramemsu_engines(dir: &Path, d: Dataset) -> Result<Vec<Box<dyn Engine>>, String> {
    let mut v: Vec<Box<dyn Engine>> =
        vec![Box::new(Tiramemsu::create(dir, Backend::Sqlite, false)?)];
    if d == Dataset::Cyclic {
        v.push(Box::new(Tiramemsu::create(dir, Backend::Sqlite, true)?));
    }
    v.push(Box::new(Tiramemsu::create(dir, Backend::Redb, false)?));
    Ok(v)
}
