//! Benchmark skeleton for M0 (`lat.md/roadmap#Benchmarks`). No targets are
//! asserted; criterion prints the results.
//!
//! - churn: N updates per key (N = 1, 10, 100, 1000), now vs as-of lookups;
//! - supersede cost as a function of the cascade-set size;
//! - executor-trait overhead against a direct `rusqlite` loop (design D-17).

use std::path::Path;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use tiramemsu::*;

fn v(s: &str) -> Value {
    Value::iri(format!("urn:tiramemsu:v:{s}"))
}

fn churned(path: &Path, keys: usize, updates: usize) -> Db {
    let db = Db::open(path, OpenOptions::default()).unwrap();
    db.transact(TxOptions::default(), |tx| {
        tx.assert(
            v("score"),
            Value::iri(vocab::SYS_CARDINALITY),
            Value::iri(vocab::SYS_ONE),
            Valid::ALWAYS,
        )
        .map(|_| ())
    })
    .unwrap();
    for u in 0..updates {
        db.transact(TxOptions::default(), |tx| {
            for k in 0..keys {
                tx.assert(
                    v(&format!("k{k}")),
                    v("score"),
                    Value::Int(u as i64),
                    Valid::ALWAYS,
                )?;
            }
            Ok(())
        })
        .unwrap();
    }
    db
}

fn churn(c: &mut Criterion) {
    let mut g = c.benchmark_group("churn");
    g.sample_size(10);
    for n in [1usize, 10, 100, 1000] {
        let dir = tempfile::tempdir().unwrap();
        let db = churned(&dir.path().join("churn.db"), 20, n);
        let p = db.now().encode(&v("score")).unwrap().unwrap();
        let s = db.now().encode(&v("k7")).unwrap().unwrap();
        let mid = (n as u64 / 2).max(1) + 1;
        g.bench_with_input(BenchmarkId::new("now", n), &n, |b, _| {
            b.iter(|| db.now().triples(Some(s), Some(p), None).unwrap())
        });
        g.bench_with_input(BenchmarkId::new("as_of", n), &n, |b, _| {
            b.iter(|| {
                db.as_of(TimeRef::Tx(mid))
                    .triples(Some(s), Some(p), None)
                    .unwrap()
            })
        });
    }
    g.finish();
}

fn supersede_cost(c: &mut Criterion) {
    let mut g = c.benchmark_group("supersede");
    g.sample_size(10);
    for size in [1usize, 10, 100, 1000] {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path().join("sup.db"), OpenOptions::default()).unwrap();
        let mut root = None;
        db.transact(TxOptions::default(), |tx| {
            let r = tx.create(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?;
            for i in 0..size.saturating_sub(1) {
                tx.create(r, v("note"), Value::Int(i as i64), Valid::ALWAYS)?;
            }
            root = Some(r);
            Ok(())
        })
        .unwrap();
        let root = root.unwrap();
        // a dry run measures the full supersede without growing the file
        g.bench_with_input(BenchmarkId::new("cascade_set", size), &size, |b, _| {
            b.iter(|| {
                db.transact(
                    TxOptions {
                        dry_run: true,
                        ..TxOptions::default()
                    },
                    |tx| tx.supersede(root, Patch::object(v("globex"))).map(|_| ()),
                )
                .unwrap()
            })
        });
    }
    g.finish();
}

fn executor_overhead(c: &mut Criterion) {
    let mut g = c.benchmark_group("executor_overhead");
    g.sample_size(20);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("exec.db");
    let db = churned(&path, 200, 1);
    let p = db.now().encode(&v("score")).unwrap().unwrap().raw();
    g.bench_function("trait_query", |b| {
        b.iter(|| {
            db.read_sql("SELECT count(*) FROM triple WHERE p = 0")
                .unwrap()
        })
    });
    let ids: Vec<i64> = db
        .now()
        .triples(None, None, None)
        .unwrap()
        .iter()
        .map(|t| t.s.raw())
        .collect();
    g.bench_function("trait_point_lookups", |b| {
        b.iter(|| {
            for s in &ids {
                db.now()
                    .triples(
                        Some(ObjectId::from_raw(*s)),
                        Some(ObjectId::from_raw(p)),
                        None,
                    )
                    .unwrap();
            }
        })
    });
    let conn = rusqlite::Connection::open(&path).unwrap();
    g.bench_function("direct_point_lookups", |b| {
        b.iter(|| {
            let mut st = conn
                .prepare_cached(
                    "SELECT eid, s, p, o, t_add, t_ret, v_from, v_to, ret_kind FROM triple \
                     WHERE s = ?1 AND p = ?2 AND t_ret IS NULL ORDER BY eid",
                )
                .unwrap();
            for s in &ids {
                let rows: Vec<i64> = st
                    .query_map([*s, p], |r| r.get::<_, i64>(0))
                    .unwrap()
                    .map(Result::unwrap)
                    .collect();
                std::hint::black_box(rows);
            }
        })
    });
    g.finish();
}

criterion_group!(benches, churn, supersede_cost, executor_overhead);
criterion_main!(benches);
