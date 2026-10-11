//! Benchmarks for the hot spots of the 2026-10-10 review (`docs/project-review.md`,
//! findings M1–M3 and L3). No targets are asserted; each group measures one path that
//! a 0.4.1 fix changes, so a before/after run shows its effect.
//!
//! - `term_cache`: lookups of a warm term set right after a transaction that interns
//!   more new terms than the cache holds (M2);
//! - `bounded_writer`: two threads writing with a budget, so one waits for the writer
//!   mutex on every transaction (M3);
//! - `as_of_instant`: a two-hop SPARQL query on an instant-based versus a
//!   transaction-based as-of view (L3);
//! - supersede cascade loading is in `core.rs` (`supersede/cascade_set`, M1).

use std::sync::Arc;
use std::time::{Duration, Instant};

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use tiramemsu::*;

fn v(s: &str) -> Value {
    Value::iri(format!("urn:tiramemsu:v:{s}"))
}

const HOT: usize = 1_000;

fn term_cache(c: &mut Criterion) {
    let mut g = c.benchmark_group("term_cache");
    g.sample_size(10);
    let dir = tempfile::tempdir().unwrap();
    let opts = OpenOptions {
        term_cache_capacity: 2_048,
        ..OpenOptions::default()
    };
    let db = Db::open(dir.path().join("terms.db"), opts).unwrap();
    let hot = |tx: &mut Tx<'_>| -> Result<()> {
        for i in 0..HOT {
            tx.assert(
                v(&format!("h{i}")),
                v("p"),
                v(&format!("o{i}")),
                Valid::ALWAYS,
            )?;
        }
        Ok(())
    };
    db.transact(TxOptions::default(), hot).unwrap();
    let mut round = 0u64;
    g.bench_function(BenchmarkId::new("warm_after_overflow", HOT), |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                round += 1;
                // 5 000 fresh terms: more than twice the capacity
                db.transact(TxOptions::default(), |tx| {
                    for i in 0..2_500 {
                        tx.assert(
                            v(&format!("c{round}_{i}")),
                            v("p"),
                            v(&format!("d{round}_{i}")),
                            Valid::ALWAYS,
                        )?;
                    }
                    Ok(())
                })
                .unwrap();
                let start = Instant::now();
                // idempotent: every fact exists, so this is term lookups plus seeks
                db.transact(TxOptions::default(), hot).unwrap();
                total += start.elapsed();
            }
            total
        })
    });
    g.finish();
}

fn bounded_writer(c: &mut Criterion) {
    let mut g = c.benchmark_group("bounded_writer");
    g.sample_size(10);
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(Db::open(dir.path().join("lock.db"), OpenOptions::default()).unwrap());
    const PER_THREAD: usize = 200;
    let mut round = 0u64;
    g.bench_function(BenchmarkId::new("two_threads", PER_THREAD * 2), |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                round += 1;
                let start = Instant::now();
                let workers: Vec<_> = (0..2)
                    .map(|w| {
                        let db = Arc::clone(&db);
                        std::thread::spawn(move || {
                            let budget = QueryBudget {
                                timeout: Some(Duration::from_secs(30)),
                                ..QueryBudget::default()
                            };
                            for i in 0..PER_THREAD {
                                db.transact_budgeted(TxOptions::default(), &budget, |tx| {
                                    tx.assert(
                                        v(&format!("w{w}_{round}_{i}")),
                                        v("p"),
                                        Value::Int(i as i64),
                                        Valid::ALWAYS,
                                    )
                                    .map(|_| ())
                                })
                                .unwrap();
                            }
                        })
                    })
                    .collect();
                for w in workers {
                    w.join().unwrap();
                }
                total += start.elapsed();
            }
            total
        })
    });
    g.finish();
}

fn as_of_instant(c: &mut Criterion) {
    let mut g = c.benchmark_group("as_of_instant");
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("instant.db"), OpenOptions::default()).unwrap();
    let nodes = 5_000;
    // one transaction per 100 nodes, so the tx table has some depth
    for chunk in (0..nodes).collect::<Vec<_>>().chunks(100) {
        db.transact(TxOptions::default(), |tx| {
            for &i in chunk {
                tx.assert(
                    v(&format!("n{i}")),
                    v("next"),
                    v(&format!("n{}", (i + 1) % nodes)),
                    Valid::ALWAYS,
                )?;
                tx.assert(
                    v(&format!("n{i}")),
                    v("label"),
                    Value::str(format!("l{i}")),
                    Valid::ALWAYS,
                )?;
            }
            Ok(())
        })
        .unwrap();
    }
    let last = db
        .events_since(0)
        .unwrap()
        .iter()
        .map(|e| e.t.0)
        .max()
        .unwrap();
    let ms = db
        .read_sql(&format!("SELECT instant FROM tx WHERE t = {last}"))
        .unwrap();
    let ms = ms
        .first()
        .and_then(|r| r.first())
        .and_then(|c| c.as_i64())
        .expect("instant of the last transaction");
    let q = "SELECT ?l WHERE { <urn:tiramemsu:v:n42> v:next ?a . ?a v:next ?b . ?b v:label ?l }";
    for (name, view) in [
        ("tx", db.as_of(TimeRef::Tx(last))),
        ("instant", db.as_of(TimeRef::Instant(ms))),
    ] {
        let rows = view.sparql(q).unwrap();
        assert!(
            matches!(&rows, SparqlResult::Solutions(s) if s.rows.len() == 1),
            "{name}"
        );
        g.bench_function(BenchmarkId::new("two_hop", name), |b| {
            b.iter(|| view.sparql(q).unwrap())
        });
    }
    g.finish();
}

criterion_group!(benches, term_cache, bounded_writer, as_of_instant);
criterion_main!(benches);
