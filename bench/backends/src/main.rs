//! The full backend comparison: tiramemsu on SQLite (with and without LFTJ) and
//! on redb, against Oxigraph on RocksDB, over the `tm-bench` workloads.
//!
//! ```sh
//! cargo run --release -- run --scale 100000 --reps 5 --out results-100000.json
//! cargo run --release -- report results-100000.json > report-100000.md
//! ```

use std::path::{Path, PathBuf};

use oxigraph::model::{GraphName, Literal, NamedNode, Quad, Term};
use oxigraph::sparql::{CancellationToken, QueryResults, SparqlEvaluator};
use oxigraph::store::Store;
use std::time::Duration;
use tm_bench::gen::{predicate_iri, Triple, NS, O};

use tm_bench::{Dataset, Engine, Rows, ViewSel, WriteOp, TIMEOUT};

/// Oxigraph on RocksDB in a directory.
struct Oxigraph {
    store: Store,
    dir: PathBuf,
}

fn s(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn node(local: &str) -> NamedNode {
    NamedNode::new_unchecked(format!("{NS}{local}"))
}

fn object(o: &O) -> Term {
    match o {
        O::Iri(x) => node(x).into(),
        O::Int(i) => Literal::from(*i).into(),
        O::Str(x) => Literal::new_simple_literal(x).into(),
    }
}

fn quad(t: &Triple) -> Quad {
    Quad::new(
        node(&t.s),
        NamedNode::new_unchecked(predicate_iri(&t.p)),
        object(&t.o),
        GraphName::DefaultGraph,
    )
}

fn dir_size(p: &Path) -> u64 {
    std::fs::read_dir(p).map_or(0, |rd| {
        rd.flatten()
            .map(|e| match e.metadata() {
                Ok(m) if m.is_dir() => dir_size(&e.path()),
                Ok(m) => m.len(),
                Err(_) => 0,
            })
            .sum()
    })
}

impl Oxigraph {
    fn eval(&self, q: &str, token: CancellationToken) -> Result<Rows, String> {
        match SparqlEvaluator::new()
            .with_cancellation_token(token)
            .parse_query(q)
            .map_err(s)?
            .on_store(&self.store)
            .execute()
            .map_err(s)?
        {
            QueryResults::Solutions(sol) => {
                let vars: Vec<_> = sol.variables().to_vec();
                let mut out = Vec::new();
                for row in sol {
                    let row = row.map_err(s)?;
                    out.push(
                        vars.iter()
                            .map(|v| row.get(v).map_or(String::new(), |t| t.to_string()))
                            .collect(),
                    );
                }
                Ok(out)
            }
            QueryResults::Boolean(b) => Ok(vec![vec![b.to_string()]]),
            QueryResults::Graph(_) => Err("not a SELECT".into()),
        }
    }
}

impl Engine for Oxigraph {
    fn name(&self) -> String {
        "oxigraph".into()
    }

    fn load(&self, data: &[Triple], _chunk: usize) -> Result<(), String> {
        let mut loader = self.store.bulk_loader();
        loader.load_quads(data.iter().map(quad)).map_err(s)?;
        loader.commit().map_err(s)
    }

    fn sparql(&self, q: &str, view: ViewSel, cap: Option<Duration>) -> Result<Rows, String> {
        if view != ViewSel::Now {
            return Err("Oxigraph has no transaction or valid time".into());
        }
        // the cap: a timer thread cancels the evaluation
        let token = CancellationToken::new();
        let (stop, stopped) = std::sync::mpsc::channel::<()>();
        let timer = cap.map(|cap| {
            let token = token.clone();
            std::thread::spawn(move || {
                if stopped.recv_timeout(cap).is_err() {
                    token.cancel();
                }
            })
        });
        let r = self.eval(q, token);
        let _ = stop.send(());
        if let Some(t) = timer {
            let _ = t.join();
        }
        r.map_err(|e| {
            if e.contains("cancel") {
                format!("{TIMEOUT}: after {cap:?}")
            } else {
                e
            }
        })
    }

    fn write(&self, op: &WriteOp) -> Option<Result<(), String>> {
        match op {
            WriteOp::Insert(t) => Some(self.store.insert(&quad(t)).map_err(s)),
            WriteOp::Replace(t) => Some((|| {
                let mut tx = self.store.start_transaction().map_err(s)?;
                let p = NamedNode::new_unchecked(predicate_iri(&t.p));
                let old: Vec<Quad> = tx
                    .quads_for_pattern(
                        Some(node(&t.s).as_ref().into()),
                        Some(p.as_ref()),
                        None,
                        Some(GraphName::DefaultGraph.as_ref()),
                    )
                    .collect::<Result<_, _>>()
                    .map_err(s)?;
                for q in &old {
                    tx.remove(q);
                }
                tx.insert(&quad(t));
                tx.commit().map_err(s)
            })()),
            // no statement ids, so no annotations to cascade and nothing to supersede
            WriteOp::Cascade(_) | WriteOp::Supersede(_) => None,
        }
    }

    fn size_bytes(&self) -> u64 {
        dir_size(&self.dir)
    }

    fn sync(&self) -> Result<(), String> {
        self.store.flush().map_err(s)
    }

    fn compact(&self) -> Option<Result<(), String>> {
        Some(self.store.optimize().map_err(s))
    }

    fn version(&self) -> String {
        "Oxigraph 0.5 on RocksDB".into()
    }
}

fn engines(dir: &Path, d: Dataset) -> Result<Vec<Box<dyn Engine>>, String> {
    let mut v = tm_bench::tiramemsu_engines(dir, d)?;
    let odir = dir.join("oxigraph");
    v.push(Box::new(Oxigraph {
        store: Store::open(&odir).map_err(s)?,
        dir: odir,
    }));
    Ok(v)
}

fn usage() -> ! {
    eprintln!("usage: tm-bench-backends run [--scale N] [--reps R] [--out FILE] [--dir DIR]\n       tm-bench-backends report FILE");
    std::process::exit(2)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let command = args.join(" ");
    let get = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    match args.get(1).map(String::as_str) {
        Some("run") => {
            let scale = get("--scale").map_or(100_000, |v| v.parse().unwrap_or_else(|_| usage()));
            let reps = get("--reps").map_or(5, |v| v.parse().unwrap_or_else(|_| usage()));
            let dir = get("--dir").map_or_else(
                || std::env::temp_dir().join(format!("tm-bench-backends-{}", std::process::id())),
                PathBuf::from,
            );
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("bench dir");
            let cfg = tm_bench::Config::new(scale, reps);
            let r = tm_bench::run(&cfg, &engines, &dir, &command);
            let _ = std::fs::remove_dir_all(&dir);
            match r {
                Ok(r) => {
                    if let Some(out) = get("--out") {
                        std::fs::write(&out, serde_json::to_string_pretty(&r).expect("json"))
                            .expect("results file");
                    }
                    println!("{}", tm_bench::report::markdown(&r));
                }
                Err(e) => {
                    eprintln!("benchmark failed: {e}");
                    std::process::exit(1);
                }
            }
        }
        Some("report") => {
            let file = args.get(2).unwrap_or_else(|| usage());
            let text = std::fs::read_to_string(file).expect("results file");
            let r: tm_bench::Results = serde_json::from_str(&text).expect("a results file");
            println!("{}", tm_bench::report::markdown(&r));
        }
        _ => usage(),
    }
}
