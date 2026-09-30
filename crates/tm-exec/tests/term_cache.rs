//! Spec `sql-execution` "Bounded term cache" and cold/warm agreement.

mod common;

use common::*;
use tiramemsu::ir::builder::IrBuilder;
use tiramemsu::*;

fn b() -> IrBuilder {
    IrBuilder::sparql()
}

fn fill(t: &TestDb, n: usize) {
    t.tx(|tx| {
        for i in 0..n {
            tx.assert(
                v(&format!("s{i}")),
                v("name"),
                s(&format!("a long string number {i}")),
                Valid::ALWAYS,
            )?;
        }
        Ok(())
    });
}

fn names() -> IrQuery {
    b().query(b().triple("?s", "v:name", "?n").project(&["n"]))
}

// sql-execution "Warm cache avoids dictionary reads"
#[test]
fn warm_cache_reads_no_dictionary() {
    let t = TestDb::open(OpenOptions {
        readers: 1,
        ..OpenOptions::default()
    });
    fill(&t, 100);
    let cold = run(&t.db.now(), &names());
    assert!(cold.stats.dictionary_reads >= 100, "{:?}", cold.stats);
    let warm = run(&t.db.now(), &names());
    assert_eq!(warm.stats.dictionary_reads, 0);
    assert!(warm.stats.cache_hits >= 100);
    assert_eq!(sorted(&cold), sorted(&warm));
}

// sql-execution "Capacity is respected"
#[test]
fn capacity_is_respected() {
    let t = TestDb::open(OpenOptions {
        term_cache_capacity: 10,
        ..OpenOptions::default()
    });
    fill(&t, 50);
    let r = run(&t.db.now(), &names());
    assert_eq!(r.len(), 50);
    assert!(t.db.term_cache_len() <= 10, "{}", t.db.term_cache_len());
    let mut got: Vec<String> = rows(&r).into_iter().map(|r| r[0].clone()).collect();
    got.sort();
    let mut want: Vec<String> = (0..50)
        .map(|i| format!("a long string number {i}"))
        .collect();
    want.sort();
    assert_eq!(got, want);
}

// sql-execution "Cold and warm cache agree"
#[test]
fn cold_and_warm_agree() {
    let t = TestDb::new();
    fill(&t, 20);
    t.tx(|tx| {
        tx.assert(
            v("x"),
            v("typed"),
            Value::literal(
                "P3D",
                Some("http://www.w3.org/2001/XMLSchema#duration"),
                None,
            ),
            Valid::ALWAYS,
        )
        .map(|_| ())
    });
    let q = b().query(b().triple("?s", "?p", "?o"));
    let cold = run(&t.db.now(), &q);
    drop(t.db);
    let fresh = Db::open(&t.path, OpenOptions::default()).unwrap();
    let cold2 = run(&fresh.now(), &q);
    let warm = run(&fresh.now(), &q);
    assert_eq!(sorted(&cold), sorted(&cold2));
    assert_eq!(sorted(&cold2), sorted(&warm));
    assert_eq!(warm.stats.dictionary_reads, 0);
}

// sql-execution "Speculation does not pollute the cache"
#[test]
fn speculation_does_not_pollute_cache() {
    let t = TestDb::new();
    fill(&t, 3);
    let spec = Value::iri("urn:tiramemsu:v:onlyInSpeculation");
    let q = b().query(b().triple("?s", "v:likes", "?o"));
    let before = t.db.term_cache_len();
    let n =
        t.db.with(
            |tx| {
                tx.assert(v("alice"), v("likes"), spec.clone(), Valid::ALWAYS)
                    .map(|_| ())
            },
            |view| {
                let r = view.execute_ir(&q, &Params::new())?;
                assert_eq!(r.get(0, "o"), Some(&spec));
                assert!(r.stats.dictionary_reads > 0);
                Ok(r.len())
            },
        )
        .unwrap();
    assert_eq!(n, 1);
    assert_eq!(t.db.term_cache_len(), before, "shared cache untouched");
    // after rollback the IRI is gone: the same query short-circuits
    let r = run(
        &t.db.now(),
        &b().query(b().triple("?s", "v:likes", spec.clone())),
    );
    assert!(r.is_empty() && !r.stats.sql_executed);
}
