//! The `rusqlite` executor host for Tiramemsu: bundled SQLite, prepared-statement
//! caching, busy timeout, and the registration hook for user functions and virtual
//! tables. Host-specific mechanisms stay in this crate.
#![deny(unsafe_code)]
#![warn(missing_docs)]

mod error;
pub mod register;
mod table_fn;

use std::path::Path;
use std::sync::Arc;

use rusqlite::types::Value as RValue;
use rusqlite::{Connection, OpenFlags};
use std::panic::AssertUnwindSafe;

use rusqlite::functions::FunctionFlags;
use tm_core::{
    AggregateFunction, AggregateState, Capabilities, Executor, Host, HostOptions, HostRegistry,
    Result, ScalarFunction, SqlValue, TableFunction,
};

pub use error::map_err;
pub use register::RegisterFn;

/// The rusqlite host: opens [`RusqliteExec`] connections.
#[derive(Clone)]
pub struct RusqliteHost {
    caps: Capabilities,
    register: Option<Arc<RegisterFn>>,
}

impl std::fmt::Debug for RusqliteHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RusqliteHost")
            .field("caps", &self.caps)
            .finish()
    }
}

impl Default for RusqliteHost {
    fn default() -> Self {
        RusqliteHost::new()
    }
}

/// The `PRAGMA compile_options` of the linked SQLite.
pub fn compile_options() -> Vec<String> {
    let Ok(conn) = Connection::open_in_memory() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Ok(mut st) = conn.prepare("PRAGMA compile_options") {
        if let Ok(rows) = st.query_map([], |r| r.get::<_, String>(0)) {
            out.extend(rows.flatten());
        }
    }
    out
}

impl RusqliteHost {
    /// A host that declares every capability its SQLite supports: `reader_pool`,
    /// `functions` and `vtab` always; `stat4` and `fts5` after checking
    /// `PRAGMA compile_options`.
    pub fn new() -> RusqliteHost {
        let opts = compile_options();
        let has = |o: &str| opts.iter().any(|x| x == o);
        RusqliteHost {
            caps: Capabilities {
                reader_pool: true,
                functions: true,
                vtab: true,
                stat4: has("ENABLE_STAT4"),
                fts5: has("ENABLE_FTS5"),
            },
            register: None,
        }
    }

    /// Adds a registration callback run on every opened connection.
    pub fn with_register(mut self, f: Arc<RegisterFn>) -> RusqliteHost {
        self.register = Some(f);
        self
    }

    fn open(&self, path: &Path, flags: OpenFlags, opts: &HostOptions) -> Result<RusqliteExec> {
        let conn = Connection::open_with_flags(path, flags).map_err(map_err)?;
        conn.busy_timeout(opts.busy_timeout).map_err(map_err)?;
        conn.set_prepared_statement_cache_capacity(256);
        register::register_defaults(&conn).map_err(map_err)?;
        if let Some(f) = &self.register {
            f(&conn).map_err(map_err)?;
        }
        Ok(RusqliteExec {
            conn,
            caps: self.caps,
        })
    }
}

impl Host for RusqliteHost {
    fn capabilities(&self) -> Capabilities {
        self.caps
    }

    fn open_writer(&self, path: &Path, opts: &HostOptions) -> Result<Box<dyn Executor>> {
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_URI;
        Ok(Box::new(self.open(path, flags, opts)?))
    }

    fn open_reader(&self, path: &Path, opts: &HostOptions) -> Result<Box<dyn Executor>> {
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_URI;
        Ok(Box::new(self.open(path, flags, opts)?))
    }
}

/// One rusqlite connection implementing the executor trait.
pub struct RusqliteExec {
    conn: Connection,
    caps: Capabilities,
}

impl std::fmt::Debug for RusqliteExec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RusqliteExec").finish()
    }
}

impl RusqliteExec {
    /// Wraps an existing connection (declaring `caps`).
    pub fn from_connection(conn: Connection, caps: Capabilities) -> RusqliteExec {
        RusqliteExec { conn, caps }
    }

    /// The underlying connection (host-specific uses such as registration).
    pub fn connection(&self) -> &Connection {
        &self.conn
    }
}

fn to_rusqlite(v: &SqlValue) -> RValue {
    match v {
        SqlValue::Null => RValue::Null,
        SqlValue::Integer(i) => RValue::Integer(*i),
        SqlValue::Real(r) => RValue::Real(*r),
        SqlValue::Text(s) => RValue::Text(s.clone()),
        SqlValue::Blob(b) => RValue::Blob(b.clone()),
    }
}

fn from_ref(v: rusqlite::types::ValueRef<'_>) -> SqlValue {
    use rusqlite::types::ValueRef;
    match v {
        ValueRef::Null => SqlValue::Null,
        ValueRef::Integer(i) => SqlValue::Integer(i),
        ValueRef::Real(r) => SqlValue::Real(r),
        ValueRef::Text(t) => SqlValue::Text(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) => SqlValue::Blob(b.to_vec()),
    }
}

impl Executor for RusqliteExec {
    fn capabilities(&self) -> Capabilities {
        self.caps
    }

    fn execute(&mut self, sql: &str, params: &[SqlValue]) -> Result<usize> {
        let mut st = self.conn.prepare_cached(sql).map_err(map_err)?;
        let mut rows = st
            .query(rusqlite::params_from_iter(params.iter().map(to_rusqlite)))
            .map_err(map_err)?;
        while rows.next().map_err(map_err)?.is_some() {}
        drop(rows);
        Ok(self.conn.changes() as usize)
    }

    fn query(
        &mut self,
        sql: &str,
        params: &[SqlValue],
        row: &mut dyn FnMut(&[SqlValue]) -> Result<()>,
    ) -> Result<()> {
        let mut st = self.conn.prepare_cached(sql).map_err(map_err)?;
        let n = st.column_count();
        let mut rows = st
            .query(rusqlite::params_from_iter(params.iter().map(to_rusqlite)))
            .map_err(map_err)?;
        let mut buf: Vec<SqlValue> = Vec::with_capacity(n);
        while let Some(r) = rows.next().map_err(map_err)? {
            buf.clear();
            for i in 0..n {
                buf.push(from_ref(r.get_ref(i).map_err(map_err)?));
            }
            row(&buf)?;
        }
        Ok(())
    }

    fn execute_batch(&mut self, sql: &str) -> Result<()> {
        self.conn.execute_batch(sql).map_err(map_err)
    }

    fn begin_immediate(&mut self) -> Result<()> {
        self.execute_batch("BEGIN IMMEDIATE")
    }

    fn begin_read(&mut self) -> Result<()> {
        self.execute_batch("BEGIN DEFERRED")
    }

    fn commit(&mut self) -> Result<()> {
        self.execute_batch("COMMIT")
    }

    fn rollback(&mut self) -> Result<()> {
        self.execute_batch("ROLLBACK")
    }

    fn savepoint(&mut self, name: &str) -> Result<()> {
        self.execute_batch(&format!("SAVEPOINT \"{name}\""))
    }

    fn rollback_to(&mut self, name: &str) -> Result<()> {
        self.execute_batch(&format!("ROLLBACK TO \"{name}\""))
    }

    fn release(&mut self, name: &str) -> Result<()> {
        self.execute_batch(&format!("RELEASE \"{name}\""))
    }

    fn registry(&mut self) -> Option<&mut dyn HostRegistry> {
        if self.caps.functions || self.caps.vtab {
            Some(self)
        } else {
            None
        }
    }
}

fn user_err(m: String) -> rusqlite::Error {
    rusqlite::Error::UserFunctionError(m.into())
}

fn flags(deterministic: bool) -> FunctionFlags {
    let mut f = FunctionFlags::SQLITE_UTF8;
    if deterministic {
        f |= FunctionFlags::SQLITE_DETERMINISTIC | FunctionFlags::SQLITE_INNOCUOUS;
    }
    f
}

struct AggAdapter(Arc<dyn Fn() -> Box<dyn AggregateState> + Send + Sync>);

type AggAcc = AssertUnwindSafe<Box<dyn AggregateState>>;

impl rusqlite::functions::Aggregate<AggAcc, RValue> for AggAdapter {
    fn init(&self, _ctx: &mut rusqlite::functions::Context<'_>) -> rusqlite::Result<AggAcc> {
        Ok(AssertUnwindSafe((self.0)()))
    }

    fn step(
        &self,
        ctx: &mut rusqlite::functions::Context<'_>,
        acc: &mut AggAcc,
    ) -> rusqlite::Result<()> {
        let args: Vec<SqlValue> = (0..ctx.len()).map(|i| from_ref(ctx.get_raw(i))).collect();
        acc.0.step(&args).map_err(user_err)
    }

    fn finalize(
        &self,
        _ctx: &mut rusqlite::functions::Context<'_>,
        acc: Option<AggAcc>,
    ) -> rusqlite::Result<RValue> {
        let mut st = match acc {
            Some(a) => a.0,
            None => (self.0)(),
        };
        st.finish().map(|v| to_rusqlite(&v)).map_err(user_err)
    }
}

impl HostRegistry for RusqliteExec {
    fn register_scalar(&mut self, f: ScalarFunction) -> Result<()> {
        let body = f.func.clone();
        self.conn
            .create_scalar_function(
                f.name.as_str(),
                f.n_args,
                flags(f.deterministic),
                move |ctx| {
                    let args: Vec<SqlValue> =
                        (0..ctx.len()).map(|i| from_ref(ctx.get_raw(i))).collect();
                    body(&args).map(|v| to_rusqlite(&v)).map_err(user_err)
                },
            )
            .map_err(map_err)
    }

    fn register_aggregate(&mut self, f: AggregateFunction) -> Result<()> {
        self.conn
            .create_aggregate_function(
                f.name.as_str(),
                f.n_args,
                flags(true),
                AggAdapter(f.init.clone()),
            )
            .map_err(map_err)
    }

    fn register_table(&mut self, f: TableFunction) -> Result<()> {
        table_fn::register(&self.conn, f).map_err(map_err)
    }
}
