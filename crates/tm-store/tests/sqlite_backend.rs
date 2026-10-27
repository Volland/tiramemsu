//! The trait engine on SQLite (`move-sqlite-onto-store`): the storage contract,
//! and the oracle comparing it with the SQL engine on the same histories. The
//! text index stays off: on SQLite it is FTS5, maintained outside the traits.

use std::sync::Arc;

use tm_core::sqlstore::SqlBackend;
use tm_core::HostOptions;
use tm_rusqlite::RusqliteHost;
use tm_store::{contract, Backend};

#[path = "common/oracle.rs"]
mod oracle;

/// A new SQLite file in its own temporary directory, kept alive with the backend.
struct Fresh {
    backend: SqlBackend,
    _dir: tempfile::TempDir,
}

impl Backend for Fresh {
    fn name(&self) -> &'static str {
        self.backend.name()
    }
    fn read(&self) -> tm_core::Result<Box<dyn tm_store::ReadStore + Send>> {
        self.backend.read()
    }
    fn write(&self) -> tm_core::Result<Box<dyn tm_store::WriteTxn>> {
        self.backend.write()
    }
}

fn fresh() -> Fresh {
    let dir = tempfile::tempdir().unwrap();
    let exec = tm_core::storage::open(
        &RusqliteHost::new(),
        &dir.path().join("t.db"),
        &HostOptions::default(),
    )
    .unwrap();
    Fresh {
        backend: SqlBackend::new(exec),
        _dir: dir,
    }
}

// The raw-row checks of `contract` (scans, scopes, refused second retraction
// and duplicate key) write statement rows without a transaction row; SQLite's
// invariant triggers refuse that by design, so only the engine-level checks
// and the oracle run here.

#[test]
fn assert_on_sqlite() {
    contract::assert_on_the_store(Arc::new(fresh()));
}

#[test]
fn cascade_on_sqlite() {
    contract::cascade_on_the_store(Arc::new(fresh()));
}

#[test]
fn dry_run_burns_ids() {
    contract::dry_run_burns_ids(Arc::new(fresh()));
}

#[test]
fn oracle_fixed_history() {
    oracle::fixed_history_with(Arc::new(fresh()), false);
}

#[test]
fn oracle_random_histories() {
    oracle::random_histories_with(|| Arc::new(fresh()), false);
}

fn v(s: &str) -> tm_core::Value {
    tm_core::Value::iri(format!("urn:tiramemsu:v:{s}"))
}

fn dump(path: &std::path::Path) -> Vec<Vec<String>> {
    let conn = rusqlite::Connection::open(path).unwrap();
    let mut out = Vec::new();
    for table in ["triple", "term", "tx", "meta", "volatile"] {
        let mut st = conn
            .prepare(&format!("SELECT * FROM {table} ORDER BY 1, 2"))
            .unwrap();
        let n = st.column_count();
        let rows = st
            .query_map([], |r| {
                (0..n)
                    .map(|i| {
                        r.get::<_, rusqlite::types::Value>(i)
                            .map(|x| format!("{x:?}"))
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .unwrap();
        for r in rows {
            let mut r = r.unwrap();
            r.insert(0, table.to_string());
            out.push(r);
        }
    }
    out
}

// a file written by the SQL engine and continued by both engines stays identical
// (the evidence of `move-sqlite-onto-store`, closed; see [[backends#One Engine In Two Phases]])
#[test]
fn old_files_keep_working() {
    use tm_core::{ManualClock, Patch, TxOptions, Valid, Value};
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (dir.path().join("a.db"), dir.path().join("b.db"));
    let clock = Arc::new(ManualClock::new(1_000));
    let opts = || tm_core::StoreOptions {
        clock: clock.clone(),
        ..Default::default()
    };
    // a file written by the SQL engine
    let mut fact = None;
    {
        let mut core = tm_core::Store::open(&RusqliteHost::new(), &a, opts()).unwrap();
        core.transact(TxOptions::default(), |tx| {
            let e = tx
                .assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?
                .eid();
            tx.assert(e, v("source"), Value::str("chat"), Valid::ALWAYS)?;
            fact = Some(e);
            tx.assert(v("bob"), v("age"), Value::Int(30), Valid::ALWAYS)?;
            Ok(())
        })
        .unwrap();
    }
    let fact = fact.unwrap();
    std::fs::copy(&a, &b).unwrap();
    // the same writes continue it: the SQL engine on `a`, the trait engine on `b`
    let more = |tx_assert: &mut dyn FnMut(u64) -> tm_core::Result<()>| {
        for k in 0..3 {
            tx_assert(k).unwrap();
        }
    };
    {
        let mut core = tm_core::Store::open(&RusqliteHost::new(), &a, opts()).unwrap();
        more(&mut |k| {
            clock.set(2_000 + k as i64);
            core.transact(TxOptions::default(), |tx| {
                if k == 1 {
                    tx.supersede(fact, Patch::object(v("globex")))?;
                } else {
                    tx.assert(v("carol"), v("age"), Value::Int(k as i64), Valid::ALWAYS)?;
                }
                Ok(())
            })
            .map(|_| ())
        });
    }
    {
        let exec =
            tm_core::storage::open(&RusqliteHost::new(), &b, &HostOptions::default()).unwrap();
        let mut tr = tm_store::Engine::open(
            Arc::new(SqlBackend::new(exec)),
            tm_store::engine::EngineOptions {
                clock: clock.clone(),
                ..Default::default()
            },
        )
        .unwrap();
        more(&mut |k| {
            clock.set(2_000 + k as i64);
            tr.transact(TxOptions::default(), |tx| {
                if k == 1 {
                    tx.supersede(fact, Patch::object(v("globex")))?;
                } else {
                    tx.assert(v("carol"), v("age"), Value::Int(k as i64), Valid::ALWAYS)?;
                }
                Ok(())
            })
            .map(|_| ())
        });
    }
    // every row is the same, numbering included
    assert_eq!(dump(&a), dump(&b));
}
