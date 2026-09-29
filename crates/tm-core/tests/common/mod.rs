//! Shared helpers for the `tm-core` integration tests.
//!
//! Every test built with [`host_test!`] runs twice: on `RusqliteHost` and on
//! `MinimalHost` (no capability, every SQL string recorded and checked).
#![allow(dead_code, unused_macros, unused_imports)]

pub mod minimal;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tempfile::TempDir;
use tm_core::read;
use tm_core::*;
use tm_rusqlite::RusqliteHost;

pub use minimal::{check_core_sql, MinimalHost, Probe};

/// Which host a [`TestDb`] runs on.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum HostKind {
    Rusqlite,
    Minimal,
}

/// Start time of every test clock.
pub const T0: i64 = 1_000_000;

/// A temporary database with a manual clock, on one host.
pub struct TestDb {
    pub dir: TempDir,
    pub path: PathBuf,
    pub clock: Arc<ManualClock>,
    pub kind: HostKind,
    pub probe: Option<Arc<Probe>>,
    pub store: Option<Store>,
    pub optimize_every: u64,
}

/// An IRI in the default vocabulary (`urn:tiramemsu:v:<name>`).
pub fn iri(name: &str) -> Value {
    Value::iri(vocab::v(name))
}

/// A plain string literal.
pub fn lit(s: &str) -> Value {
    Value::str(s)
}

/// A `sys:` IRI.
pub fn sys(name: &str) -> Value {
    Value::iri(format!("{}{name}", vocab::SYS))
}

/// Unwraps a result with a readable message.
#[track_caller]
pub fn assert_ok<T>(r: Result<T>) -> T {
    match r {
        Ok(v) => v,
        Err(e) => panic!("unexpected error: {e:?}"),
    }
}

/// Epoch ms of midnight UTC on a date `YYYY-MM-DD`.
pub fn day(date: &str) -> i64 {
    value::parse_date(date).expect("date") * 86_400_000
}

impl TestDb {
    pub fn new(kind: HostKind) -> TestDb {
        TestDb::with_optimize_every(kind, 1000)
    }

    pub fn with_optimize_every(kind: HostKind, optimize_every: u64) -> TestDb {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("test.db");
        let mut db = TestDb {
            dir,
            path,
            clock: Arc::new(ManualClock::new(T0)),
            kind,
            probe: None,
            store: None,
            optimize_every,
        };
        db.open().expect("open");
        db
    }

    pub fn options(&self) -> StoreOptions {
        StoreOptions {
            clock: self.clock.clone(),
            optimize_every: self.optimize_every,
            ..StoreOptions::default()
        }
    }

    /// Opens (or reopens) the store on this test's host.
    pub fn open(&mut self) -> Result<()> {
        self.store = None;
        let opts = self.options();
        let store = match self.kind {
            HostKind::Rusqlite => Store::open(&RusqliteHost::new(), &self.path, opts)?,
            HostKind::Minimal => {
                let host = MinimalHost::new();
                if let Some(p) = &self.probe {
                    // keep injected faults and the log across reopen
                    let host = MinimalHost {
                        probe: p.clone(),
                        ..host
                    };
                    Store::open(&host, &self.path, opts)?
                } else {
                    self.probe = Some(host.probe.clone());
                    Store::open(&host, &self.path, opts)?
                }
            }
        };
        self.store = Some(store);
        Ok(())
    }

    pub fn close(&mut self) {
        self.store = None;
    }

    pub fn reopen(&mut self) {
        self.close();
        self.open().expect("reopen");
    }

    pub fn store(&mut self) -> &mut Store {
        self.store.as_mut().expect("open store")
    }

    pub fn probe(&self) -> Option<&Probe> {
        self.probe.as_deref()
    }

    /// Runs a transaction that must commit.
    #[track_caller]
    pub fn tx(&mut self, f: impl FnOnce(&mut Tx<'_>) -> Result<()>) -> TxReport {
        assert_ok(self.store().transact(TxOptions::default(), f))
    }

    pub fn try_tx(&mut self, f: impl FnOnce(&mut Tx<'_>) -> Result<()>) -> Result<TxReport> {
        self.store().transact(TxOptions::default(), f)
    }

    pub fn try_tx_opts(
        &mut self,
        opts: TxOptions,
        f: impl FnOnce(&mut Tx<'_>) -> Result<()>,
    ) -> Result<TxReport> {
        self.store().transact(opts, f)
    }

    pub fn read<R>(&mut self, f: impl FnOnce(&mut dyn Executor) -> Result<R>) -> R {
        assert_ok(self.store().read(f))
    }

    pub fn triples(
        &mut self,
        spec: ViewSpec,
        s: Option<ObjectId>,
        p: Option<ObjectId>,
        o: Option<ObjectId>,
    ) -> Vec<Triple> {
        self.read(|e| read::triples(e, &spec, s, p, o))
    }

    pub fn now(
        &mut self,
        s: Option<ObjectId>,
        p: Option<ObjectId>,
        o: Option<ObjectId>,
    ) -> Vec<Triple> {
        self.triples(ViewSpec::NOW, s, p, o)
    }

    pub fn history_all(&mut self) -> Vec<Triple> {
        self.triples(ViewSpec::history(), None, None, None)
    }

    pub fn as_of(&mut self, t: u64) -> Vec<Triple> {
        self.triples(ViewSpec::as_of(TimeRef::Tx(t)), None, None, None)
    }

    pub fn now_eids(&mut self) -> Vec<Eid> {
        self.now(None, None, None).iter().map(|t| t.eid).collect()
    }

    pub fn row(&mut self, eid: Eid) -> Triple {
        self.triples(ViewSpec::history(), None, None, None)
            .into_iter()
            .find(|t| t.eid == eid)
            .expect("row exists")
    }

    pub fn is_live(&mut self, eid: Eid) -> bool {
        self.now_eids().contains(&eid)
    }

    /// Read-path lookup of a value (never inserts).
    pub fn oid(&mut self, v: &Value) -> Option<ObjectId> {
        let v = v.clone();
        self.read(move |e| TermReader::encode(e, &v))
    }

    /// The id of a value that must already exist.
    #[track_caller]
    pub fn id(&mut self, v: &Value) -> ObjectId {
        self.oid(v).unwrap_or_else(|| panic!("{v:?} not interned"))
    }

    pub fn decode(&mut self, id: ObjectId) -> Value {
        let r = TermReader::new(16);
        self.read(|e| r.decode(e, id, false))
    }

    pub fn events_since(&mut self, t: u64) -> Vec<Event> {
        self.read(|e| read::events_since(e, t))
    }

    /// Runs test-only SQL (not recorded as core SQL on the minimal host).
    pub fn quiet<R>(&mut self, f: impl FnOnce(&mut TestDb) -> R) -> R {
        use std::sync::atomic::Ordering;
        let p = self.probe.clone();
        if let Some(p) = &p {
            p.quiet.store(true, Ordering::SeqCst);
        }
        let r = f(self);
        if let Some(p) = &p {
            p.quiet.store(false, Ordering::SeqCst);
        }
        r
    }

    pub fn rows(&mut self, sql: &str) -> Vec<Vec<SqlValue>> {
        self.quiet(|db| db.read(|e| e.rows(sql, &[])))
    }

    pub fn scalar(&mut self, sql: &str) -> i64 {
        self.quiet(|db| db.read(|e| e.query_i64(sql, &[])))
            .unwrap_or(0)
    }

    pub fn count(&mut self, table: &str) -> i64 {
        self.scalar(&format!("SELECT count(*) FROM {table}"))
    }

    pub fn meta(&mut self, key: &str) -> i64 {
        self.scalar(&format!("SELECT value FROM meta WHERE key = '{key}'"))
    }

    /// Contents of every engine table, for no-trace comparisons.
    pub fn snapshot(&mut self) -> Vec<(String, Vec<Vec<SqlValue>>)> {
        ["meta", "term", "tx", "triple", "volatile", "pred_multi"]
            .into_iter()
            .map(|t| {
                let order = if t == "volatile" { "1, 2" } else { "1" };
                (
                    t.to_string(),
                    self.rows(&format!("SELECT * FROM {t} ORDER BY {order}")),
                )
            })
            .collect()
    }

    /// Snapshot without the `meta` table.
    pub fn data_snapshot(&mut self) -> Vec<(String, Vec<Vec<SqlValue>>)> {
        self.snapshot()
            .into_iter()
            .filter(|(t, _)| t != "meta")
            .collect()
    }

    /// A raw SQLite connection on the file, opened without the engine.
    pub fn raw(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(&self.path).expect("raw open")
    }

    pub fn last_t(&mut self) -> u64 {
        self.meta("last_t") as u64
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        if std::thread::panicking() {
            return;
        }
        if let Some(p) = &self.probe {
            for sql in p.log.lock().unwrap().iter() {
                if let Err(e) = check_core_sql(sql) {
                    panic!("core SQL uses an optional feature: {e}");
                }
            }
        }
    }
}

/// Defines one test that runs on both hosts.
#[macro_export]
macro_rules! host_test {
    ($(#[$m:meta])* fn $name:ident($db:ident) $body:block) => {
        $(#[$m])*
        mod $name {
            #[allow(unused_imports)]
            use super::*;
            fn run($db: &mut TestDb) $body
            #[test]
            fn rusqlite() {
                run(&mut TestDb::new(HostKind::Rusqlite));
            }
            #[test]
            fn minimal() {
                run(&mut TestDb::new(HostKind::Minimal));
            }
        }
    };
}

/// Asserts that a result is the given error pattern.
#[macro_export]
macro_rules! assert_err {
    ($e:expr, $p:pat) => {
        match $e {
            Err($p) => {}
            other => panic!("expected {}, got {:?}", stringify!($p), other),
        }
    };
    ($e:expr, $p:pat if $g:expr) => {
        match $e {
            Err($p) if $g => {}
            other => panic!("expected {}, got {:?}", stringify!($p), other),
        }
    };
}

pub fn path_exists(p: &Path) -> bool {
    p.exists()
}
