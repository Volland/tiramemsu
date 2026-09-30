//! Registration hook for user functions and virtual tables.
//!
//! The query engine registers its host-neutral functions and table functions
//! through [`tm_core::HostRegistry`]; this module holds the host-specific
//! defaults: the `rarray` table-valued function used for frontier chunks.

use rusqlite::Connection;

/// A callback run on every connection the host opens, after it is configured.
pub type RegisterFn = dyn Fn(&Connection) -> rusqlite::Result<()> + Send + Sync;

/// The default registration on every connection: the `rarray` virtual table.
pub fn register_defaults(conn: &Connection) -> rusqlite::Result<()> {
    rusqlite::vtab::array::load_module(conn)
}
