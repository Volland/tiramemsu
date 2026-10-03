#![doc = include_str!("../README.md")]
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
use std::sync::Mutex;
use tm_core::{
    AggregateFunction, AggregateState, Capabilities, ConnTableFunction, Error, Executor, Host,
    HostOptions, HostRegistry, Interrupt, Result, ScalarFunction, SqlError, SqlValue,
    TableFunction,
};

pub use error::map_err;
pub use register::RegisterFn;

/// The rusqlite host: opens [`RusqliteExec`] connections.
///
/// Pass it to `tm_core::Store::open`. It is cheap to clone and holds no connection
/// itself; every `open_writer` / `open_reader` call opens a new one. The writer
/// creates the file when it is missing, readers are read-only.
///
/// ```
/// use tm_core::Host;
/// use tm_rusqlite::RusqliteHost;
///
/// let caps = RusqliteHost::default().capabilities();
/// assert!(caps.reader_pool && caps.functions && caps.vtab);
/// ```
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
///
/// Useful to see why `stat4` or `fts5` is or is not declared. Returns an empty list
/// if an in-memory connection cannot be opened.
///
/// ```
/// let opts = tm_rusqlite::compile_options();
/// assert!(opts.iter().any(|o| o.starts_with("THREADSAFE")));
/// ```
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
    ///
    /// The callback runs after the busy timeout and the default registrations, on
    /// the writer and on every reader, so a function registered here is visible to
    /// all queries. If it fails, opening the connection fails with `Error::Sqlite`.
    /// A second call replaces the first callback.
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
        Ok(RusqliteExec::from_connection(conn, self.caps))
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
///
/// Normally created by [`RusqliteHost`]. Statements run through
/// `prepare_cached`, parameters are bound positionally (`?1`, `?2`, ...), and an
/// `SqlValue::IntArray` is bound as a `rarray` value.
///
/// ```
/// use tm_core::{Capabilities, Executor, SqlValue};
/// use tm_rusqlite::RusqliteExec;
///
/// let conn = rusqlite::Connection::open_in_memory().unwrap();
/// let mut exec = RusqliteExec::from_connection(conn, Capabilities::default());
/// exec.execute_batch("CREATE TABLE t(x INTEGER)").unwrap();
/// exec.execute("INSERT INTO t VALUES (?1)", &[SqlValue::Integer(7)]).unwrap();
/// let mut seen = Vec::new();
/// exec.query("SELECT x FROM t", &[], &mut |r| { seen.push(r[0].clone()); Ok(()) }).unwrap();
/// assert_eq!(seen, vec![SqlValue::Integer(7)]);
/// ```
pub struct RusqliteExec {
    conn: Connection,
    caps: Capabilities,
    /// The typed error a native table function stored before failing its statement.
    slot: ErrorSlot,
    /// Non-owning handles on `conn` that table functions use to read re-entrantly;
    /// their statement caches are flushed before `conn` closes.
    borrowed: Vec<Arc<Mutex<RusqliteExec>>>,
    /// The stop conditions of the running operation, polled by the progress handler.
    interrupt: Option<Interrupt>,
}

/// SQLite VM instructions between two polls of an installed [`Interrupt`].
const INTERRUPT_EVERY_OPS: std::ffi::c_int = 1000;

/// A per-connection slot for the typed error of a failed native table function.
pub(crate) type ErrorSlot = Arc<Mutex<Option<Error>>>;

impl Drop for RusqliteExec {
    fn drop(&mut self) {
        for b in self.borrowed.drain(..) {
            if let Ok(g) = b.lock() {
                g.conn.flush_prepared_statement_cache();
            }
        }
    }
}

impl std::fmt::Debug for RusqliteExec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RusqliteExec").finish()
    }
}

impl RusqliteExec {
    /// Wraps an existing connection (declaring `caps`).
    pub fn from_connection(conn: Connection, caps: Capabilities) -> RusqliteExec {
        RusqliteExec {
            conn,
            caps,
            slot: ErrorSlot::default(),
            borrowed: Vec::new(),
            interrupt: None,
        }
    }

    /// The typed error of a failed native table function if there is one, else the
    /// mapped SQLite error; an interrupt caused by the installed stop conditions
    /// becomes their typed error (`Cancelled`, `DeadlineExceeded`).
    fn fail(&self, e: rusqlite::Error) -> Error {
        let typed = self.slot.lock().ok().and_then(|mut g| g.take());
        self.typed_interrupt(typed.unwrap_or_else(|| map_err(e)))
    }

    fn typed_interrupt(&self, e: Error) -> Error {
        match &self.interrupt {
            Some(i) if e.sql().is_some_and(|s| s.code == SqlError::INTERRUPT) => {
                i.check().err().unwrap_or(e)
            }
            _ => e,
        }
    }

    fn clear_slot(&self) {
        if let Ok(mut g) = self.slot.lock() {
            *g = None;
        }
    }

    /// The underlying connection (host-specific uses such as registration).
    pub fn connection(&self) -> &Connection {
        &self.conn
    }
}

/// A bound parameter: a plain value or an integer array (`rarray`).
enum Bind {
    Val(RValue),
    Arr(rusqlite::vtab::array::Array),
}

impl rusqlite::ToSql for Bind {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        match self {
            Bind::Val(v) => v.to_sql(),
            Bind::Arr(a) => a.to_sql(),
        }
    }
}

fn to_bind(v: &SqlValue) -> Bind {
    match v {
        SqlValue::IntArray(xs) => Bind::Arr(std::rc::Rc::new(
            xs.iter().map(|x| RValue::Integer(*x)).collect(),
        )),
        other => Bind::Val(to_rusqlite(other)),
    }
}

fn to_rusqlite(v: &SqlValue) -> RValue {
    match v {
        SqlValue::Null => RValue::Null,
        SqlValue::Integer(i) => RValue::Integer(*i),
        SqlValue::Real(r) => RValue::Real(*r),
        SqlValue::Text(s) => RValue::Text(s.clone()),
        SqlValue::Blob(b) => RValue::Blob(b.clone()),
        SqlValue::IntArray(_) => RValue::Null,
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
        self.clear_slot();
        let mut st = self.conn.prepare_cached(sql).map_err(|e| self.fail(e))?;
        let mut rows = st
            .query(rusqlite::params_from_iter(params.iter().map(to_bind)))
            .map_err(|e| self.fail(e))?;
        while rows.next().map_err(|e| self.fail(e))?.is_some() {}
        drop(rows);
        Ok(self.conn.changes() as usize)
    }

    fn query(
        &mut self,
        sql: &str,
        params: &[SqlValue],
        row: &mut dyn FnMut(&[SqlValue]) -> Result<()>,
    ) -> Result<()> {
        self.clear_slot();
        let mut st = self.conn.prepare_cached(sql).map_err(|e| self.fail(e))?;
        let n = st.column_count();
        let mut rows = st
            .query(rusqlite::params_from_iter(params.iter().map(to_bind)))
            .map_err(|e| self.fail(e))?;
        let mut buf: Vec<SqlValue> = Vec::with_capacity(n);
        while let Some(r) = rows.next().map_err(|e| self.fail(e))? {
            buf.clear();
            for i in 0..n {
                buf.push(from_ref(r.get_ref(i).map_err(map_err)?));
            }
            row(&buf)?;
        }
        Ok(())
    }

    fn execute_batch(&mut self, sql: &str) -> Result<()> {
        self.conn
            .execute_batch(sql)
            .map_err(|e| self.typed_interrupt(map_err(e)))
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

    fn set_interrupt(&mut self, interrupt: Option<Interrupt>) {
        // The progress handler is per connection handle, so it also stops the
        // neighbour queries a native table function runs on its borrowed handle.
        let installed = match interrupt.clone() {
            Some(i) => self
                .conn
                .progress_handler(INTERRUPT_EVERY_OPS, Some(move || i.tripped())),
            None => self.conn.progress_handler(0, None::<fn() -> bool>),
        };
        // only an owning connection can install a handler; a borrowed one keeps none
        self.interrupt = installed.ok().and(interrupt);
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

    fn register_conn_table(&mut self, f: ConnTableFunction) -> Result<()> {
        let inner =
            table_fn::borrowed_exec(&self.conn, self.caps, self.slot.clone()).map_err(map_err)?;
        let inner = Arc::new(Mutex::new(inner));
        self.borrowed.push(inner.clone());
        table_fn::register_conn(&self.conn, f, inner, self.slot.clone()).map_err(map_err)
    }
}
