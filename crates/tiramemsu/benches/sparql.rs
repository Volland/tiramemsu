//! SPARQL variants of the point and 2-hop latency benchmarks
//! (`lat.md/roadmap#Benchmarks`, "Point and 2-hop latency"), run through
//! `View::sparql` for the now, asOf and validAt views. No targets are asserted.
//!
//! The graph has `STATEMENTS` (default 100 000; `SPARQL_BENCH_STATEMENTS=1000000`
//! or `10000000` for the roadmap sizes) statements: `n{i} v:next n{i+1}` chains
//! and `n{i} v:label "l{i}"`, with a valid-time interval on every edge.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use tiramemsu::*;

fn v(s: &str) -> Value {
    Value::iri(format!("urn:tiramemsu:v:{s}"))
}

fn statements() -> usize {
    std::env::var("SPARQL_BENCH_STATEMENTS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(100_000)
}

fn build(dir: &std::path::Path, statements: usize) -> Db {
    let db = Db::open(dir.join("sparql.db"), OpenOptions::default()).unwrap();
    let nodes = statements / 2;
    for chunk in (0..nodes).collect::<Vec<_>>().chunks(10_000) {
        db.transact(TxOptions::default(), |tx| {
            for &i in chunk {
                tx.assert(
                    v(&format!("n{i}")),
                    v("next"),
                    v(&format!("n{}", (i + 1) % nodes)),
                    Valid::from(1_000),
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
    db
}

fn sparql_latency(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let n = statements();
    let db = build(dir.path(), n);
    let last = db
        .events_since(0)
        .unwrap()
        .iter()
        .map(|e| e.t.0)
        .max()
        .unwrap_or(1);
    let mid = n as i64 / 4;
    let point = format!("SELECT ?l WHERE {{ <urn:tiramemsu:v:n{mid}> v:label ?l }}");
    let two_hop = format!(
        "SELECT ?l WHERE {{ <urn:tiramemsu:v:n{mid}> v:next ?a . ?a v:next ?b . ?b v:label ?l }}"
    );
    let views: Vec<(&str, View<'_>)> = vec![
        ("now", db.now()),
        ("asOf", db.as_of(TimeRef::Tx(last))),
        ("validAt", db.now().valid_at(5_000)),
    ];
    let mut g = c.benchmark_group(format!("sparql_latency_{n}"));
    g.sample_size(20);
    for (name, view) in &views {
        for (shape, q) in [("point", &point), ("two_hop", &two_hop)] {
            g.bench_with_input(BenchmarkId::new(shape, name), q, |b, q| {
                b.iter(|| view.sparql(q).unwrap())
            });
        }
    }
    g.finish();
}

criterion_group!(benches, sparql_latency);
criterion_main!(benches);
