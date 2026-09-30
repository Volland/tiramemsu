//! The host interface of the interpreter: how a compiled Cypher program reaches
//! the store. The facade implements it over a view (reads) or a transaction
//! (reads and writes), so `tm-cypher` stays free of `tm-exec` and SQLite.

use tm_core::{Result, Tx, Value};
use tm_ir::{IrQuery, Params};

/// The rows of an IR query: every cell is a term (`None` = missing).
#[derive(Clone, Debug, Default)]
pub struct Rows {
    /// Column variable names.
    pub columns: Vec<String>,
    /// Rows.
    pub rows: Vec<Vec<Option<Value>>>,
}

impl Rows {
    /// The index of a column.
    pub fn col(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c == name)
    }
}

/// Runs IR and, on a write handle, transaction operations.
pub trait Runner {
    /// Executes an IR query on the connection this program runs on. Each pattern
    /// is evaluated under its own view.
    fn run_ir(&mut self, q: &IrQuery, params: &Params) -> Result<Rows>;

    /// Wall-clock start of the query in epoch milliseconds.
    fn now_ms(&self) -> i64;

    /// The raw 64-bit ObjectId of `v`, or `None` when it is not in the dictionary.
    fn object_id(&mut self, v: &Value) -> Result<Option<i64>>;

    /// Volatile entries `(key iri, value)` of subject `s`.
    fn volatile_of(&mut self, s: &Value) -> Result<Vec<(String, Value)>>;

    /// The hop cap of unbounded variable-length and shortest-path patterns
    /// (`OpenOptions::path_max_hops`).
    fn path_max_hops(&self) -> u32 {
        15
    }

    /// True when write operations are available.
    fn writable(&self) -> bool;

    /// Runs `f` on the transaction. Fails with `Unsupported` on a read-only runner.
    fn with_tx(&mut self, f: &mut dyn FnMut(&mut Tx<'_>) -> Result<()>) -> Result<()>;
}
