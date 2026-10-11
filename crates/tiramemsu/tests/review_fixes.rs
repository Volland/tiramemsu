//! Regression tests for the fixes of the 2026-10-10 review (`docs/project-review.md`):
//! batched cascade loading (M1), the release-signalled bounded writer wait (M3) and
//! LFTJ inputs charged to the byte budget (M4). The term-cache policy (M2) is tested
//! in `tm-core` (`term::tests`).
#![cfg(feature = "sparql")]

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use tiramemsu::*;

fn v(s: &str) -> Value {
    Value::iri(format!("urn:tiramemsu:v:{s}"))
}

/// More layers than one `IN` chunk (500), nested two deep.
const NOTES: i64 = 1_200;

// @lat: [[tests#Review Fixes#Batched Cascade Keeps Its Order]]
#[test]
fn a_batched_cascade_replays_layers_in_their_original_order() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("m.db"), OpenOptions::default()).unwrap();
    let mut root = None;
    db.transact(TxOptions::default(), |tx| {
        let r = tx.create(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?;
        for i in 0..NOTES {
            let note = tx.create(r, v("note"), Value::Int(i), Valid::ALWAYS)?;
            if i % 100 == 0 {
                tx.create(note, v("checked"), Value::Int(i), Valid::ALWAYS)?;
            }
        }
        root = Some(r);
        Ok(())
    })
    .unwrap();
    let root = root.unwrap();
    let mut new_root = None;
    db.transact(TxOptions::default(), |tx| {
        new_root = Some(tx.supersede(root, Patch::object(v("globex")))?);
        Ok(())
    })
    .unwrap();
    let new_root = new_root.unwrap();
    let now = db.now();
    let note = now.encode(&v("note")).unwrap().unwrap();
    let checked = now.encode(&v("checked")).unwrap().unwrap();
    // every note moved to the new root, and new eids follow the old note order
    let mut moved = now.triples(Some(new_root.oid()), Some(note), None).unwrap();
    assert_eq!(moved.len() as i64, NOTES);
    moved.sort_by_key(|t| t.eid);
    let values: Vec<i64> = moved
        .iter()
        .map(|t| match now.decode(t.o).unwrap() {
            Value::Int(i) => i,
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(values, (0..NOTES).collect::<Vec<_>>());
    // the second level followed its notes
    assert_eq!(now.triples(None, Some(checked), None).unwrap().len(), 12);
    // and nothing of the old cascade is live
    assert!(now
        .triples(Some(root.oid()), None, None)
        .unwrap()
        .is_empty());
}

/// Holds the writer for `hold` on another thread; returns once it is held.
fn hold_writer(db: &Arc<Db>, hold: Duration) -> thread::JoinHandle<()> {
    let (held_tx, held_rx) = std::sync::mpsc::channel();
    let db = Arc::clone(db);
    let h = thread::spawn(move || {
        db.transact(TxOptions::default(), |tx| {
            tx.assert(v("holder"), v("p"), Value::Int(1), Valid::ALWAYS)?;
            held_tx.send(()).unwrap();
            thread::sleep(hold);
            Ok(())
        })
        .unwrap();
    });
    held_rx.recv().unwrap();
    h
}

// @lat: [[tests#Review Fixes#Bounded Writer Wait Wakes And Cancels]]
#[test]
fn a_bounded_writer_wait_wakes_on_release_and_stops_on_cancel() {
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(Db::open(dir.path().join("m.db"), OpenOptions::default()).unwrap());
    let budget = QueryBudget {
        timeout: Some(Duration::from_secs(10)),
        ..QueryBudget::default()
    };

    // a budgeted write waits for the holder and then commits
    let holder = hold_writer(&db, Duration::from_millis(150));
    let start = Instant::now();
    db.transact_budgeted(TxOptions::default(), &budget, |tx| {
        tx.assert(v("waiter"), v("p"), Value::Int(2), Valid::ALWAYS)
            .map(|_| ())
    })
    .unwrap();
    let waited = start.elapsed();
    holder.join().unwrap();
    assert!(
        waited < Duration::from_secs(5),
        "woke {waited:?} after a 150 ms hold"
    );
    assert_eq!(db.now().triples(None, None, None).unwrap().len(), 2);

    // cancelling the waiter ends the wait long before the holder lets go
    let holder = hold_writer(&db, Duration::from_millis(1_500));
    let cancel = CancelToken::new();
    let budget = QueryBudget {
        cancel: Some(cancel.clone()),
        ..QueryBudget::default()
    };
    let canceller = {
        let cancel = cancel.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            cancel.cancel();
        })
    };
    let start = Instant::now();
    let r = db.transact_budgeted(TxOptions::default(), &budget, |tx| {
        tx.assert(v("cancelled"), v("p"), Value::Int(3), Valid::ALWAYS)
            .map(|_| ())
    });
    let waited = start.elapsed();
    canceller.join().unwrap();
    holder.join().unwrap();
    assert!(matches!(r, Err(Error::Cancelled)), "{r:?}");
    assert!(
        waited < Duration::from_millis(1_000),
        "cancel took {waited:?}"
    );
}

fn triangle_db(path: &std::path::Path, opts: OpenOptions) -> Db {
    let db = Db::open(path, opts).unwrap();
    let node = |i: u64| Value::iri(format!("urn:tiramemsu:v:n{i}"));
    db.transact(TxOptions::default(), |tx| {
        for i in 0..300u64 {
            let (a, b, c) = (i, 1_000 + i, 2_000 + i);
            tx.create(node(a), v("k"), node(b), Valid::ALWAYS)?;
            tx.create(node(b), v("k"), node(c), Valid::ALWAYS)?;
            tx.create(node(c), v("k"), node(a), Valid::ALWAYS)?;
        }
        Ok(())
    })
    .unwrap();
    db
}

// @lat: [[tests#Review Fixes#LFTJ Inputs Count Against The Byte Budget]]
#[test]
fn lftj_inputs_are_charged_to_the_byte_budget() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tri.db");
    drop(triangle_db(&path, OpenOptions::default()));
    let q = "SELECT (COUNT(*) AS ?n) WHERE { ?a v:k ?b . ?b v:k ?c . ?c v:k ?a }";
    let budget = QueryBudget {
        max_bytes: Some(4_096),
        ..QueryBudget::default()
    };
    // the SQL route decodes one small row: within the budget
    let sql_db = Db::open(&path, OpenOptions::default()).unwrap();
    sql_db.now().with_budget(&budget).sparql(q).unwrap();
    drop(sql_db);
    // the native route materialises 900 rows per pattern first: over the budget
    let lftj = OpenOptions {
        planner: PlannerOptions {
            lftj: LftjConfig {
                enabled: true,
                min_rows_estimate: 0,
            },
        },
        ..OpenOptions::default()
    };
    let native = Db::open(&path, lftj).unwrap();
    let r = native.now().with_budget(&budget).sparql(q);
    assert!(
        matches!(
            r,
            Err(Error::ResultLimitExceeded {
                limit: ResultLimit::Bytes(4_096)
            })
        ),
        "{r:?}"
    );
    // without a byte budget the native route answers
    match native.now().sparql(q).unwrap() {
        SparqlResult::Solutions(s) => assert_eq!(s.rows.len(), 1),
        other => panic!("{other:?}"),
    }
}
