//! `backend-benchmarks`: the harness checks results before timing, covers every
//! workload, reports losses and records its inputs.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use tm_bench::suite::{Entry, Meta};
use tm_bench::*;

/// One small run shared by the scenarios that read its results.
fn smoke() -> &'static Results {
    static R: OnceLock<Results> = OnceLock::new();
    R.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config {
            threads: vec![1, 4],
            writes: 20,
            ..Config::new(10_000, 1)
        };
        run(
            &cfg,
            &tiramemsu_engines,
            dir.path(),
            "tm bench --scale 10000 --reps 1",
        )
        .unwrap_or_else(|e| panic!("{e}"))
    })
}

// @tg: verifies:: [[openspec:backend-benchmarks#Workload coverage#Small-scale smoke run]] {since: "2026-10-27", level: integration}
#[test]
fn small_scale_smoke_run() {
    let r = smoke();
    let pairs: BTreeSet<(&str, &str)> = r
        .entries
        .iter()
        .map(|e| (e.workload.as_str(), e.engine.as_str()))
        .collect();
    for w in [
        "load",
        "size",
        "watdiv",
        "lookup",
        "cypher",
        "paths",
        "concurrency",
        "writes",
        "cyclic",
    ] {
        for e in ["sqlite", "redb"] {
            assert!(pairs.contains(&(w, e)), "no {w} entry for {e}");
        }
    }
    assert!(pairs.contains(&("cyclic", "sqlite-lftj")));
    let variants: BTreeSet<&str> = r
        .entries
        .iter()
        .filter(|e| e.workload == "lookup")
        .map(|e| e.variant.as_str())
        .collect();
    assert_eq!(variants, BTreeSet::from(["now", "asOf", "validAt"]));
    // every timed entry has a median and a p95, every value entry a unit
    for e in &r.entries {
        assert!(e.median_us.is_some() == e.p95_us.is_some(), "{e:?}");
        assert!(
            e.median_us.is_some() || (e.value.is_some() && e.unit.is_some()),
            "{e:?}"
        );
    }
}

/// An engine that adds a row to one query's answer.
struct Lying(Box<dyn Engine>, &'static str);

impl Engine for Lying {
    fn name(&self) -> String {
        self.0.name()
    }
    fn load(&self, d: &[gen::Triple], c: usize) -> Result<(), String> {
        self.0.load(d, c)
    }
    fn sparql(&self, q: &str, v: ViewSel, cap: Option<Duration>) -> Result<Rows, String> {
        let mut r = self.0.sparql(q, v, cap)?;
        if q.contains(self.1) {
            r.push(vec!["<urn:extra>".into(); r.first().map_or(1, Vec::len)]);
        }
        Ok(r)
    }
    fn views(&self) -> bool {
        self.0.views()
    }
    fn cypher(&self, q: &str, cap: Option<Duration>) -> Option<Result<Rows, String>> {
        self.0.cypher(q, cap)
    }
    fn last_t(&self) -> Option<u64> {
        self.0.last_t()
    }
    fn write(&self, op: &WriteOp) -> Option<Result<(), String>> {
        self.0.write(op)
    }
    fn size_bytes(&self) -> u64 {
        self.0.size_bytes()
    }
    fn version(&self) -> String {
        self.0.version()
    }
}

// @tg: verifies:: [[openspec:backend-benchmarks#Equal results before timing#Mismatch fails the run]] {since: "2026-10-27", level: integration}
#[test]
fn mismatch_fails_the_run() {
    // the comparison itself
    let ok = vec![vec!["<urn:a>".to_string()]];
    let bad = vec![vec!["<urn:b>".to_string()]];
    let m = check(
        "q",
        &[("sqlite".into(), Ok(ok.clone())), ("redb".into(), Ok(bad))],
    )
    .unwrap_err();
    assert_eq!(m.engines, ("sqlite".to_string(), "redb".to_string()));
    assert_eq!(
        check(
            "q",
            &[("sqlite".into(), Ok(ok.clone())), ("redb".into(), Ok(ok))]
        ),
        Ok(1)
    );
    // a whole run stops at the first query an engine answers differently
    let dir = tempfile::tempdir().unwrap();
    let factory = |d: &Path, ds: Dataset| -> Result<Vec<Box<dyn Engine>>, String> {
        let mut v = tiramemsu_engines(d, ds)?;
        let last = v.pop().expect("an engine");
        v.push(Box::new(Lying(last, "?p wd:soldBy ?s . ?s wd:locatedIn")));
        Ok(v)
    };
    let cfg = Config {
        threads: vec![1],
        writes: 2,
        ..Config::new(5_000, 1)
    };
    match run(&cfg, &factory, dir.path(), "test") {
        Err(Failure::Mismatch(m)) => {
            assert!(m.query.contains("L2"), "{m}");
            assert_eq!(m.engines, ("sqlite".to_string(), "redb".to_string()));
        }
        other => panic!(
            "expected a mismatch, got {:?}",
            other.map(|r| r.entries.len())
        ),
    }
}

fn entry(engine: &str, query: &str, median: f64) -> Entry {
    Entry {
        workload: "watdiv".into(),
        query: query.into(),
        engine: engine.into(),
        variant: "now".into(),
        rows: Some(1),
        median_us: Some(median),
        p95_us: Some(median * 1.2),
        value: None,
        unit: None,
        note: None,
    }
}

// @tg: verifies:: [[openspec:backend-benchmarks#Honest report#Report lists losses]] {since: "2026-10-27", level: unit}
#[test]
fn report_lists_losses() {
    let mut r = smoke().clone();
    r.entries = vec![
        entry("sqlite", "fast on sqlite", 100.0),
        entry("redb", "fast on sqlite", 300.0),
        entry("sqlite", "fast on redb", 400.0),
        entry("redb", "fast on redb", 100.0),
    ];
    let md = report::markdown(&r);
    let weak = md
        .split("**redb slower or larger than sqlite**")
        .nth(1)
        .expect("a weaknesses section");
    let weak = weak.split("**").next().unwrap();
    assert!(weak.contains("watdiv / fast on sqlite (now): 3.0×"), "{md}");
    assert!(!weak.contains("fast on redb"), "{md}");
    let strong = md
        .split("**redb faster or smaller than sqlite**")
        .nth(1)
        .unwrap();
    assert!(
        strong
            .split("**")
            .next()
            .unwrap()
            .contains("fast on redb (now): 4.0×"),
        "{md}"
    );
    // the full report of a real run names every workload
    let full = report::markdown(smoke());
    for w in [
        "load",
        "watdiv",
        "lookup",
        "cyclic",
        "writes",
        "size",
        "concurrency",
        "cypher",
        "paths",
    ] {
        assert!(full.contains(&format!("### {w}")), "{w}");
    }
}

// @tg: verifies:: [[openspec:backend-benchmarks#Reproducible runs#Results carry their inputs]] {since: "2026-10-27", level: integration}
#[test]
fn results_carry_their_inputs() {
    let m: &Meta = &smoke().meta;
    assert_eq!(m.scale, 10_000);
    assert_eq!(m.seeds.get("watdiv"), Some(&7));
    assert!(m.versions.contains_key("sqlite") && m.versions.contains_key("redb"));
    assert!(!m.machine.is_empty() && m.command.starts_with("tm bench"));
    // the file reads back with the same inputs and measurements (floats within
    // the last digit serde_json may change)
    let text = serde_json::to_string(smoke()).unwrap();
    let back: Results = serde_json::from_str(&text).unwrap();
    assert_eq!(&back.meta, m);
    let key = |e: &Entry| {
        (
            e.workload.clone(),
            e.query.clone(),
            e.engine.clone(),
            e.variant.clone(),
            e.rows,
        )
    };
    assert_eq!(
        back.entries.iter().map(key).collect::<Vec<_>>(),
        smoke().entries.iter().map(key).collect::<Vec<_>>()
    );
}
