//! Mapping `rusqlite::Error` to the host-neutral `SqlError`.

use tm_core::{Error, SqlError};

/// Converts a rusqlite error into `Error::Sqlite`.
pub fn map_err(e: rusqlite::Error) -> Error {
    Error::Sqlite(match &e {
        rusqlite::Error::SqliteFailure(f, msg) => SqlError {
            code: f.extended_code & 0xff,
            extended_code: f.extended_code,
            message: msg.clone().unwrap_or_else(|| f.to_string()),
        },
        other => SqlError::new(SqlError::ERROR, other.to_string()),
    })
}
