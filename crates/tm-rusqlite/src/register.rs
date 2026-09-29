//! Registration hook for user functions and virtual tables.
//!
//! Empty in M0; the query engine (M1) and the path engine register `tm_path`,
//! `rarray` users and scalar functions here.

use rusqlite::Connection;

/// A callback run on every connection the host opens, after it is configured.
pub type RegisterFn = dyn Fn(&Connection) -> rusqlite::Result<()> + Send + Sync;

/// The default registration: nothing in M0.
pub fn register_defaults(_conn: &Connection) -> rusqlite::Result<()> {
    Ok(())
}
