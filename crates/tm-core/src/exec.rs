//! The executor boundary: the only way `tm-core` reaches SQLite.
//!
//! `tm-core` depends on no SQLite binding. A host crate (for example
//! `tm-rusqlite`) implements [`Host`] and [`Executor`].

use std::fmt;
use std::path::Path;
use std::time::Duration;

use crate::error::Result;

/// A SQLite value crossing the executor boundary.
#[derive(Clone, Debug, PartialEq)]
pub enum SqlValue {
    /// NULL.
    Null,
    /// INTEGER.
    Integer(i64),
    /// REAL.
    Real(f64),
    /// TEXT.
    Text(String),
    /// BLOB.
    Blob(Vec<u8>),
}

impl SqlValue {
    /// The integer, if this is one.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            SqlValue::Integer(i) => Some(*i),
            _ => None,
        }
    }

    /// The real, if this is one.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            SqlValue::Real(r) => Some(*r),
            SqlValue::Integer(i) => Some(*i as f64),
            _ => None,
        }
    }

    /// The text, if this is one.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            SqlValue::Text(s) => Some(s),
            _ => None,
        }
    }

    /// True for NULL.
    pub fn is_null(&self) -> bool {
        matches!(self, SqlValue::Null)
    }
}

impl From<i64> for SqlValue {
    fn from(v: i64) -> SqlValue {
        SqlValue::Integer(v)
    }
}

impl From<Option<i64>> for SqlValue {
    fn from(v: Option<i64>) -> SqlValue {
        v.map_or(SqlValue::Null, SqlValue::Integer)
    }
}

impl From<&str> for SqlValue {
    fn from(v: &str) -> SqlValue {
        SqlValue::Text(v.to_string())
    }
}

impl From<String> for SqlValue {
    fn from(v: String) -> SqlValue {
        SqlValue::Text(v)
    }
}

impl From<Option<f64>> for SqlValue {
    fn from(v: Option<f64>) -> SqlValue {
        v.map_or(SqlValue::Null, SqlValue::Real)
    }
}

/// A host-neutral SQLite error: result codes plus message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SqlError {
    /// Primary result code (`SQLITE_BUSY` = 5, ...).
    pub code: i32,
    /// Extended result code.
    pub extended_code: i32,
    /// Message reported by SQLite or the host.
    pub message: String,
}

impl SqlError {
    /// `SQLITE_ERROR`.
    pub const ERROR: i32 = 1;
    /// `SQLITE_BUSY`.
    pub const BUSY: i32 = 5;
    /// `SQLITE_FULL`.
    pub const FULL: i32 = 13;
    /// `SQLITE_CONSTRAINT`.
    pub const CONSTRAINT: i32 = 19;

    /// Builds an error from a primary code and message.
    pub fn new(code: i32, message: impl Into<String>) -> SqlError {
        SqlError {
            code,
            extended_code: code,
            message: message.into(),
        }
    }
}

impl fmt::Display for SqlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (code {})", self.message, self.extended_code)
    }
}

impl std::error::Error for SqlError {}

/// Optional features a host declares. The engine never probes for them.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Capabilities {
    /// Read-only connections on the same file are available.
    pub reader_pool: bool,
    /// Scalar user functions can be registered.
    pub functions: bool,
    /// Virtual tables can be registered.
    pub vtab: bool,
    /// SQLite is compiled with `ENABLE_STAT4`.
    pub stat4: bool,
    /// SQLite is compiled with `ENABLE_FTS5`.
    pub fts5: bool,
}

/// One SQLite connection, driven synchronously.
///
/// Every statement `tm-core` issues, including DDL, pragmas and migrations, goes
/// through this trait. Hosts may cache prepared statements.
// @lat: [[architecture#Executor]]
pub trait Executor: Send {
    /// The capabilities of the host that opened this executor.
    fn capabilities(&self) -> Capabilities;

    /// Runs one statement with bound parameters (`?N`); returns the changed-row count
    /// for DML statements.
    fn execute(&mut self, sql: &str, params: &[SqlValue]) -> Result<usize>;

    /// Runs one statement with bound parameters and calls `row` for every result row.
    fn query(
        &mut self,
        sql: &str,
        params: &[SqlValue],
        row: &mut dyn FnMut(&[SqlValue]) -> Result<()>,
    ) -> Result<()>;

    /// Runs several statements without parameters (DDL, pragmas).
    fn execute_batch(&mut self, sql: &str) -> Result<()>;

    /// `BEGIN IMMEDIATE`.
    fn begin_immediate(&mut self) -> Result<()>;

    /// Begins a read transaction whose snapshot is held until commit or rollback.
    fn begin_read(&mut self) -> Result<()>;

    /// `COMMIT`.
    fn commit(&mut self) -> Result<()>;

    /// `ROLLBACK`.
    fn rollback(&mut self) -> Result<()>;

    /// `SAVEPOINT name`.
    fn savepoint(&mut self, name: &str) -> Result<()>;

    /// `ROLLBACK TO name`.
    fn rollback_to(&mut self, name: &str) -> Result<()>;

    /// `RELEASE name`.
    fn release(&mut self, name: &str) -> Result<()>;
}

impl dyn Executor + '_ {
    /// Collects every row of a query.
    pub fn rows(&mut self, sql: &str, params: &[SqlValue]) -> Result<Vec<Vec<SqlValue>>> {
        let mut out = Vec::new();
        self.query(sql, params, &mut |r| {
            out.push(r.to_vec());
            Ok(())
        })?;
        Ok(out)
    }

    /// The first row of a query, if any.
    pub fn first_row(&mut self, sql: &str, params: &[SqlValue]) -> Result<Option<Vec<SqlValue>>> {
        let mut out = None;
        self.query(sql, params, &mut |r| {
            if out.is_none() {
                out = Some(r.to_vec());
            }
            Ok(())
        })?;
        Ok(out)
    }

    /// The first column of the first row as an integer, if any and not NULL.
    pub fn query_i64(&mut self, sql: &str, params: &[SqlValue]) -> Result<Option<i64>> {
        Ok(self
            .first_row(sql, params)?
            .and_then(|r| r.first().and_then(SqlValue::as_i64)))
    }
}

/// Options a host needs to open a connection.
#[derive(Clone, Debug)]
pub struct HostOptions {
    /// How long a connection waits for a lock before failing with `SQLITE_BUSY`.
    pub busy_timeout: Duration,
}

impl Default for HostOptions {
    fn default() -> Self {
        HostOptions {
            busy_timeout: Duration::from_secs(5),
        }
    }
}

/// A SQLite host: opens executors on a database file.
pub trait Host: Send + Sync {
    /// The capabilities this host declares.
    fn capabilities(&self) -> Capabilities;

    /// Opens the read-write connection (creating the file if needed).
    fn open_writer(&self, path: &Path, opts: &HostOptions) -> Result<Box<dyn Executor>>;

    /// Opens a read-only connection. Only called when `reader_pool` is declared.
    fn open_reader(&self, path: &Path, opts: &HostOptions) -> Result<Box<dyn Executor>>;
}

/// Positional parameters for generated SQL (`?1`, `?2`, ...).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Params(Vec<SqlValue>);

impl Params {
    /// An empty parameter list.
    pub fn new() -> Params {
        Params(Vec::new())
    }

    /// Adds a value and returns its placeholder text (`?N`).
    pub fn push(&mut self, v: impl Into<SqlValue>) -> String {
        self.0.push(v.into());
        format!("?{}", self.0.len())
    }

    /// The bound values.
    pub fn values(&self) -> &[SqlValue] {
        &self.0
    }
}
