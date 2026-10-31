//! The Datalog benchmark (OpenSpec change `add-datalog-engine`, task 9): goal
//! queries and full closures on SQLite and redb, against N3 (feature `n3`) on
//! the same Horn programs and against Soufflé's interpreter when `souffle` is on
//! the PATH. Every engine must produce the same number of facts before it is
//! timed.
//!
//! cargo run --release -p tm-bench --features datalog,n3 --example datalog -- [--noise 100000] [--reps 3]
//!
//! Each store holds a graph of `v:e` edges and `--noise` `v:name` literals that
//! no rule reads (pruning skips them).

use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Instant;

use tiramemsu::*;

const V: &str = "urn:tiramemsu:v:";

fn v(s: impl std::fmt::Display) -> Value {
    Value::iri(format!("{V}{s}"))
}

fn arg(name: &str) -> Option<String> {
    let a: Vec<String> = std::env::args().collect();
    a.iter()
        .position(|x| x == name)
        .and_then(|i| a.get(i + 1).cloned())
}

struct Lcg(u64);
impl Lcg {
    fn next(&mut self, m: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % m
    }
}

/// The graphs: name, edges.
fn graphs() -> Vec<(&'static str, Vec<(u64, u64)>)> {
    let mut out = Vec::new();
    // a chain of 1 500 nodes: 1.1 M closure facts
    out.push(("chain-1500", (0..1499).map(|i| (i, i + 1)).collect()));
    // a hub: 2 000 spokes into one node that points to a 200-node chain
    let mut hub: Vec<(u64, u64)> = (1..2001).map(|i| (i, 0)).collect();
    hub.extend((0..200).map(|i| (if i == 0 { 0 } else { 3000 + i - 1 }, 3000 + i)));
    out.push(("hub-2000x200", hub));
    // a small world: a ring of 1 000 with 300 random chords
    let mut r = Lcg(42);
    let mut sw: Vec<(u64, u64)> = (0..1000).map(|i| (i, (i + 1) % 1000)).collect();
    sw.extend((0..300).map(|_| (r.next(1000), r.next(1000))));
    out.push(("smallworld-1000", sw));
    out
}

fn open(path: &Path, backend: Backend) -> Db {
    Db::create(
        path,
        backend,
        OpenOptions {
            clock: Arc::new(ManualClock::new(1_000)),
            ..OpenOptions::default()
        },
    )
    .expect("create")
}

fn load(db: &Db, edges: &[(u64, u64)], noise: u64) {
    db.transact(TxOptions::default(), |t| {
        for &(a, b) in edges {
            t.assert(
                v(format!("n{a}")),
                v("e"),
                v(format!("n{b}")),
                Valid::ALWAYS,
            )?;
        }
        for i in 0..noise {
            t.assert(
                v(format!("x{i}")),
                v("name"),
                Value::str(format!("name {i}")),
                Valid::ALWAYS,
            )?;
        }
        Ok(())
    })
    .expect("load");
}

fn time<R>(reps: usize, mut f: impl FnMut() -> R) -> (R, f64) {
    let mut best = f64::MAX;
    let mut out = None;
    for _ in 0..reps {
        let t = Instant::now();
        let r = f();
        best = best.min(t.elapsed().as_secs_f64() * 1000.0);
        out = Some(r);
    }
    (out.expect("one rep"), best)
}

const TC: &str = "r(X, Y) :- v:e(X, Y).\nr(X, Z) :- r(X, Y), v:e(Y, Z).\n";
const TC_IRI: &str = "v:r(X, Y) :- v:e(X, Y).\nv:r(X, Z) :- v:r(X, Y), v:e(Y, Z).\n";
const SG: &str = "sg(X, X) :- v:e(_, X).\nsg(X, Y) :- v:e(A, X), sg(A, B), v:e(B, Y).\n";

fn souffle(dir: &Path, edges: &[(u64, u64)], program: &str) -> Option<(u64, f64)> {
    let bin = std::env::var("SOUFFLE").unwrap_or_else(|_| "souffle".into());
    Command::new(&bin).arg("--version").output().ok()?;
    let d = dir.join("souffle");
    std::fs::create_dir_all(&d).ok()?;
    let facts: String = edges.iter().map(|(a, b)| format!("{a}\t{b}\n")).collect();
    std::fs::write(d.join("e.facts"), facts).ok()?;
    std::fs::write(d.join("p.dl"), program).ok()?;
    let t = Instant::now();
    let out = Command::new(&bin)
        .args(["-F", ".", "-D", ".", "p.dl"])
        .current_dir(&d)
        .output()
        .ok()?;
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    if !out.status.success() {
        eprintln!("souffle: {}", String::from_utf8_lossy(&out.stderr));
        return None;
    }
    let csv = std::fs::read_to_string(d.join("r.csv")).ok()?;
    Some((csv.lines().count() as u64, ms))
}

/// The evaluation and result milliseconds an explain reports, best of `reps`.
fn split(
    db: &Db,
    p: &DatalogProgram,
    goal: &str,
    opts: &DatalogOptions,
    reps: usize,
) -> (f64, f64) {
    let ms = |plan: &[String], prefix: &str| -> f64 {
        plan.iter()
            .find_map(|l| l.strip_prefix(prefix))
            .and_then(|r| r.strip_suffix(" ms"))
            .and_then(|r| r.parse().ok())
            .expect("phase time")
    };
    let mut best = (f64::MAX, f64::MAX);
    for _ in 0..reps.max(1) {
        let e = db
            .now()
            .explain_datalog(p, Some(goal), opts)
            .expect("explain");
        best.0 = best.0.min(ms(&e.plan, "time: evaluation "));
        best.1 = best.1.min(ms(&e.plan, "time: results "));
    }
    best
}

fn main() {
    let noise: u64 = arg("--noise")
        .and_then(|x| x.parse().ok())
        .unwrap_or(100_000);
    let reps: usize = arg("--reps").and_then(|x| x.parse().ok()).unwrap_or(3);
    let dir = std::env::temp_dir().join(format!("tm-bench-datalog-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let full = DatalogOptions {
        demand: false,
        ..DatalogOptions::default()
    };
    println!("| workload | engine | ms | facts |\n|---|---|---:|---:|");
    for (name, edges) in graphs() {
        let mut counts = Vec::new();
        for backend in [Backend::Sqlite, Backend::Redb] {
            let path = dir.join(format!("{name}.{}", backend.name()));
            let db = open(&path, backend);
            load(&db, &edges, noise);
            let p = DatalogProgram::Text(TC.into());
            let (r, ms) = time(reps, || {
                db.now().datalog(&p, Some("r(X, Y)"), &full).expect("tc")
            });
            println!(
                "| tc {name} | datalog {} | {ms:.1} | {} |",
                backend.name(),
                r.rows.len()
            );
            counts.push(r.rows.len() as u64);
            let (ev, rs) = split(&db, &p, "r(X, Y)", &full, reps);
            println!(
                "| tc {name} | datalog {} evaluation only | {ev:.1} | |",
                backend.name()
            );
            println!(
                "| tc {name} | datalog {} results only | {rs:.1} | |",
                backend.name()
            );
            let g = DatalogProgram::Text(TC.into());
            let (r, ms) = time(reps, || {
                db.now()
                    .datalog(&g, Some("r(v:n1, Y)"), &DatalogOptions::default())
                    .expect("goal")
            });
            println!(
                "| goal {name} | datalog {} (demand) | {ms:.1} | {} |",
                backend.name(),
                r.rows.len()
            );
            if name == "chain-1500" {
                let sg = DatalogProgram::Text(SG.into());
                let (r, ms) = time(reps, || {
                    db.now().datalog(&sg, Some("sg(X, Y)"), &full).expect("sg")
                });
                println!(
                    "| sg {name} | datalog {} | {ms:.1} | {} |",
                    backend.name(),
                    r.rows.len()
                );
            }
            #[cfg(feature = "n3")]
            if backend == Backend::Sqlite && name != "chain-1500" {
                let dl = DatalogProgram::Text(TC_IRI.into());
                let (d, dms) = time(1, || {
                    db.now()
                        .datalog_reason(&dl, &DatalogOptions::default())
                        .expect("reason")
                });
                let rules = Rules::Text(
                    "{ ?x v:e ?y } => { ?x v:r ?y } .\n{ ?x v:r ?y . ?y v:e ?z } => { ?x v:r ?z } .".into(),
                );
                let (n, nms) = time(1, || {
                    db.now()
                        .reason(rules.clone(), &ReasonOptions::default())
                        .expect("n3")
                });
                // N3 keeps one statement per support; compare when it completed
                let complete = n.status == ReasonStatus::Complete;
                if complete {
                    assert_eq!(
                        d.conclusions.len(),
                        n.conclusions.len(),
                        "datalog and N3 disagree on {name}"
                    );
                }
                println!(
                    "| tc {name} (IRI) | datalog reason sqlite | {dms:.1} | {} |",
                    d.conclusions.len()
                );
                println!(
                    "| tc {name} (IRI) | n3 (eyeron) sqlite{} | {nms:.1} | {} |",
                    if complete { "" } else { ", incomplete" },
                    n.conclusions.len()
                );
            }
        }
        assert!(
            counts.windows(2).all(|w| w[0] == w[1]),
            "backends disagree on {name}"
        );
        let prog =
            ".decl e(x: number, y: number)\n.input e\n.decl r(x: number, y: number)\n.output r\n\
                    r(x, y) :- e(x, y).\nr(x, z) :- r(x, y), e(y, z).\n";
        if let Some((n, ms)) = souffle(&dir, &edges, prog) {
            assert_eq!(n, counts[0], "souffle disagrees on {name}");
            println!("| tc {name} | souffle interpreter (incl. start) | {ms:.1} | {n} |");
        }
    }
    // DatalogMTL: contagion over a contact chain with daily contacts
    let day = 86_400_000i64;
    for backend in [Backend::Sqlite, Backend::Redb] {
        let db = open(&dir.join(format!("mtl.{}", backend.name())), backend);
        db.transact(TxOptions::default(), |t| {
            t.assert(
                v("n0"),
                v("sick"),
                Value::Bool(true),
                Valid::between(0, day),
            )?;
            for i in 0..2000i64 {
                t.assert(
                    v(format!("n{i}")),
                    v("contact"),
                    v(format!("n{}", i + 1)),
                    Valid::between(i * day / 4, i * day / 4 + day),
                )?;
            }
            Ok(())
        })
        .expect("load");
        let p = DatalogProgram::Text(
            "c(X) :- v:sick(X, true).\nc(Y) :- past[0d, 3d] c(X), v:contact(X, Y).\n".into(),
        );
        let o = DatalogOptions {
            window: Some(Valid::between(0, 1000 * day)),
            demand: false,
            ..DatalogOptions::default()
        };
        let (r, ms) = time(reps, || {
            db.now().datalog(&p, Some("c(X)"), &o).expect("mtl")
        });
        println!(
            "| contagion 2000 contacts | datalog {} | {ms:.1} | {} |",
            backend.name(),
            r.rows.len()
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}
