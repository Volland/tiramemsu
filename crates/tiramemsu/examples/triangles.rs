//! The triangle benchmark through tiramemsu itself: SQL route versus the native
//! cyclic-join operator (LFTJ) on the same database file (`bench/triangles/`).
//!
//! For every graph it first counts triangles `a→b→c→a` on both routes and stops
//! with an error if the counts differ; only then does it time each route (best
//! of a few runs). Run with
//! `cargo run --release -p tiramemsu --example triangles -- [small|full]`.
#![allow(dead_code)]

use std::time::{Duration, Instant};

use tiramemsu::ir::builder::IrBuilder;
use tiramemsu::ir::{Agg, Op};
use tiramemsu::*;

/// One measured graph.
#[derive(Clone, Debug)]
pub struct Row {
    /// Graph name.
    pub name: String,
    /// Distinct edges.
    pub edges: usize,
    /// Triangles (equal on both routes).
    pub triangles: i64,
    /// Best SQL time.
    pub sql: Duration,
    /// Best native time.
    pub native: Duration,
}

/// A small deterministic generator (no dependency).
pub struct Lcg(pub u64);

impl Lcg {
    /// Next value in `0..n`.
    pub fn below(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) % n
    }
}

/// `n` nodes with out-degree 5.
pub fn uniform(n: u64) -> Vec<(u64, u64)> {
    (1..=n)
        .flat_map(|i| (1..=5).map(move |k| (i, (i * 7919 + k) % n + 1)))
        .collect()
}

/// `hubs × spokes` both ways, plus `noise` random edges among `4 × spokes` nodes.
pub fn skewed(hubs: u64, spokes: u64, noise: u64, rng: &mut Lcg) -> Vec<(u64, u64)> {
    let mut e = Vec::new();
    for h in 1..=hubs {
        for s in 0..spokes {
            let s = 1_000_000 + s;
            e.push((h, s));
            e.push((s, h));
        }
    }
    let span = 4 * spokes.max(1);
    for _ in 0..noise {
        e.push((2_000_000 + rng.below(span), 2_000_000 + rng.below(span)));
    }
    e
}

/// Three layers of `n`: all of A→B and B→C, and three random C→A edges per C node.
pub fn layered(n: u64, rng: &mut Lcg) -> Vec<(u64, u64)> {
    let (a, b, c) = (1u64, 10_000_000u64, 20_000_000u64);
    let mut e = Vec::new();
    for i in 0..n {
        for j in 0..n {
            e.push((a + i, b + j));
            e.push((b + i, c + j));
        }
        for _ in 0..3 {
            e.push((c + i, a + rng.below(n)));
        }
    }
    e
}

fn node(i: u64) -> Value {
    Value::iri(format!("urn:tiramemsu:v:n{i}"))
}

fn lftj_options() -> OpenOptions {
    OpenOptions {
        planner: PlannerOptions {
            lftj: LftjConfig {
                enabled: true,
                min_rows_estimate: 0,
            },
        },
        ..OpenOptions::default()
    }
}

fn count_query() -> IrQuery {
    let b = IrBuilder::sparql();
    let tri = Op::join(vec![
        b.triple("?a", "v:k", "?b"),
        b.triple("?b", "v:k", "?c"),
        b.triple("?c", "v:k", "?a"),
    ]);
    b.query(tri.aggregate(&[], vec![Agg::count_star("n")]))
}

fn count(db: &Db, q: &IrQuery) -> Result<(i64, Duration)> {
    let t = Instant::now();
    let r = db.now().execute_ir(q, &Params::new())?;
    let d = t.elapsed();
    match r.get(0, "n") {
        Some(Value::Int(n)) => Ok((*n, d)),
        other => Err(Error::InvalidQuery {
            msg: format!("count returned {other:?}"),
        }),
    }
}

/// Loads `edges` into a fresh database in `dir`, checks that both routes count
/// the same triangles (and that the native handle really routes natively), then
/// times each route `reps` times and keeps the best.
pub fn compare(
    dir: &std::path::Path,
    name: &str,
    mut edges: Vec<(u64, u64)>,
    reps: usize,
) -> std::result::Result<Row, String> {
    edges.sort_unstable();
    edges.dedup();
    let path = dir.join(format!(
        "{}.db",
        name.replace(|c: char| !c.is_alphanumeric(), "_")
    ));
    let _ = std::fs::remove_file(&path);
    let e = |e: Error| format!("{name}: {e}");
    let sql_db = Db::open(&path, OpenOptions::default()).map_err(e)?;
    let k = Value::iri("urn:tiramemsu:v:k");
    for chunk in edges.chunks(50_000) {
        sql_db
            .transact(TxOptions::default(), |tx| {
                for (a, b) in chunk {
                    tx.create(node(*a), k.clone(), node(*b), Valid::ALWAYS)?;
                }
                Ok(())
            })
            .map_err(e)?;
    }
    sql_db.optimize().map_err(e)?;
    let native_db = Db::open(&path, lftj_options()).map_err(e)?;
    let q = count_query();
    let routed = native_db
        .now()
        .explain_ir(&q, &Params::new())
        .map_err(e)?
        .regions
        .iter()
        .any(|r| r.kind == RegionKind::NativeLftj);
    if !routed {
        return Err(format!("{name}: the native handle did not route to LFTJ"));
    }
    // correctness gate: identical counts before any timing
    let (want, _) = count(&sql_db, &q).map_err(e)?;
    let (got, _) = count(&native_db, &q).map_err(e)?;
    if want != got {
        return Err(format!("{name}: SQL counts {want} triangles, LFTJ {got}"));
    }
    let best = |db: &Db| -> std::result::Result<Duration, String> {
        let mut best = Duration::MAX;
        for _ in 0..reps.max(1) {
            let (n, d) = count(db, &q).map_err(e)?;
            if n != want {
                return Err(format!("{name}: count changed between runs"));
            }
            best = best.min(d);
        }
        Ok(best)
    };
    let sql = best(&sql_db)?;
    let native = best(&native_db)?;
    Ok(Row {
        name: name.to_string(),
        edges: edges.len(),
        triangles: want,
        sql,
        native,
    })
}

/// The graphs of one scale: `small` runs in seconds, `full` matches `bench.py`.
pub fn graphs(full: bool) -> Vec<(String, Vec<(u64, u64)>)> {
    let mut rng = Lcg(3);
    let mut g = Vec::new();
    if full {
        g.push(("uniform, out-degree 5".to_string(), uniform(100_000)));
        g.push((
            "300 hubs × 1 500 spokes, both ways, plus noise".to_string(),
            skewed(300, 1_500, 200_000, &mut rng),
        ));
        for n in [150, 300, 600] {
            g.push((
                format!("3 layers of {n}, 3 closing edges each"),
                layered(n, &mut rng),
            ));
        }
    } else {
        g.push(("uniform, out-degree 5".to_string(), uniform(5_000)));
        g.push((
            "30 hubs × 150 spokes, both ways, plus noise".to_string(),
            skewed(30, 150, 5_000, &mut rng),
        ));
        for n in [40, 80] {
            g.push((
                format!("3 layers of {n}, 3 closing edges each"),
                layered(n, &mut rng),
            ));
        }
    }
    g
}

fn ms(d: Duration) -> String {
    format!("{:.1} ms", d.as_secs_f64() * 1e3)
}

fn main() {
    let full = std::env::args().nth(1).as_deref() == Some("full");
    let dir = std::env::temp_dir().join("tiramemsu-triangles");
    let _ = std::fs::create_dir_all(&dir);
    println!("| Graph | Edges | Triangles | SQL | LFTJ | SQL / LFTJ |");
    println!("|---|---|---|---|---|---|");
    for (name, edges) in graphs(full) {
        match compare(&dir, &name, edges, 3) {
            Ok(r) => println!(
                "| {} | {} | {} | {} | {} | {:.1}× |",
                r.name,
                r.edges,
                r.triangles,
                ms(r.sql),
                ms(r.native),
                r.sql.as_secs_f64() / r.native.as_secs_f64().max(1e-9)
            ),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
    }
}
