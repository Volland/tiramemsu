//! Facade-level tests: open/reopen, reader pool, concurrency, re-entrancy,
//! speculation through `Db::with`, and `Patch::from_fields`.

#[allow(dead_code)]
#[path = "../../tm-core/tests/common/minimal.rs"]
mod minimal;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;

use tiramemsu::*;

fn v(s: &str) -> Value {
    Value::iri(format!("urn:tiramemsu:v:{s}"))
}

fn lit(s: &str) -> Value {
    Value::str(s)
}

fn tmp() -> (tempfile::TempDir, std::path::PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("f.db");
    (d, p)
}

fn open(path: &std::path::Path) -> Db {
    Db::open(path, OpenOptions::default()).unwrap()
}

fn id(db: &Db, val: &Value) -> ObjectId {
    db.now().encode(val).unwrap().expect("interned")
}

// sql-executor "The rusqlite host": declared capabilities through the facade.
#[test]
fn rusqlite_host_declares_every_capability() {
    let (_d, p) = tmp();
    let db = open(&p);
    let c = db.capabilities();
    assert!(c.reader_pool && c.functions && c.vtab && c.stat4 && c.fts5);
    assert_eq!(db.reader_count(), 4);
    let opts: Vec<String> = db
        .read_sql("PRAGMA compile_options")
        .unwrap()
        .into_iter()
        .filter_map(|r| r[0].as_str().map(str::to_string))
        .collect();
    assert!(opts.iter().any(|o| o == "ENABLE_STAT4"));
    assert!(opts.iter().any(|o| o == "ENABLE_FTS5"));
    // every engine connection is in WAL mode
    let mode = db.read_sql("PRAGMA journal_mode").unwrap();
    assert_eq!(mode[0][0], SqlValue::from("wal"));
}

// sql-executor "Host without a reader pool": the writer serves every read.
#[test]
fn host_without_reader_pool() {
    let (_d, p) = tmp();
    let host = minimal::MinimalHost::new();
    let probe = host.probe.clone();
    let db = Db::open_with_host(host, &p, OpenOptions::default()).unwrap();
    assert_eq!(db.capabilities(), Capabilities::default());
    assert_eq!(db.reader_count(), 0);
    db.transact(TxOptions::default(), |tx| {
        tx.assert(v("a"), v("p"), v("b"), Valid::ALWAYS).map(|_| ())
    })
    .unwrap();
    probe.clear_log();
    assert_eq!(db.now().triples(None, None, None).unwrap().len(), 1);
    let log = probe.log.lock().unwrap().clone();
    assert!(log.iter().any(|s| s.contains("FROM triple a")), "{log:?}");
    // reads from inside a write on the same thread would deadlock the writer
    let r = db.transact(TxOptions::default(), |_| {
        assert!(matches!(
            db.now().triples(None, None, None),
            Err(Error::Reentrant)
        ));
        Ok(())
    });
    assert!(r.is_ok());
    // core SQL recorded on this host uses no optional feature
    for s in probe.log.lock().unwrap().iter() {
        minimal::check_core_sql(s).unwrap();
    }
}

// storage-format "Opening an existing database preserves its contents".
#[test]
fn reopen_continues_numbering_and_keeps_history() {
    let (_d, p) = tmp();
    let (mut max_eid, mut max_node) = (0, 0);
    {
        let db = open(&p);
        for i in 0..7 {
            let rep = db
                .transact(TxOptions::default(), |tx| {
                    let n = tx.new_node()?;
                    max_node = max_node.max(n.unsigned_payload());
                    let e = tx
                        .assert(n, v("n"), Value::Int(i), Valid::between(i, i + 10))?
                        .eid();
                    if i % 2 == 0 {
                        tx.retract(e)?;
                    }
                    Ok(())
                })
                .unwrap();
            max_eid = max_eid.max(rep.asserted[0].n());
        }
        let hist = db.history().triples(None, None, None).unwrap();
        drop(db);
        let db = open(&p);
        assert_eq!(db.history().triples(None, None, None).unwrap(), hist);
        let rep = db
            .transact(TxOptions::default(), |tx| {
                let n = tx.new_node()?;
                assert!(n.unsigned_payload() > max_node);
                tx.assert(n, v("n"), Value::Int(99), Valid::ALWAYS)
                    .map(|_| ())
            })
            .unwrap();
        assert_eq!(rep.t, TxId(8));
        assert!(rep.asserted[0].n() > max_eid);
    }
}

// storage-format "One writer connection and a pool of readers" — reads do not
// block on a running write.
#[test]
fn reads_do_not_block_on_a_running_write() {
    let (_d, p) = tmp();
    let db = Arc::new(open(&p));
    db.transact(TxOptions::default(), |tx| {
        tx.assert(v("a"), v("p"), v("committed"), Valid::ALWAYS)
            .map(|_| ())
    })
    .unwrap();
    let started = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let w = {
        let (db, started, release) = (db.clone(), started.clone(), release.clone());
        thread::spawn(move || {
            db.transact(TxOptions::default(), |tx| {
                tx.assert(v("a"), v("p"), v("pending"), Valid::ALWAYS)?;
                started.wait();
                release.wait();
                Ok(())
            })
            .unwrap()
        })
    };
    started.wait();
    let rows = db.now().triples(None, None, None).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(db.now().decode(rows[0].o).unwrap(), v("committed"));
    release.wait();
    w.join().unwrap();
    assert_eq!(db.now().triples(None, None, None).unwrap().len(), 2);
}

// storage-format "Read snapshot is consistent"; temporal-views "Uncommitted
// writes are invisible".
#[test]
fn read_snapshot_is_consistent() {
    let (_d, p) = tmp();
    let db = Arc::new(open(&p));
    db.transact(TxOptions::default(), |tx| {
        tx.assert(v("a"), v("status"), Value::Int(0), Valid::ALWAYS)
            .map(|_| ())
    })
    .unwrap();
    let (a, status) = (id(&db, &v("a")), id(&db, &v("status")));
    let done = Arc::new(AtomicBool::new(false));
    let reader = {
        let (db, done) = (db.clone(), done.clone());
        thread::spawn(move || {
            let mut n = 0;
            while !done.load(Ordering::SeqCst) || n < 50 {
                let rows = db.now().triples(Some(a), Some(status), None).unwrap();
                assert_eq!(rows.len(), 1, "saw a mix: {rows:?}");
                n += 1;
            }
            n
        })
    };
    for i in 1..150 {
        db.transact(TxOptions::default(), |tx| {
            tx.retract_matching(Some(a), Some(status), None)?;
            tx.assert(a, status, Value::Int(i), Valid::ALWAYS)
                .map(|_| ())
        })
        .unwrap();
    }
    done.store(true, Ordering::SeqCst);
    assert!(reader.join().unwrap() >= 50);
}

// transactions "Single writer serialisation" — concurrent transactions.
#[test]
fn concurrent_transactions_from_many_threads() {
    let (_d, p) = tmp();
    let db = Arc::new(open(&p));
    let hs: Vec<_> = (0..8)
        .map(|k| {
            let db = db.clone();
            thread::spawn(move || {
                for i in 0..100 {
                    db.transact(TxOptions::default(), |tx| {
                        tx.create(v(&format!("t{k}")), v("n"), Value::Int(i), Valid::ALWAYS)
                            .map(|_| ())
                    })
                    .unwrap();
                }
            })
        })
        .collect();
    for h in hs {
        h.join().unwrap();
    }
    let rows = db.read_sql("SELECT t, instant FROM tx ORDER BY t").unwrap();
    assert_eq!(rows.len(), 800);
    for (i, r) in rows.iter().enumerate() {
        assert_eq!(r[0].as_i64(), Some(i as i64 + 1));
    }
    assert!(rows.windows(2).all(|w| w[0][1].as_i64() < w[1][1].as_i64()));
}

// transactions "Read-modify-write is atomic" — concurrent upsert.
#[test]
fn concurrent_upsert_of_a_unique_value() {
    let (_d, p) = tmp();
    let db = Arc::new(open(&p));
    db.transact(TxOptions::default(), |tx| {
        tx.assert(
            v("email"),
            Value::iri(vocab::SYS_UNIQUE),
            Value::Bool(true),
            Valid::ALWAYS,
        )
        .map(|_| ())
    })
    .unwrap();
    for round in 0..10 {
        let val = lit(&format!("user{round}@example.org"));
        let barrier = Arc::new(Barrier::new(2));
        let hs: Vec<_> = (0..2)
            .map(|_| {
                let (db, barrier, val) = (db.clone(), barrier.clone(), val.clone());
                thread::spawn(move || {
                    barrier.wait();
                    let mut node = None;
                    db.transact(TxOptions::default(), |tx| {
                        node = Some(tx.upsert(v("email"), val.clone())?);
                        Ok(())
                    })
                    .unwrap();
                    node.unwrap()
                })
            })
            .collect();
        let nodes: Vec<ObjectId> = hs.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(nodes[0], nodes[1]);
        let o = id(&db, &val);
        assert_eq!(
            db.now()
                .triples(None, Some(id(&db, &v("email"))), Some(o))
                .unwrap()
                .len(),
            1
        );
    }
}

// transactions "Two handles on one file".
#[test]
fn two_handles_on_one_file() {
    let (_d, p) = tmp();
    let a = Arc::new(open(&p));
    let b = Arc::new(open(&p));
    let hs: Vec<_> = [a.clone(), b.clone()]
        .into_iter()
        .enumerate()
        .map(|(k, db)| {
            thread::spawn(move || {
                for i in 0..50 {
                    db.transact(TxOptions::default(), |tx| {
                        tx.create(
                            v(&format!("h{k}")),
                            v("n"),
                            lit(&format!("value number {i}")),
                            Valid::ALWAYS,
                        )
                        .map(|_| ())
                    })
                    .unwrap();
                }
            })
        })
        .collect();
    for h in hs {
        h.join().unwrap();
    }
    let ts: Vec<i64> = a
        .read_sql("SELECT t FROM tx ORDER BY t")
        .unwrap()
        .iter()
        .map(|r| r[0].as_i64().unwrap())
        .collect();
    assert_eq!(ts, (1..=100).collect::<Vec<_>>());
    let dups = a
        .read_sql("SELECT tag, lex FROM term GROUP BY 1, 2 HAVING count(*) > 1")
        .unwrap();
    assert!(dups.is_empty());
    assert_eq!(b.history().triples(None, None, None).unwrap().len(), 100);
}

// transactions "Re-entrant writes are rejected".
#[test]
fn nested_transaction_is_rejected() {
    let (_d, p) = tmp();
    let db = open(&p);
    let rep = db
        .transact(TxOptions::default(), |tx| {
            let inner = db.transact(TxOptions::default(), |_| Ok(()));
            assert!(matches!(inner, Err(Error::Reentrant)));
            let inner = db.with(|_| Ok(()), |_| Ok(()));
            assert!(matches!(inner, Err(Error::Reentrant)));
            tx.assert(v("a"), v("p"), v("b"), Valid::ALWAYS).map(|_| ())
        })
        .unwrap();
    assert_eq!(rep.t, TxId(1));
    let r = db.transact(TxOptions::default(), |_| {
        db.transact(TxOptions::default(), |_| Ok(()))?;
        Ok(())
    });
    assert!(matches!(r, Err(Error::Reentrant)));
    assert_eq!(
        db.transact(TxOptions::default(), |_| Ok(())).unwrap().t,
        TxId(2)
    );
}

// speculative-transactions through `Db::with`: uncommitted state on the writer
// view, other readers see only committed state, writers wait, nested writes fail.
#[test]
fn speculation_through_the_facade() {
    let (_d, p) = tmp();
    let db = Arc::new(open(&p));
    db.transact(TxOptions::default(), |tx| {
        tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::between(0, 10))
            .map(|_| ())
    })
    .unwrap();
    let (alice, works) = (id(&db, &v("alice")), id(&db, &v("worksAt")));
    let writer_done = Arc::new(AtomicBool::new(false));
    let mut writer = None;
    let employers = db
        .with(
            |tx| {
                tx.assert(v("alice"), v("worksAt"), v("globex"), Valid::ALWAYS)
                    .map(|_| ())
            },
            |view| {
                let rows = view.triples(Some(alice), Some(works), None)?;
                let names: Vec<Value> = rows
                    .iter()
                    .map(|t| view.decode(t.o))
                    .collect::<Result<_>>()?;
                // narrowed by a valid-at filter
                assert_eq!(
                    view.valid_at(50)
                        .triples(Some(alice), Some(works), None)?
                        .len(),
                    1
                );
                // another reader sees only committed state
                let other = {
                    let db = db.clone();
                    thread::spawn(move || db.now().triples(None, None, None).unwrap().len())
                };
                assert_eq!(other.join().unwrap(), 1);
                // a writer on another thread waits for the speculation
                let (db2, flag) = (db.clone(), writer_done.clone());
                writer = Some(thread::spawn(move || {
                    db2.transact(TxOptions::default(), |tx| {
                        assert!(tx.lookup(&v("globex"))?.is_none());
                        Ok(())
                    })
                    .unwrap();
                    flag.store(true, Ordering::SeqCst);
                }));
                thread::sleep(Duration::from_millis(100));
                assert!(!writer_done.load(Ordering::SeqCst));
                // nested writes fail
                assert!(matches!(
                    db.transact(TxOptions::default(), |_| Ok(())),
                    Err(Error::Reentrant)
                ));
                Ok(names)
            },
        )
        .unwrap();
    assert!(employers.contains(&v("globex")));
    writer.unwrap().join().unwrap();
    assert!(writer_done.load(Ordering::SeqCst));
    assert_eq!(db.now().triples(None, None, None).unwrap().len(), 1);
    assert_eq!(db.events_since(0).unwrap().len(), 1);
}

// supersede "Patch rules" — patch names the subject (bindings path).
#[test]
fn patch_from_fields() {
    for bad in ["s", "p"] {
        let r = Patch::from_fields([(bad, PatchField::Value(v("x")))]);
        assert!(matches!(r, Err(Error::InvalidPatch(_))), "{bad}");
    }
    assert!(matches!(
        Patch::from_fields([("o", PatchField::Time(Some(1)))]),
        Err(Error::InvalidPatch(_))
    ));
    let patch = Patch::from_fields([
        ("o", PatchField::Value(lit("Alice"))),
        ("v_to", PatchField::Time(None)),
    ])
    .unwrap();
    assert_eq!(patch.o, Some(lit("Alice")));
    assert_eq!((patch.v_from, patch.v_to), (None, Some(None)));
    let (_d, p) = tmp();
    let db = open(&p);
    let rep = db
        .transact(TxOptions::default(), |tx| {
            tx.assert(v("alice"), v("name"), lit("Alcie"), Valid::between(1, 5))
                .map(|_| ())
        })
        .unwrap();
    let e = rep.asserted[0];
    let mut new = None;
    db.transact(TxOptions::default(), |tx| {
        new = Some(tx.supersede(e, patch)?);
        Ok(())
    })
    .unwrap();
    let row = db.now().triples(None, None, None).unwrap();
    let root = row.iter().find(|t| Some(t.eid) == new).unwrap();
    assert_eq!(db.now().decode(root.o).unwrap(), lit("Alice"));
    assert_eq!((root.v_from, root.v_to), (Some(1), None));
}

// Views, values, events and statistics through the facade.
#[test]
fn views_values_and_optimize() {
    let (_d, p) = tmp();
    let db = Db::open(
        &p,
        OpenOptions {
            optimize_every: 3,
            ..OpenOptions::default()
        },
    )
    .unwrap();
    for i in 0..6 {
        db.transact(TxOptions::default(), |tx| {
            tx.assert(v("a"), v("n"), Value::Int(i), Valid::ALWAYS)?;
            tx.set_volatile(v("a"), v("seen"), Value::Int(i))
        })
        .unwrap();
    }
    assert_eq!(db.optimize_runs().unwrap(), 2);
    let (a, seen) = (id(&db, &v("a")), id(&db, &v("seen")));
    assert_eq!(
        db.now().values(a, seen).unwrap(),
        vec![id(&db, &Value::Int(5))]
    );
    assert!(db.as_of(TimeRef::Tx(6)).values(a, seen).unwrap().is_empty());
    assert_eq!(
        db.as_of(TimeRef::Tx(3))
            .triples(Some(a), None, None)
            .unwrap()
            .len(),
        3
    );
    assert_eq!(db.history().triples(None, None, None).unwrap().len(), 6);
    assert_eq!(db.events_since(4).unwrap().len(), 2);
    assert_eq!(db.now().encode(&v("never")).unwrap(), None);
    db.optimize().unwrap();
    assert!(!db
        .read_sql("SELECT * FROM sqlite_stat1")
        .unwrap()
        .is_empty());
    let dry = db
        .transact(
            TxOptions {
                dry_run: true,
                ..TxOptions::default()
            },
            |tx| {
                tx.assert(v("b"), v("n"), Value::Int(1), Valid::ALWAYS)
                    .map(|_| ())
            },
        )
        .unwrap();
    assert_eq!(dry.t, TxId(7));
    assert_eq!(
        db.transact(TxOptions::default(), |_| Ok(())).unwrap().t,
        TxId(7)
    );
}
