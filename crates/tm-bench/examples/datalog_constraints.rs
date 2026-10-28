//! Commit overhead of enforced Datalog constraints (OpenSpec change
//! `add-datalog-constraints`, task 4.4): a 100-statement transaction on a store of
//! `--size` `v:role` statements, with and without an enforced constraint over
//! `v:role`, on SQLite and redb.
//!
//! cargo run --release -p tm-bench --features datalog --example datalog_constraints -- [--size 1000000] [--reps 5]

use std::sync::Arc;
use std::time::Instant;

use tiramemsu::*;

fn v(s: impl std::fmt::Display) -> Value {
    Value::iri(format!("urn:tiramemsu:v:{s}"))
}

fn arg(name: &str) -> Option<String> {
    let a: Vec<String> = std::env::args().collect();
    a.iter()
        .position(|x| x == name)
        .and_then(|i| a.get(i + 1).cloned())
}

fn main() {
    let size: u64 = arg("--size")
        .and_then(|x| x.parse().ok())
        .unwrap_or(100_000);
    let reps: usize = arg("--reps").and_then(|x| x.parse().ok()).unwrap_or(5);
    let dir = std::env::temp_dir().join(format!("tm-bench-dlc-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    println!("| backend | store | enforced | ms per 100-statement commit | checks | seed rows |\n|---|---:|---|---:|---:|---:|");
    for backend in [Backend::Sqlite, Backend::Redb] {
        let db = Db::create(
            dir.join(format!("c.{}", backend.name())),
            backend,
            OpenOptions {
                clock: Arc::new(ManualClock::new(1_000)),
                ..OpenOptions::default()
            },
        )
        .expect("create");
        for chunk in 0..size.div_ceil(100_000) {
            db.transact(TxOptions::default(), |t| {
                for i in chunk * 100_000..((chunk + 1) * 100_000).min(size) {
                    t.assert(
                        v(format!("p{i}")),
                        v("role"),
                        v(format!("r{}", i % 50)),
                        Valid::ALWAYS,
                    )?;
                }
                Ok(())
            })
            .expect("load");
        }
        let mut n = 0u64;
        let mut run = |db: &Db, enforced: bool| {
            let mut best = f64::MAX;
            let mut rep = None;
            for _ in 0..reps {
                let t = Instant::now();
                let r = db
                    .transact(TxOptions::default(), |t| {
                        for _ in 0..100 {
                            n += 1;
                            t.assert(v(format!("q{n}")), v("role"), v("staff"), Valid::ALWAYS)?;
                        }
                        Ok(())
                    })
                    .expect("commit");
                best = best.min(t.elapsed().as_secs_f64() * 1000.0);
                rep = Some(r);
            }
            let r = rep.expect("rep");
            println!(
                "| {} | {size} | {} | {best:.2} | {} | {} |",
                backend.name(),
                if enforced { "yes" } else { "no" },
                r.constraint_checks,
                r.seed_rows
            );
        };
        run(&db, false);
        db.transact(TxOptions::default(), |t| {
            t.store_program(
                v("policies"),
                "constraint one_ceo error :- v:role(X, v:CEO), v:role(Y, v:CEO), X != Y.",
            )?;
            t.enforce(v("policies"), true)?;
            Ok(())
        })
        .expect("enforce");
        run(&db, true);
        // writes that match an atom: every new `v:role` seeds a probe of `v:banned`
        db.transact(TxOptions::default(), |t| {
            t.store_program(
                v("policies"),
                "constraint banned error :- v:role(X, R), v:banned(R, true).",
            )?;
            Ok(())
        })
        .expect("supersede");
        run(&db, true);
    }
    let _ = std::fs::remove_dir_all(&dir);
}
