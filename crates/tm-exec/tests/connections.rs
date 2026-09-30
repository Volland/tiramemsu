//! Spec `sql-execution` "Connection selection" and "Typed errors".

mod common;

use std::sync::mpsc::channel;
use std::sync::Arc;
use std::thread;

use common::*;
use tiramemsu::ir::builder::IrBuilder;
use tiramemsu::ir::Op;
use tiramemsu::*;

fn b() -> IrBuilder {
    IrBuilder::sparql()
}

// sql-execution "Reader for ordinary queries"
#[test]
fn reader_runs_while_writer_holds_a_transaction() {
    let t = Arc::new(TestDb::new());
    t.tx(|tx| {
        tx.assert(v("alice"), v("likes"), v("coffee"), Valid::ALWAYS)
            .map(|_| ())
    });
    let (started_tx, started_rx) = channel();
    let (done_tx, done_rx) = channel::<()>();
    let t2 = t.clone();
    let writer = thread::spawn(move || {
        t2.db
            .transact(TxOptions::default(), |tx| {
                tx.assert(v("alice"), v("likes"), v("tea"), Valid::ALWAYS)?;
                started_tx.send(()).unwrap();
                done_rx.recv().unwrap();
                Ok(())
            })
            .unwrap();
    });
    started_rx.recv().unwrap();
    let q = b().query(b().triple("v:alice", "v:likes", "?x"));
    let r = run(&t.db.now(), &q);
    assert_eq!(rows(&r), expect(&[&["coffee"]]), "last committed state");
    done_tx.send(()).unwrap();
    writer.join().unwrap();
    assert_eq!(run(&t.db.now(), &q).len(), 2);
}

// sql-execution "Speculative view sees uncommitted state" / "Speculative constant encoding"
#[test]
fn speculative_view() {
    let t = TestDb::new();
    t.tx(|tx| {
        tx.assert(v("alice"), v("likes"), v("coffee"), Valid::ALWAYS)
            .map(|_| ())
    });
    let q = b().query(b().triple("v:alice", "v:likes", "?x"));
    let seen =
        t.db.with(
            |tx| {
                tx.assert(v("alice"), v("likes"), v("tea"), Valid::ALWAYS)
                    .map(|_| ())
            },
            |view| Ok(view.execute_ir(&q, &Params::new())?.column("x")),
        )
        .unwrap();
    assert!(seen.contains(&Some(v("tea"))));
    assert_eq!(run(&t.db.now(), &q).len(), 1, "tea is gone after `with`");
    // a constant created only by the speculation matches inside, short-circuits after
    let new_thing = v("newThing");
    let q = b().query(b().triple("?s", "?p", new_thing.clone()));
    let inside =
        t.db.with(
            |tx| {
                tx.assert(v("alice"), v("likes"), new_thing.clone(), Valid::ALWAYS)
                    .map(|_| ())
            },
            |view| view.execute_ir(&q, &Params::new()),
        )
        .unwrap();
    assert_eq!(inside.len(), 1);
    assert!(inside.stats.sql_executed);
    let after = run(&t.db.now(), &q);
    assert!(after.is_empty() && !after.stats.sql_executed);
}

// sql-execution "Error returns the connection"
#[test]
fn error_returns_the_connection() {
    let t = TestDb::open(OpenOptions {
        readers: 1,
        ..OpenOptions::default()
    });
    t.tx(|tx| tx.assert(v("a"), v("p"), v("b"), Valid::ALWAYS).map(|_| ()));
    // fails in prepare (no operator), and via a storage/planning error path
    let bad = b().query(Op::Path(tiramemsu::ir::PathPattern {
        start: "?a".into(),
        end: "?b".into(),
        path: tiramemsu::ir::PathExpr::iri("urn:p").star(),
        mode: tiramemsu::ir::PathMode::Reachability,
        max_hops: None,
        bind_path: None,
        view: tiramemsu::ir::View::NOW,
    }));
    for _ in 0..3 {
        assert!(matches!(
            t.db.now().execute_ir(&bad, &Params::new()),
            Err(Error::Unsupported { .. })
        ));
    }
    // an error raised inside the read transaction (a runtime SQL failure)
    let broken = b().query(
        b().triple("?s", "v:p", "?o")
            .filter(tiramemsu::ir::Expr::Func(
                tiramemsu::ir::Func::Regex,
                vec![
                    tiramemsu::ir::Expr::var("o"),
                    tiramemsu::ir::Expr::val(s("(")),
                ],
            )),
    );
    let _ = t.db.now().execute_ir(&broken, &Params::new());
    let ok = run(&t.db.now(), &b().query(b().triple("?s", "v:p", "?o")));
    assert_eq!(ok.len(), 1);
    assert_eq!(t.db.reader_count(), 1);
}
