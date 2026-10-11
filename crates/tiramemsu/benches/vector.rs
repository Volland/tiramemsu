//! Vector recall latency (`lat.md/query#Vector Recall`): exact search and the
//! sign-bit prefilter over `VECTOR_BENCH_STATEMENTS` embedded statements (default
//! 10 000) at 384 and 768 dimensions, f32 and f16. No targets are asserted.
//!
//! Vectors are pseudo-random (a fixed LCG), so the numbers measure the scan, not
//! retrieval quality.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use tiramemsu::*;

fn v(s: &str) -> Value {
    Value::iri(format!("urn:tiramemsu:v:{s}"))
}

fn statements() -> usize {
    std::env::var("VECTOR_BENCH_STATEMENTS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10_000)
}

struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> f32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 40) as f32 / (1u64 << 24) as f32) - 0.5
    }

    fn vector(&mut self, dims: usize) -> Vec<f32> {
        (0..dims).map(|_| self.next()).collect()
    }
}

fn build(dir: &std::path::Path, n: usize, dims: u32, encoding: VectorEncoding) -> Db {
    let db = Db::open(dir.join("vec.db"), OpenOptions::default()).unwrap();
    let model = v("bench-model");
    db.transact(TxOptions::default(), |tx| {
        tx.define_embedding_model(
            model.clone(),
            &EmbeddingModel {
                dimensions: dims,
                metric: Metric::Cosine,
                encoding,
            },
        )
        .map(|_| ())
    })
    .unwrap();
    let mut rng = Lcg(42);
    for chunk in (0..n).collect::<Vec<_>>().chunks(2_000) {
        db.transact(TxOptions::default(), |tx| {
            for &i in chunk {
                let e = tx
                    .assert(
                        v(&format!("m{i}")),
                        v("note"),
                        Value::Int(i as i64),
                        Valid::ALWAYS,
                    )?
                    .eid();
                tx.set_embedding(e, model.clone(), &rng.vector(dims as usize))?;
            }
            Ok(())
        })
        .unwrap();
    }
    db
}

fn vector_search(c: &mut Criterion) {
    let n = statements();
    for (dims, encoding) in [
        (384u32, VectorEncoding::F32),
        (768, VectorEncoding::F32),
        (768, VectorEncoding::F16),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let db = build(dir.path(), n, dims, encoding);
        let m = db.now().encode(&v("bench-model")).unwrap().unwrap();
        let query = Lcg(7).vector(dims as usize);
        let mut g = c.benchmark_group(format!("vector_{n}_{dims}_{}", encoding.name()));
        g.sample_size(20);
        let exact = VectorQuery::new(m, query.clone());
        assert_eq!(
            db.now().vector_search(&exact).unwrap().candidates as usize,
            n
        );
        g.bench_function(BenchmarkId::new("exact", 10), |b| {
            b.iter(|| db.now().vector_search(&exact).unwrap())
        });
        let pre = VectorQuery {
            prefilter: Some(200),
            ..exact.clone()
        };
        g.bench_function(BenchmarkId::new("prefilter", 200), |b| {
            b.iter(|| db.now().vector_search(&pre).unwrap())
        });
        g.finish();
    }
}

/// Hybrid recall over notes with text (text index) and 384-d vectors.
fn hybrid_recall(c: &mut Criterion) {
    let n = statements();
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(
        dir.path().join("recall.db"),
        OpenOptions {
            text_index: true,
            ..OpenOptions::default()
        },
    )
    .unwrap();
    let model = v("bench-model");
    db.transact(TxOptions::default(), |tx| {
        tx.define_embedding_model(model.clone(), &EmbeddingModel::new(384))
            .map(|_| ())
    })
    .unwrap();
    let mut rng = Lcg(11);
    for chunk in (0..n).collect::<Vec<_>>().chunks(2_000) {
        db.transact(TxOptions::default(), |tx| {
            for &i in chunk {
                let text = format!("note {i} topic{} place{}", i % 50, i % 97);
                let e = tx.create(v("trip"), v("note"), Value::str(text), Valid::ALWAYS)?;
                tx.set_embedding(e, model.clone(), &rng.vector(384))?;
            }
            Ok(())
        })
        .unwrap();
    }
    let m = db.now().encode(&model).unwrap().unwrap();
    let q = Lcg(3).vector(384);
    let mut g = c.benchmark_group(format!("recall_{n}_384"));
    g.sample_size(20);
    for (name, rq) in [
        (
            "text_and_vector",
            RecallQuery {
                text: Some(TextQuery::new("topic7")),
                vector: Some(VectorQuery::new(m, q.clone())),
                ..RecallQuery::default()
            },
        ),
        (
            "text_and_vector_expand2",
            RecallQuery {
                text: Some(TextQuery::new("topic7")),
                vector: Some(VectorQuery::new(m, q.clone())),
                expand: 2,
                ..RecallQuery::default()
            },
        ),
        (
            "text_only",
            RecallQuery {
                text: Some(TextQuery::new("topic7")),
                ..RecallQuery::default()
            },
        ),
    ] {
        assert!(!db.now().recall(&rq).unwrap().is_empty());
        g.bench_function(name, |b| b.iter(|| db.now().recall(&rq).unwrap()));
    }
    g.finish();
}

criterion_group!(benches, vector_search, hybrid_recall);
criterion_main!(benches);
