//! The redb backend through the facade (OpenSpec change `add-redb-backend`):
//! detection, creation, durability, the layout, statistics, text recall,
//! concurrent readers, conversion and compaction.
#![cfg(feature = "redb")]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};
use std::time::Duration;

use tiramemsu::*;

fn v(s: &str) -> Value {
    Value::iri(format!("urn:tiramemsu:v:{s}"))
}

fn tmp(name: &str) -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join(name);
    (d, p)
}

fn redb(path: &Path) -> Db {
    Db::open(
        path,
        OpenOptions {
            backend: Backend::Redb,
            ..OpenOptions::default()
        },
    )
    .unwrap()
}

fn clock() -> Arc<ManualClock> {
    Arc::new(ManualClock::new(1_000))
}

/// A history with layers, a supersede, retractions, a graph, volatile state,
/// an embedding and burned ids.
fn history(db: &Db) -> Vec<TxReport> {
    let model = v("model");
    let mut out = Vec::new();
    out.push(
        db.transact(TxOptions::default(), |tx| {
            let f = tx
                .assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?
                .eid();
            tx.assert(
                f,
                v("source"),
                Value::str("the chat of monday"),
                Valid::ALWAYS,
            )?;
            tx.assert(
                v("alice"),
                v("name"),
                Value::str("Alice Liddell"),
                Valid::between(10, 20),
            )?;
            tx.add_to_graph(f, v("work"), AssertOpts::default())?;
            tx.define_embedding_model(model.clone(), &EmbeddingModel::new(2))?;
            tx.set_embedding(f, model.clone(), &[0.6, 0.8])?;
            tx.set_volatile(v("alice"), v("mood"), Value::str("calm"))?;
            Ok(())
        })
        .unwrap(),
    );
    let f = out[0].asserted[0];
    out.push(
        db.transact(TxOptions::default(), |tx| {
            tx.supersede(f, Patch::object(v("globex")))?;
            Ok(())
        })
        .unwrap(),
    );
    out.push(
        db.transact(
            TxOptions {
                dry_run: true,
                ..TxOptions::default()
            },
            |tx| {
                tx.assert(v("bob"), v("knows"), v("alice"), Valid::ALWAYS)
                    .map(|_| ())
            },
        )
        .unwrap(),
    );
    out.push(
        db.transact(TxOptions::default(), |tx| {
            tx.assert(v("bob"), v("knows"), v("alice"), Valid::ALWAYS)?;
            let name = tx.lookup(&Value::str("Alice Liddell"))?.unwrap();
            let alice = tx.encode(v("alice"))?;
            let p = tx.encode(v("name"))?;
            tx.retract_matching(Some(alice), Some(p), Some(name))?;
            Ok(())
        })
        .unwrap(),
    );
    out
}

fn rows(db: &Db) -> Vec<Triple> {
    db.history().triples(None, None, None).unwrap()
}

// --- backend-selection -------------------------------------------------------

// @tg: verifies:: [[openspec:backend-selection#Detection by file header#Open a redb file without saying so]] {since: "2026-10-27", level: integration}
#[test]
fn open_a_redb_file_without_saying_so() {
    let (_d, p) = tmp("m.redb");
    drop(redb(&p));
    let db = Db::open(&p, OpenOptions::default()).unwrap();
    assert_eq!(db.backend(), Backend::Redb);
}

// @tg: verifies:: [[openspec:backend-selection#Detection by file header#Open a SQLite file without saying so]] {since: "2026-10-27", level: integration}
#[test]
fn open_a_sqlite_file_without_saying_so() {
    let (_d, p) = tmp("m.db");
    let db = Db::create(&p, Backend::Sqlite, OpenOptions::default()).unwrap();
    assert_eq!(db.backend(), Backend::Sqlite);
    history(&db);
    drop(db);
    let db = Db::open(&p, OpenOptions::default()).unwrap();
    assert_eq!(db.backend(), Backend::Sqlite);
    assert!(!rows(&db).is_empty());
}

// @tg: verifies:: [[openspec:backend-selection#Detection by file header#Unknown content]] {since: "2026-10-27", level: integration}
#[test]
fn unknown_content() {
    let (_d, p) = tmp("notes.txt");
    std::fs::write(&p, b"just some notes").unwrap();
    let r = Db::open(&p, OpenOptions::default());
    assert!(matches!(r, Err(Error::UnknownFormat { .. })), "{r:?}");
    assert_eq!(std::fs::read(&p).unwrap(), b"just some notes");
}

// @tg: verifies:: [[openspec:backend-selection#Explicit backend choice#New file on redb]] {since: "2026-10-27", level: integration}
#[test]
fn new_file_on_redb() {
    let (_d, p) = tmp("new.redb");
    let db = redb(&p);
    assert_eq!(db.backend(), Backend::Redb);
    drop(db);
    assert_eq!(&std::fs::read(&p).unwrap()[..4], b"redb");
}

// @tg: verifies:: [[openspec:backend-selection#Explicit backend choice#Mismatch]] {since: "2026-10-27", level: integration}
#[test]
fn mismatch() {
    let (_d, p) = tmp("m.db");
    drop(Db::create(&p, Backend::Sqlite, OpenOptions::default()).unwrap());
    let r = Db::open(
        &p,
        OpenOptions {
            backend: Backend::Redb,
            ..OpenOptions::default()
        },
    );
    match r {
        Err(Error::BackendMismatch {
            expected, found, ..
        }) => {
            assert_eq!((expected.as_str(), found.as_str()), ("redb", "sqlite"))
        }
        other => panic!("{other:?}"),
    }
}

// @tg: verifies:: [[openspec:backend-selection#Creating a database#Create refuses an existing file]] {since: "2026-10-27", level: integration}
#[test]
fn create_refuses_an_existing_file() {
    let (_d, p) = tmp("m.redb");
    drop(Db::create(&p, Backend::Redb, OpenOptions::default()).unwrap());
    let before = std::fs::read(&p).unwrap();
    let r = Db::create(&p, Backend::Redb, OpenOptions::default());
    assert!(matches!(r, Err(Error::AlreadyExists(_))), "{r:?}");
    assert_eq!(std::fs::read(&p).unwrap(), before);
}

// --- redb-storage ---------------------------------------------------------------

// @tg: verifies:: [[openspec:redb-storage#Same logical database#Same history on both backends]] {since: "2026-10-27", level: integration}
#[test]
fn same_history_on_both_backends() {
    let (_d, a) = tmp("a.db");
    let (_e, b) = tmp("b.redb");
    let opts = |backend| OpenOptions {
        backend,
        clock: clock(),
        ..OpenOptions::default()
    };
    let sqlite = Db::open(&a, opts(Backend::Sqlite)).unwrap();
    let kv = Db::open(&b, opts(Backend::Redb)).unwrap();
    assert_eq!(history(&sqlite), history(&kv));
    assert_eq!(rows(&sqlite), rows(&kv));
    assert_eq!(sqlite.events_since(0).unwrap(), kv.events_since(0).unwrap());
    let g = sqlite.now().encode(&v("work")).unwrap().unwrap();
    assert_eq!(kv.now().encode(&v("work")).unwrap(), Some(g));
    assert_eq!(sqlite.now().graphs().unwrap(), kv.now().graphs().unwrap());
}

// @tg: verifies:: [[openspec:redb-storage#Permutation layout#Retraction leaves the live tables]] {since: "2026-10-27", level: integration}
#[test]
fn retraction_leaves_the_live_tables() {
    let (_d, p) = tmp("m.redb");
    let db = redb(&p);
    let r = db
        .transact(TxOptions::default(), |tx| {
            tx.assert(v("a"), v("p"), v("b"), Valid::ALWAYS).map(|_| ())
        })
        .unwrap();
    let e = r.asserted[0].oid().raw();
    let store = db.redb_store().unwrap();
    assert_eq!(store.presence(e).unwrap(), (vec![true; 6], vec![true; 3]));
    db.transact(TxOptions::default(), |tx| {
        tx.retract(r.asserted[0]).map(|_| ())
    })
    .unwrap();
    assert_eq!(store.presence(e).unwrap(), (vec![false; 6], vec![true; 3]));
    let gone = &db.history().triples(None, None, None).unwrap()[0];
    assert_eq!(gone.ret_kind, Some(RetKind::Explicit));
}

// @tg: verifies:: [[openspec:redb-storage#Exact predicate statistics#Statistics follow writes]] {since: "2026-10-27", level: integration}
#[test]
fn statistics_follow_writes() {
    let (_d, path) = tmp("m.redb");
    let db = redb(&path);
    db.transact(TxOptions::default(), |tx| {
        tx.assert(v("a"), v("p"), Value::Int(1), Valid::ALWAYS)?;
        tx.assert(v("a"), v("p"), Value::Int(2), Valid::ALWAYS)?;
        tx.assert(v("b"), v("p"), Value::Int(1), Valid::ALWAYS)?;
        Ok(())
    })
    .unwrap();
    let p = db.now().encode(&v("p")).unwrap().unwrap().raw();
    let store = db.redb_store().unwrap().clone();
    assert_eq!(store.predicate_stats(p).unwrap(), (3, 2, 2));
    db.transact(TxOptions::default(), |tx| {
        let a = tx.encode(v("a"))?;
        let pp = tx.encode(v("p"))?;
        let two = tx.encode(Value::Int(2))?;
        tx.retract_matching(Some(a), Some(pp), Some(two))?;
        Ok(())
    })
    .unwrap();
    assert_eq!(store.predicate_stats(p).unwrap(), (2, 2, 1));
}

// @tg: verifies:: [[openspec:redb-storage#Exact predicate statistics#Failed transactions leave statistics unchanged]] {since: "2026-10-27", level: integration}
#[test]
fn failed_transactions_leave_statistics_unchanged() {
    let (_d, path) = tmp("m.redb");
    let db = redb(&path);
    db.transact(TxOptions::default(), |tx| {
        tx.assert(v("a"), v("p"), Value::Int(1), Valid::ALWAYS)
            .map(|_| ())
    })
    .unwrap();
    let p = db.now().encode(&v("p")).unwrap().unwrap().raw();
    let store = db.redb_store().unwrap().clone();
    let before = store.predicate_stats(p).unwrap();
    let r = db.transact(TxOptions::default(), |tx| {
        tx.assert(v("b"), v("p"), Value::Int(9), Valid::ALWAYS)?;
        tx.assert(v("c"), v("p"), Value::Int(8), Valid::ALWAYS)?;
        Err(Error::custom("give up"))
    });
    assert!(r.is_err());
    assert_eq!(store.predicate_stats(p).unwrap(), before);
}

// @tg: verifies:: [[openspec:redb-storage#Text index parity#Same recall on both backends]] {since: "2026-10-27", level: integration}
#[test]
fn same_recall_on_both_backends() {
    let (_d, a) = tmp("a.db");
    let (_e, b) = tmp("b.redb");
    let opts = |backend| OpenOptions {
        backend,
        clock: clock(),
        text_index: true,
        ..OpenOptions::default()
    };
    let sqlite = Db::open(&a, opts(Backend::Sqlite)).unwrap();
    let kv = Db::open(&b, opts(Backend::Redb)).unwrap();
    let notes = [
        "met Alice at the Lisbon offsite",
        "Lisbon trip booked for the spring offsite",
        "Café crème in Lisbon, again",
        "Bob prefers the Porto office",
        "the offsite agenda: memory, graphs, time",
    ];
    for db in [&sqlite, &kv] {
        db.transact(TxOptions::default(), |tx| {
            for (i, n) in notes.iter().enumerate() {
                tx.assert(
                    v(&format!("n{i}")),
                    v("note"),
                    Value::str(*n),
                    Valid::ALWAYS,
                )?;
            }
            Ok(())
        })
        .unwrap();
    }
    for (q, mode) in [
        ("lisbon", TextMode::All),
        ("lisbon offsite", TextMode::All),
        ("porto offsite", TextMode::Any),
        ("lisbon offsite", TextMode::Phrase),
        ("cafe creme", TextMode::All),
        ("off*", TextMode::All),
    ] {
        let tq = TextQuery {
            mode,
            ..TextQuery::new(q)
        };
        let x = sqlite.now().text_search(&tq).unwrap();
        let y = kv.now().text_search(&tq).unwrap();
        assert_eq!(x.len(), y.len(), "{q:?}");
        for (h, k) in x.iter().zip(&y) {
            assert_eq!((h.eid, h.rank, &h.text), (k.eid, k.rank, &k.text), "{q:?}");
            assert!(
                (h.lexical - k.lexical).abs() <= 1e-9 * h.lexical.abs().max(1.0),
                "{q:?}"
            );
        }
    }
}

// @tg: verifies:: [[openspec:redb-storage#Text index parity#Speculation rolls the index back]] {since: "2026-10-27", level: integration}
#[test]
fn speculation_rolls_the_index_back() {
    let (_d, p) = tmp("m.redb");
    let db = Db::open(
        &p,
        OpenOptions {
            backend: Backend::Redb,
            text_index: true,
            ..OpenOptions::default()
        },
    )
    .unwrap();
    let inside = db
        .with(
            |tx| {
                tx.assert(
                    v("a"),
                    v("note"),
                    Value::str("quantum walrus"),
                    Valid::ALWAYS,
                )?;
                Ok(())
            },
            |view| Ok(view.text_search(&TextQuery::new("walrus"))?.len()),
        )
        .unwrap();
    assert_eq!(inside, 1);
    assert!(db
        .now()
        .text_search(&TextQuery::new("walrus"))
        .unwrap()
        .is_empty());
}

// @tg: verifies:: [[openspec:redb-storage#Readers without a pool#Many readers during a write]] {since: "2026-10-27", level: integration}
#[test]
fn many_readers_during_a_write() {
    let (_d, p) = tmp("m.redb");
    let db = Arc::new(redb(&p));
    db.transact(TxOptions::default(), |tx| {
        for i in 0..100 {
            tx.assert(v(&format!("s{i}")), v("p"), Value::Int(i), Valid::ALWAYS)?;
        }
        Ok(())
    })
    .unwrap();
    let started = Arc::new(Barrier::new(2));
    let done = Arc::new(Barrier::new(2));
    let writer = {
        let (db, started, done) = (db.clone(), started.clone(), done.clone());
        std::thread::spawn(move || {
            db.transact(TxOptions::default(), |tx| {
                tx.assert(v("late"), v("p"), Value::Int(-1), Valid::ALWAYS)?;
                started.wait();
                done.wait();
                Ok(())
            })
            .unwrap();
        })
    };
    started.wait();
    let readers: Vec<_> = (0..32)
        .map(|_| {
            let db = db.clone();
            std::thread::spawn(move || db.now().triples(None, None, None).unwrap().len())
        })
        .collect();
    for r in readers {
        assert_eq!(
            r.join().unwrap(),
            100,
            "readers see the last committed state"
        );
    }
    done.wait();
    writer.join().unwrap();
    assert_eq!(db.now().triples(None, None, None).unwrap().len(), 101);
}

// @tg: verifies:: [[openspec:redb-storage#Lossless conversion#Round trip]] {since: "2026-10-27", level: integration}
#[test]
fn round_trip() {
    let (d, a) = tmp("a.db");
    let b = d.path().join("b.redb");
    let c = d.path().join("c.db");
    let opts = OpenOptions {
        clock: clock(),
        text_index: true,
        ..OpenOptions::default()
    };
    let original = Db::open(&a, opts.clone()).unwrap();
    history(&original);
    let before = rows(&original);
    let events = original.events_since(0).unwrap();
    drop(original);
    convert(&a, &b, Backend::Redb).unwrap();
    convert(&b, &c, Backend::Sqlite).unwrap();
    let mut next = Vec::new();
    for p in [&a, &b, &c] {
        let db = Db::open(p, opts.clone()).unwrap();
        assert_eq!(rows(&db), before, "{p:?}");
        assert_eq!(db.events_since(0).unwrap(), events, "{p:?}");
        // supersede retracted the source layer and replayed it onto the new eid
        assert_eq!(
            db.now()
                .text_search(&TextQuery::new("monday"))
                .unwrap()
                .len(),
            1,
            "{p:?}"
        );
        assert_eq!(
            db.history()
                .text_search(&TextQuery::new("monday"))
                .unwrap()
                .len(),
            2
        );
        let r = db
            .transact(TxOptions::default(), |tx| {
                tx.assert(v("carol"), v("p"), Value::Int(1), Valid::ALWAYS)
                    .map(|_| ())
            })
            .unwrap();
        next.push((r.t, r.asserted));
    }
    assert!(next.windows(2).all(|w| w[0] == w[1]), "{next:?}");
}

// @tg: verifies:: [[openspec:redb-storage#Lossless conversion#Existing destination]] {since: "2026-10-27", level: integration}
#[test]
fn existing_destination() {
    let (d, a) = tmp("a.db");
    let b = d.path().join("b.redb");
    drop(Db::open(&a, OpenOptions::default()).unwrap());
    std::fs::write(&b, b"keep me").unwrap();
    assert!(matches!(
        convert(&a, &b, Backend::Redb),
        Err(Error::AlreadyExists(_))
    ));
    assert_eq!(std::fs::read(&b).unwrap(), b"keep me");
}

// @tg: verifies:: [[openspec:redb-storage#Compaction#Compaction after churn]] {since: "2026-10-27", level: integration}
#[test]
fn compaction_after_churn() {
    let (_d, p) = tmp("m.redb");
    let db = redb(&p);
    for i in 0..200 {
        db.transact(TxOptions::default(), |tx| {
            let e = tx
                .assert(v("k"), v("value"), Value::Int(i), Valid::ALWAYS)?
                .eid();
            if i % 2 == 0 {
                tx.retract(e)?;
            }
            Ok(())
        })
        .unwrap();
    }
    let before = rows(&db);
    db.sync().unwrap();
    let size = std::fs::metadata(&p).unwrap().len();
    db.compact().unwrap();
    assert!(std::fs::metadata(&p).unwrap().len() <= size);
    assert_eq!(rows(&db), before);
    drop(db);
    assert_eq!(rows(&redb(&p)), before);
}

// --- durability-modes -------------------------------------------------------------

/// Runs the named test in a child process with `CHILD` set, and waits for it.
fn in_child(test: &str, dir: &Path) {
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([test, "--exact", "--nocapture", "--test-threads=1"])
        .env("TM_DURABILITY_CHILD", dir)
        .status()
        .unwrap();
    assert!(!status.success(), "the child aborts after committing");
}

fn child_dir() -> Option<PathBuf> {
    std::env::var_os("TM_DURABILITY_CHILD").map(PathBuf::from)
}

fn open_with(path: &Path, durability: Durability) -> Db {
    Db::open(
        path,
        OpenOptions {
            backend: Backend::Redb,
            durability,
            flush_interval: Duration::from_secs(3600),
            flush_commits: 1_000_000,
            ..OpenOptions::default()
        },
    )
    .unwrap()
}

fn commit_n(db: &Db, n: i64) {
    for i in 0..n {
        db.transact(TxOptions::default(), |tx| {
            tx.assert(v("k"), v("n"), Value::Int(i), Valid::ALWAYS)
                .map(|_| ())
        })
        .unwrap();
    }
}

// @tg: verifies:: [[openspec:durability-modes#Durability option#Immediate survives a crash]] {since: "2026-10-27", level: integration}
#[test]
fn immediate_survives_a_crash() {
    if let Some(dir) = child_dir() {
        let db = open_with(&dir.join("m.redb"), Durability::Immediate);
        commit_n(&db, 3);
        std::process::abort();
    }
    let d = tempfile::tempdir().unwrap();
    in_child("immediate_survives_a_crash", d.path());
    let db = redb(&d.path().join("m.redb"));
    assert_eq!(db.now().triples(None, None, None).unwrap().len(), 3);
}

// @tg: verifies:: [[openspec:durability-modes#Durability option#Normal is consistent after a crash]] {since: "2026-10-27", level: integration}
#[test]
fn normal_is_consistent_after_a_crash() {
    if let Some(dir) = child_dir() {
        let db = open_with(&dir.join("m.redb"), Durability::Normal);
        commit_n(&db, 20);
        std::process::abort();
    }
    let d = tempfile::tempdir().unwrap();
    in_child("normal_is_consistent_after_a_crash", d.path());
    let db = redb(&d.path().join("m.redb"));
    let events = db.events_since(0).unwrap();
    let ts: Vec<u64> = events.iter().map(|e| e.t.0).collect();
    assert!(
        ts.iter().enumerate().all(|(i, t)| *t == i as u64 + 1),
        "gap-free prefix: {ts:?}"
    );
    // the file still takes writes, with gap-free numbering
    let r = db
        .transact(TxOptions::default(), |tx| {
            tx.assert(v("after"), v("n"), Value::Int(0), Valid::ALWAYS)
                .map(|_| ())
        })
        .unwrap();
    assert_eq!(r.t.0, ts.len() as u64 + 1);
}

// @tg: verifies:: [[openspec:durability-modes#Durability option#Sync makes commits durable]] {since: "2026-10-27", level: integration}
#[test]
fn sync_makes_commits_durable() {
    if let Some(dir) = child_dir() {
        let db = open_with(&dir.join("m.redb"), Durability::None);
        commit_n(&db, 5);
        db.sync().unwrap();
        std::process::abort();
    }
    let d = tempfile::tempdir().unwrap();
    in_child("sync_makes_commits_durable", d.path());
    let db = redb(&d.path().join("m.redb"));
    assert_eq!(db.now().triples(None, None, None).unwrap().len(), 5);
}
