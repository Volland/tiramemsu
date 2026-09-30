//! Path benchmarks (`lat.md/roadmap#Benchmarks`, "Paths"): shortest-path latency
//! between random bound pairs, 3-hop trails (`p{1,3}` and the relationship wildcard),
//! reachability over a chain and a small-world graph, all-shortest paths on a grid,
//! and the `rarray` chunk sizes. No targets are asserted.
//!
//! `PATH_BENCH_STATEMENTS` sets the size of the power-law graph (default 100 000;
//! `1000000` is the roadmap size, `10000000` the large one).

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use tiramemsu::*;
use tm_exec::{PathEngine, PathOptions, PathRequest};
use tm_rusqlite::RusqliteExec;

fn v(s: &str) -> Value {
    Value::iri(format!("urn:tiramemsu:v:{s}"))
}

fn statements() -> usize {
    std::env::var("PATH_BENCH_STATEMENTS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(100_000)
}

/// A tiny deterministic generator (xorshift).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    /// A skewed pick: small indices are hubs.
    fn hub(&mut self, n: usize) -> usize {
        let u = (self.next() % 1_000_000) as f64 / 1_000_000.0;
        ((u * u * u) * n as f64) as usize % n
    }
}

fn commit(db: &Db, edges: impl Iterator<Item = (String, &'static str, String)>) {
    let all: Vec<_> = edges.collect();
    for chunk in all.chunks(10_000) {
        db.transact(TxOptions::default(), |tx| {
            for (a, p, b) in chunk {
                tx.assert(v(a), v(p), v(b), Valid::from(1_000))?;
            }
            Ok(())
        })
        .unwrap();
    }
}

fn power_law(db: &Db, statements: usize) -> usize {
    let nodes = (statements / 5).max(10);
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    commit(
        db,
        (0..statements).map(|_| {
            (
                format!("p{}", rng.below(nodes)),
                "link",
                format!("p{}", rng.hub(nodes)),
            )
        }),
    );
    nodes
}

fn ids(db: &Db, prefix: &str, n: usize, count: usize) -> Vec<ObjectId> {
    let mut rng = Rng(42);
    let view = db.now();
    let mut out = Vec::new();
    while out.len() < count {
        if let Some(id) = view
            .encode(&v(&format!("{prefix}{}", rng.below(n))))
            .unwrap()
        {
            out.push(id);
        }
    }
    out
}

fn path_latency(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("path.db"), OpenOptions::default()).unwrap();
    let nodes = power_law(&db, statements());
    let last = db
        .events_since(0)
        .unwrap()
        .iter()
        .map(|e| e.t.0)
        .max()
        .unwrap();
    let starts = ids(&db, "p", nodes, 64);
    let ends = ids(&db, "p", nodes, 64);
    let views: Vec<(&str, View<'_>)> = vec![
        ("now", db.now()),
        ("asof", db.as_of(TimeRef::Tx(last))),
        ("valid", db.now().valid_at(2_000)),
    ];
    let mut g = c.benchmark_group("shortest");
    g.sample_size(10);
    for (name, view) in views.iter().take(2) {
        g.bench_with_input(
            BenchmarkId::new("any_shortest_pairs", name),
            view,
            |b, view| {
                let mut i = 0;
                b.iter(|| {
                    i += 1;
                    let rows = view
                        .path(starts[i % 64], "link+", PathMode::AnyShortest, 6)
                        .unwrap();
                    rows.iter().filter(|r| r.end == ends[i % 64]).count()
                })
            },
        );
    }
    g.finish();
    let mut g = c.benchmark_group("trail3");
    g.sample_size(10);
    for (name, view) in &views {
        for expr in ["link{1,3}", "sys:anyRelationship{1,3}"] {
            g.bench_with_input(BenchmarkId::new(expr, name), view, |b, view| {
                let mut i = 0;
                b.iter(|| {
                    i += 1;
                    view.path(starts[i % 64], expr, PathMode::Trail, 3)
                        .map(|r| r.len())
                        .unwrap_or(0)
                })
            });
        }
    }
    g.finish();
}

fn reach_and_grid(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("shapes.db"), OpenOptions::default()).unwrap();
    let n = statements().min(100_000);
    commit(
        &db,
        (0..n).map(|i| (format!("c{i}"), "next", format!("c{}", i + 1))),
    );
    // a small world: a ring plus random shortcuts
    let mut rng = Rng(7);
    let ring = (n / 2).max(10);
    commit(
        &db,
        (0..ring)
            .map(|i| (format!("w{i}"), "near", format!("w{}", (i + 1) % ring)))
            .chain((0..ring / 10).map(|_| {
                (
                    format!("w{}", rng.below(ring)),
                    "near",
                    format!("w{}", rng.below(ring)),
                )
            })),
    );
    // a 30 x 30 grid
    let side = 30;
    let mut grid = Vec::new();
    for x in 0..side {
        for y in 0..side {
            if x + 1 < side {
                grid.push((format!("g{x}_{y}"), "step", format!("g{}_{y}", x + 1)));
            }
            if y + 1 < side {
                grid.push((format!("g{x}_{y}"), "step", format!("g{x}_{}", y + 1)));
            }
        }
    }
    commit(&db, grid.into_iter());
    let view = db.now();
    let mut g = c.benchmark_group("reach");
    g.sample_size(10);
    let c0 = view.encode(&v("c0")).unwrap().unwrap();
    g.bench_function("chain_plus", |b| {
        b.iter(|| {
            view.path(c0, "next+", PathMode::Reachability, u32::MAX)
                .unwrap()
                .len()
        })
    });
    let w0 = view.encode(&v("w0")).unwrap().unwrap();
    g.bench_function("small_world_plus", |b| {
        b.iter(|| {
            view.path(w0, "near+", PathMode::Reachability, u32::MAX)
                .unwrap()
                .len()
        })
    });
    let g0 = view.encode(&v("g0_0")).unwrap().unwrap();
    g.bench_function("grid_all_shortest", |b| {
        b.iter(|| {
            let rows = view.path(g0, "step+", PathMode::AllShortest, 12).unwrap();
            rows.len()
        })
    });
    g.finish();
}

fn chunk_sizes(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("chunks.db");
    let db = Db::open(&path, OpenOptions::default()).unwrap();
    let nodes = power_law(&db, statements().min(200_000));
    let start = ids(&db, "p", nodes, 1)[0];
    let conn = rusqlite::Connection::open(&path).unwrap();
    tm_rusqlite::register::register_defaults(&conn).unwrap();
    let mut exec = RusqliteExec::from_connection(conn, Capabilities::default());
    let mut g = c.benchmark_group("rarray_chunk");
    g.sample_size(10);
    for batch in [64usize, 256, 1024] {
        let engine = PathEngine::new(PathOptions {
            batch,
            ..PathOptions::default()
        });
        g.bench_with_input(BenchmarkId::new("reach", batch), &batch, |b, _| {
            b.iter(|| {
                engine
                    .eval(
                        &mut exec,
                        &PathRequest {
                            start,
                            path: "link+",
                            mode: PathMode::Reachability,
                            max_hops: Some(4),
                            view: ViewSpec::NOW,
                            end: None,
                        },
                    )
                    .unwrap()
                    .len()
            })
        });
    }
    g.finish();
}

criterion_group!(benches, path_latency, reach_and_grid, chunk_sizes);
criterion_main!(benches);
