//! The regression gate of `move-sqlite-onto-store`: the same writes on two SQLite
//! files, one through the SQL engine (`tm_core::Store`) and one through the
//! trait engine on SQLite (`tm_store::Engine` over `SqlBackend`). It prints the
//! median latency of each operation on both and the slowdown, and exits with
//! status 1 when any median is more than 5 % slower on the trait engine.
//!
//! `cargo run --release -p tiramemsu --example write_gate -- [samples]`

use std::sync::Arc;
use std::time::Instant;

use tiramemsu::*;
use tm_core::sqlstore::SqlBackend;
use tm_core::{HostOptions, StoreOptions};
use tm_rusqlite::RusqliteHost;
use tm_store::engine::{Engine, EngineOptions};

fn v(s: &str) -> Value {
    Value::iri(format!("urn:tiramemsu:v:{s}"))
}

fn median(mut x: Vec<f64>) -> f64 {
    x.sort_by(|a, b| a.partial_cmp(b).unwrap());
    x[x.len() / 2]
}

/// One operation, written once for both engines (their `Tx` types differ but
/// have the same methods).
macro_rules! ops {
    ($tx:ident, $op:expr, $i:expr, $root:expr) => {{
        let i: usize = $i;
        match $op {
            "assert 1" => {
                $tx.assert(
                    v(&format!("s{i}")),
                    v("p"),
                    Value::Int(i as i64),
                    Valid::ALWAYS,
                )?;
            }
            "assert 100" => {
                for k in 0..100 {
                    $tx.assert(
                        v(&format!("b{i}_{k}")),
                        v("p"),
                        Value::Int(k),
                        Valid::ALWAYS,
                    )?;
                }
            }
            "replace (cardinality one)" => {
                $tx.assert(v("k"), v("score"), Value::Int(i as i64), Valid::ALWAYS)?;
            }
            "annotated assert + retract" => {
                let e = $tx
                    .assert(v(&format!("f{i}")), v("about"), v("x"), Valid::ALWAYS)?
                    .eid();
                for n in ["source", "confidence", "method"] {
                    $tx.assert(e, v(n), Value::Int(1), Valid::ALWAYS)?;
                }
                $tx.retract(e)?;
            }
            "supersede, 100 layers (dry run)" => {
                $tx.supersede($root, Patch::object(Value::Int(i as i64 + 1_000_000)))?;
            }
            _ => unreachable!(),
        }
        Ok(())
    }};
}

const OPS: [&str; 5] = [
    "assert 1",
    "assert 100",
    "replace (cardinality one)",
    "annotated assert + retract",
    "supersede, 100 layers (dry run)",
];

fn main() {
    let samples: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(300);
    let dir = tempfile::tempdir().unwrap();
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let host = RusqliteHost::new();
    let mut sql = tm_core::Store::open(
        &host,
        &dir.path().join("sql.db"),
        StoreOptions {
            clock: clock.clone(),
            ..StoreOptions::default()
        },
    )
    .unwrap();
    let exec = tm_core::storage::open(&host, &dir.path().join("trait.db"), &HostOptions::default())
        .unwrap();
    let mut tr = Engine::open(
        Arc::new(SqlBackend::new(exec)),
        EngineOptions {
            clock,
            ..EngineOptions::default()
        },
    )
    .unwrap();
    // setup: the cardinality-one predicate and a statement with 100 layers
    let setup = |opts: TxOptions| opts;
    let mut roots = (None, None);
    sql.transact(setup(TxOptions::default()), |tx| {
        tx.assert(
            v("score"),
            Value::iri(vocab::SYS_CARDINALITY),
            Value::iri(vocab::SYS_ONE),
            Valid::ALWAYS,
        )?;
        let r = tx.create(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?;
        for k in 0..99 {
            tx.create(r, v("note"), Value::Int(k), Valid::ALWAYS)?;
        }
        roots.0 = Some(r);
        Ok(())
    })
    .unwrap();
    tr.transact(setup(TxOptions::default()), |tx| {
        tx.assert(
            v("score"),
            Value::iri(vocab::SYS_CARDINALITY),
            Value::iri(vocab::SYS_ONE),
            Valid::ALWAYS,
        )?;
        let r = tx.create(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?;
        for k in 0..99 {
            tx.create(r, v("note"), Value::Int(k), Valid::ALWAYS)?;
        }
        roots.1 = Some(r);
        Ok(())
    })
    .unwrap();
    let (r0, r1) = (roots.0.unwrap(), roots.1.unwrap());

    println!("| operation | SQL engine | trait engine on SQLite | change |");
    println!("|---|---|---|---|");
    let mut failed = false;
    for op in OPS {
        let dry = op.contains("dry run");
        let opts = TxOptions {
            dry_run: dry,
            ..TxOptions::default()
        };
        let (mut a, mut b) = (Vec::new(), Vec::new());
        // interleaved rounds, so drift affects both engines alike
        for i in 0..samples {
            let t = Instant::now();
            sql.transact(opts, |tx| ops!(tx, op, i, r0)).unwrap();
            a.push(t.elapsed().as_secs_f64() * 1e6);
            let t = Instant::now();
            tr.transact(opts, |tx| ops!(tx, op, i, r1)).unwrap();
            b.push(t.elapsed().as_secs_f64() * 1e6);
        }
        let (ma, mb) = (median(a), median(b));
        let change = (mb / ma - 1.0) * 100.0;
        failed |= change > 5.0;
        println!("| {op} | {ma:.0} µs | {mb:.0} µs | {change:+.1} % |");
    }
    if failed {
        eprintln!("gate: at least one median is more than 5 % slower on the trait engine");
        std::process::exit(1);
    }
}
