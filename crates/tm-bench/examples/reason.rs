//! The N3 reasoning benchmark: `View::reason` (load, pruning and the
//! valid-time sweep) against store size and the number of validity bounds, on
//! SQLite and redb. See `tm_bench::reason`.
//!
//! cargo run --release -p tm-bench --features n3 --example reason -- [--full] [--reps 3] [--out results.json]
//!
//! The default is a short run (10 000 statements, 1 and 16 bounds); `--full`
//! adds 100 000 statements and 256 bounds.

use tm_bench::reason::{markdown, run, ReasonConfig};

fn arg(name: &str) -> Option<String> {
    let a: Vec<String> = std::env::args().collect();
    a.iter()
        .position(|x| x == name)
        .and_then(|i| a.get(i + 1).cloned())
}

fn main() {
    let full = std::env::args().any(|a| a == "--full");
    let mut cfg = if full {
        ReasonConfig::full()
    } else {
        ReasonConfig::quick()
    };
    if let Some(r) = arg("--reps").and_then(|r| r.parse().ok()) {
        cfg.reps = r;
    }
    let dir = std::env::temp_dir().join(format!("tm-bench-reason-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create the benchmark directory");
    let r = run(&cfg, &dir);
    let _ = std::fs::remove_dir_all(&dir);
    let entries = r.unwrap_or_else(|e| panic!("{e}"));
    if let Some(p) = arg("--out") {
        let text = serde_json::to_string_pretty(&entries).expect("results serialize");
        std::fs::write(p, text).expect("write the results");
    }
    print!("{}", markdown(&entries));
}
