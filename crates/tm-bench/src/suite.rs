//! The workloads and the driver: every query is first run on every engine and
//! the results compared; only then is it timed. A mismatch stops the run.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::engine::{Engine, Rows, ViewSel, WriteOp, TIMEOUT};
use crate::gen::{self, Triple, O};

/// The datasets an engine factory is asked for.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Dataset {
    /// The WatDiv-style graph.
    Watdiv,
    /// A cyclic graph (uniform or skewed).
    Cyclic,
}

/// Creates the engines under test for a dataset in a fresh directory.
pub type Factory<'f> = dyn Fn(&Path, Dataset) -> Result<Vec<Box<dyn Engine>>, String> + 'f;

/// What to run.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    /// Approximate statements of the WatDiv-style dataset.
    pub scale: u64,
    /// Timed repetitions per query (after one checked warm-up run).
    pub reps: usize,
    /// Generator seed.
    pub seed: u64,
    /// Reader threads of the concurrency workload.
    pub threads: Vec<usize>,
    /// Statements per bulk-load transaction.
    pub chunk: usize,
    /// Operations per write workload.
    pub writes: usize,
    /// The longest one query may run (seconds): an engine that passes it on the
    /// checked run is recorded as timed out and left out of that comparison.
    pub timeout_s: u64,
}

impl Config {
    /// A configuration for `scale` statements with `reps` repetitions.
    pub fn new(scale: u64, reps: usize) -> Config {
        Config {
            scale,
            reps: reps.max(1),
            seed: 7,
            threads: vec![1, 4, 16],
            chunk: 50_000,
            writes: 200,
            timeout_s: 60,
        }
    }
}

/// One measurement.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// `load`, `watdiv`, `cyclic`, `lookup`, `concurrency`, `writes`, `cypher`, `paths`, `size`.
    pub workload: String,
    /// The query or operation name.
    pub query: String,
    /// The engine.
    pub engine: String,
    /// The view (`now`, `asOf`, `validAt`) or the dataset variant.
    pub variant: String,
    /// Result rows (queries).
    pub rows: Option<u64>,
    /// Median latency in microseconds.
    pub median_us: Option<f64>,
    /// 95th percentile latency in microseconds.
    pub p95_us: Option<f64>,
    /// A non-latency value (seconds, bytes per statement, queries per second).
    pub value: Option<f64>,
    /// The unit of `value`.
    pub unit: Option<String>,
    /// `timeout` when the engine passed the time cap on this query.
    #[serde(default)]
    pub note: Option<String>,
}

/// Everything needed to reproduce a run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    pub scale: u64,
    pub reps: usize,
    pub seeds: BTreeMap<String, u64>,
    pub versions: BTreeMap<String, String>,
    pub machine: String,
    pub command: String,
    pub durability: String,
    pub statements: u64,
    pub unix_time: u64,
    /// The per-query time cap in seconds.
    #[serde(default)]
    pub timeout_s: u64,
}

/// A results file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Results {
    pub meta: Meta,
    pub entries: Vec<Entry>,
}

/// Engines disagreed on a query: no timing is recorded for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mismatch {
    pub query: String,
    pub engines: (String, String),
    pub detail: String,
}

impl std::fmt::Display for Mismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "results differ on `{}` between {} and {}: {}",
            self.query, self.engines.0, self.engines.1, self.detail
        )
    }
}

/// A failed run: a mismatch, or an engine error outside a compared query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    Mismatch(Mismatch),
    Engine(String),
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failure::Mismatch(m) => m.fmt(f),
            Failure::Engine(e) => f.write_str(e),
        }
    }
}

impl From<String> for Failure {
    fn from(e: String) -> Failure {
        Failure::Engine(e)
    }
}

/// Results of at most this many rows are compared row by row; larger ones by count.
pub const COMPARE_ROWS: usize = 1_000;

// @tg: implements:: [[openspec:backend-benchmarks#Equal results before timing]] {since: "2026-10-27"}
/// Compares the results of one query on every engine: the counts always, and
/// the sorted rows when there are at most [`COMPARE_ROWS`].
pub fn check(query: &str, results: &[(String, Result<Rows, String>)]) -> Result<u64, Mismatch> {
    let mut first: Option<(&String, Rows)> = None;
    for (name, r) in results {
        let mut rows = match r {
            Ok(rows) => rows.clone(),
            Err(e) => {
                let other = results
                    .iter()
                    .find(|(n, _)| n != name)
                    .map_or("-".to_string(), |(n, _)| n.clone());
                return Err(Mismatch {
                    query: query.to_string(),
                    engines: (name.clone(), other),
                    detail: format!("{name} failed: {e}"),
                });
            }
        };
        rows.sort();
        match &first {
            None => first = Some((name, rows)),
            Some((n0, r0)) => {
                let detail = if r0.len() != rows.len() {
                    Some(format!("{} rows against {}", r0.len(), rows.len()))
                } else if rows.len() <= COMPARE_ROWS && *r0 != rows {
                    let at = r0.iter().zip(&rows).position(|(a, b)| a != b).unwrap_or(0);
                    Some(format!("row {at}: {:?} against {:?}", r0[at], rows[at]))
                } else {
                    None
                };
                if let Some(detail) = detail {
                    return Err(Mismatch {
                        query: query.to_string(),
                        engines: ((*n0).clone(), name.clone()),
                        detail,
                    });
                }
            }
        }
    }
    Ok(first.map_or(0, |(_, r)| r.len() as u64))
}

/// Median and 95th percentile of `samples` (microseconds).
pub fn stats(samples: &mut [f64]) -> (f64, f64) {
    if samples.is_empty() {
        return (0.0, 0.0);
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let at = |q: f64| samples[((samples.len() as f64 - 1.0) * q).round() as usize];
    (at(0.5), at(0.95))
}

fn micros<T>(f: impl FnOnce() -> T) -> (f64, T) {
    let t = Instant::now();
    let r = f();
    (t.elapsed().as_secs_f64() * 1e6, r)
}

/// The driver state: configuration and collected entries.
pub struct Run<'c> {
    pub cfg: &'c Config,
    pub entries: Vec<Entry>,
}

impl Run<'_> {
    fn push(&mut self, workload: &str, query: &str, engine: &str, variant: &str) -> &mut Entry {
        self.entries.push(Entry {
            workload: workload.to_string(),
            query: query.to_string(),
            engine: engine.to_string(),
            variant: variant.to_string(),
            rows: None,
            median_us: None,
            p95_us: None,
            value: None,
            unit: None,
            note: None,
        });
        self.entries.last_mut().expect("just pushed")
    }

    /// Checks `run` on every engine, then times it `reps` times on each.
    fn query(
        &mut self,
        workload: &str,
        name: &str,
        variant: &str,
        engines: &[&dyn Engine],
        run: &dyn Fn(&dyn Engine) -> Result<Rows, String>,
    ) -> Result<(), Failure> {
        if engines.is_empty() {
            return Ok(());
        }
        // the checked run, under the time cap: an engine that passes it takes no
        // further part in this query
        let mut first: Vec<(String, Result<Rows, String>)> = Vec::new();
        let mut finished: Vec<&dyn Engine> = Vec::new();
        for e in engines {
            match run(*e) {
                Err(msg) if msg.starts_with(TIMEOUT) => {
                    self.push(workload, name, &e.name(), variant).note = Some("timeout".into());
                }
                r => {
                    first.push((e.name(), r));
                    finished.push(*e);
                }
            }
        }
        let rows =
            check(&format!("{workload}/{name} [{variant}]"), &first).map_err(Failure::Mismatch)?;
        for e in &finished {
            let mut samples = Vec::with_capacity(self.cfg.reps);
            for _ in 0..self.cfg.reps {
                let (us, r) = micros(|| run(*e));
                r?;
                samples.push(us);
            }
            let (median, p95) = stats(&mut samples);
            let en = self.push(workload, name, &e.name(), variant);
            en.rows = Some(rows);
            en.median_us = Some(median);
            en.p95_us = Some(p95);
        }
        Ok(())
    }

    fn value(
        &mut self,
        workload: &str,
        query: &str,
        engine: &str,
        variant: &str,
        value: f64,
        unit: &str,
    ) {
        let en = self.push(workload, query, engine, variant);
        en.value = Some(value);
        en.unit = Some(unit.to_string());
    }
}

/// The `i`-th operation of a write workload.
type OpGen = Box<dyn Fn(usize) -> WriteOp>;

const PREFIX: &str = "PREFIX wd: <urn:wd:> ";

/// The WatDiv-style query templates: linear (L), star (S), snowflake (F) and
/// complex (C), with constants chosen to be selective at every scale.
pub fn watdiv_queries() -> Vec<(&'static str, String)> {
    let q = |body: &str| format!("{PREFIX}{body}");
    vec![
        ("L1", q("SELECT ?u ?c WHERE { ?u wd:livesIn ?c . ?c wd:inCountry wd:country3 }")),
        ("L2", q("SELECT ?r ?p WHERE { ?r wd:about ?p . ?p wd:soldBy ?s . ?s wd:locatedIn wd:city1 }")),
        ("L3", q("SELECT ?f ?p ?g WHERE { wd:user3 wd:follows ?f . ?f wd:likes ?p . ?p wd:hasGenre ?g }")),
        ("S1", q("SELECT ?p ?x ?s WHERE { ?p a wd:Product ; wd:hasGenre wd:genre2 ; wd:price ?x ; wd:soldBy ?s . FILTER(?x < 50) }")),
        ("S2", q("SELECT ?u ?a ?n WHERE { ?u wd:age ?a ; wd:livesIn wd:city4 ; wd:name ?n }")),
        ("S3", q("SELECT ?r ?p ?u WHERE { ?r wd:rating 5 ; wd:about ?p ; wd:reviewer ?u . ?p wd:hasGenre wd:genre7 }")),
        ("F1", q("SELECT ?u ?p ?s ?c WHERE { ?u wd:livesIn wd:city1 . ?u wd:likes ?p . ?p wd:hasGenre ?g . ?p wd:soldBy ?s . ?s wd:locatedIn ?c }")),
        ("F2", q("SELECT ?r ?u ?a WHERE { ?r wd:about ?p . ?r wd:rating ?k . ?p wd:hasGenre wd:genre1 . ?p wd:price ?x . ?r wd:reviewer ?u . ?u wd:age ?a . FILTER(?k >= 4 && ?x > 150) }")),
        ("C1", q("SELECT ?g (COUNT(?r) AS ?n) WHERE { ?r wd:about ?p . ?p wd:hasGenre ?g } GROUP BY ?g")),
        ("C2", q("SELECT (COUNT(*) AS ?n) WHERE { ?u wd:follows ?f . ?f wd:follows ?u . ?u wd:livesIn ?c . ?f wd:livesIn ?c }")),
    ]
}

/// LSQB-style cyclic subgraph counts.
pub fn cyclic_queries() -> Vec<(&'static str, String)> {
    let q = |body: &str| format!("{PREFIX}SELECT (COUNT(*) AS ?n) WHERE {{ {body} }}");
    vec![
        ("triangle", q("?a wd:k ?b . ?b wd:k ?c . ?c wd:k ?a")),
        (
            "diamond",
            q("?a wd:k ?b . ?a wd:k ?c . ?b wd:k ?d . ?c wd:k ?d . FILTER(?b != ?c)"),
        ),
        (
            "4-cycle",
            q("?a wd:k ?b . ?b wd:k ?c . ?c wd:k ?d . ?d wd:k ?a"),
        ),
        (
            "tailed triangle",
            q("?a wd:k ?b . ?b wd:k ?c . ?c wd:k ?a . ?c wd:k ?d"),
        ),
    ]
}

fn user(i: u64) -> String {
    format!("user{i}")
}

// @tg: implements:: [[openspec:backend-benchmarks#Workload coverage]] {since: "2026-10-27"}
/// Runs every workload on the engines `factory` creates in `dir`.
pub fn run(
    cfg: &Config,
    factory: &Factory<'_>,
    dir: &Path,
    command: &str,
) -> Result<Results, Failure> {
    let mut run = Run {
        cfg,
        entries: Vec::new(),
    };
    let cap = Some(Duration::from_secs(cfg.timeout_s.max(1)));
    let data = gen::watdiv(cfg.scale, cfg.seed);
    let users = gen::Shape::for_triples(cfg.scale).users;
    let wdir = dir.join("watdiv");
    std::fs::create_dir_all(&wdir).map_err(|e| e.to_string())?;
    let engines = factory(&wdir, Dataset::Watdiv)?;
    let all: Vec<&dyn Engine> = engines.iter().map(|e| e.as_ref()).collect();
    let mut versions = BTreeMap::new();
    for e in &all {
        versions.insert(e.name(), e.version());
    }

    // load and size
    for e in &all {
        let (us, r) = micros(|| e.load(&data, cfg.chunk));
        r?;
        run.value("load", "bulk load", &e.name(), "watdiv", us / 1e6, "s");
        e.sync()?;
        run.value(
            "size",
            "after load",
            &e.name(),
            "watdiv",
            e.size_bytes() as f64 / data.len() as f64,
            "B/statement",
        );
    }
    // a second state: the first 100 users get a new age, so `asOf` reads the past
    let t_load: Vec<Option<u64>> = all.iter().map(|e| e.last_t()).collect();
    for e in &all {
        for i in 0..users.min(100) {
            let op = WriteOp::Replace(Triple {
                s: user(i),
                p: "age".into(),
                o: O::Int(99),
            });
            match e.write(&op) {
                Some(r) => r?,
                None => {
                    return Err(Failure::Engine(format!(
                        "{} cannot replace a value",
                        e.name()
                    )))
                }
            }
        }
    }

    // WatDiv templates
    for (name, q) in watdiv_queries() {
        run.query("watdiv", name, "now", &all, &|e| {
            e.sparql(&q, ViewSel::Now, cap)
        })?;
    }

    // point and 2-hop lookups in every view
    let point = format!("{PREFIX}SELECT ?p ?o WHERE {{ wd:user7 ?p ?o }}");
    let two_hop =
        format!("{PREFIX}SELECT ?f ?p WHERE {{ wd:user7 wd:follows ?f . ?f wd:likes ?p }}");
    let viewed: Vec<(usize, &dyn Engine)> = all
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, e)| e.views())
        .collect();
    for (name, q) in [("point", &point), ("2-hop", &two_hop)] {
        run.query("lookup", name, "now", &all, &|e| {
            e.sparql(q, ViewSel::Now, cap)
        })?;
        // asOf the end of the load: each engine's own transaction number
        for (i, e) in &viewed {
            let Some(t) = t_load[*i] else { continue };
            let one = [*e];
            run.query("lookup", name, "asOf", &one, &|e| {
                e.sparql(q, ViewSel::AsOf(t), cap)
            })?;
        }
        let ve: Vec<&dyn Engine> = viewed.iter().map(|(_, e)| *e).collect();
        run.query("lookup", name, "validAt", &ve, &|e| {
            e.sparql(q, ViewSel::ValidAt(1_800_000_000_000), cap)
        })?;
    }
    // the asOf answers agree across the engines that have views
    if viewed.len() > 1 {
        let first: Vec<(String, Result<Rows, String>)> = viewed
            .iter()
            .filter_map(|(i, e)| {
                t_load[*i].map(|t| (e.name(), e.sparql(&point, ViewSel::AsOf(t), cap)))
            })
            .collect();
        check("lookup/point [asOf]", &first).map_err(Failure::Mismatch)?;
    }

    // Cypher on the engines that have it
    let cy: Vec<&dyn Engine> = all
        .iter()
        .copied()
        .filter(|e| e.cypher("RETURN 1 AS x", cap).is_some())
        .collect();
    for (name, q) in [
        ("2-hop", "MATCH (u {name: 'User 7'})-[:follows]->(f)-[:likes]->(p) RETURN f.name AS f, p.title AS p"),
        ("star", "MATCH (p:Product)-[:hasGenre]->(g {name: 'Genre 2'}) WHERE p.price < 50 RETURN p.title AS t, p.price AS x"),
        ("aggregate", "MATCH (r)-[:about]->(p)-[:hasGenre]->(g) RETURN g.name AS g, count(r) AS n"),
    ] {
        run.query("cypher", name, "now", &cy, &|e| e.cypher(q, cap).unwrap_or_else(|| Err("no Cypher".into())))?;
    }

    // paths
    let reach =
        format!("{PREFIX}SELECT (COUNT(DISTINCT ?x) AS ?n) WHERE {{ wd:user1 wd:follows+ ?x }}");
    run.query("paths", "reachability", "now", &all, &|e| {
        e.sparql(&reach, ViewSel::Now, cap)
    })?;
    for (name, q) in [
        ("shortest", "MATCH p = shortestPath((a {name: 'User 1'})-[:follows*]->(b {name: 'User 2'})) RETURN length(p) AS n"),
        ("trail 1..3", "MATCH (a {name: 'User 1'})-[:follows*1..3]->(b) RETURN count(*) AS n"),
    ] {
        run.query("paths", name, "now", &cy, &|e| e.cypher(q, cap).unwrap_or_else(|| Err("no Cypher".into())))?;
    }

    // concurrent readers: 2-hop lookups from different users
    for &threads in &cfg.threads {
        let per_thread = 20 * cfg.reps;
        for e in &all {
            let t = Instant::now();
            let failed = std::thread::scope(|sc| {
                let hs: Vec<_> = (0..threads)
                    .map(|k| {
                        sc.spawn(move || {
                            for i in 0..per_thread {
                                let u = (k * per_thread + i) as u64 % users;
                                let q = format!("{PREFIX}SELECT ?f ?p WHERE {{ wd:user{u} wd:follows ?f . ?f wd:likes ?p }}");
                                e.sparql(&q, ViewSel::Now, None)?;
                            }
                            Ok::<(), String>(())
                        })
                    })
                    .collect();
                hs.into_iter()
                    .find_map(|h| h.join().ok().and_then(|r| r.err()))
            });
            if let Some(err) = failed {
                return Err(Failure::Engine(err));
            }
            let qps = (threads * per_thread) as f64 / t.elapsed().as_secs_f64();
            run.value(
                "concurrency",
                "2-hop",
                &e.name(),
                &format!("{threads} threads"),
                qps,
                "queries/s",
            );
        }
    }

    // writes
    let n = cfg.writes;
    let ops: Vec<(&str, OpGen)> = vec![
        (
            "insert",
            Box::new(|i| {
                WriteOp::Insert(Triple {
                    s: format!("new{i}"),
                    p: "name".into(),
                    o: O::Str(format!("New {i}")),
                })
            }),
        ),
        (
            "replace (churn)",
            Box::new(|i| {
                WriteOp::Replace(Triple {
                    s: user(i as u64 % 50),
                    p: "age".into(),
                    o: O::Int(i as i64),
                })
            }),
        ),
        (
            "cascade",
            Box::new(|i| {
                WriteOp::Cascade(Triple {
                    s: format!("fact{i}"),
                    p: "about".into(),
                    o: O::Iri("product1".into()),
                })
            }),
        ),
        // a fresh value each time: superseding with the same value is refused
        (
            "supersede",
            Box::new(|i| {
                WriteOp::Supersede(Triple {
                    s: user(100 + i as u64 % 50),
                    p: "age".into(),
                    o: O::Int(1_000 + i as i64),
                })
            }),
        ),
    ];
    for (name, op) in &ops {
        for e in &all {
            let mut samples = Vec::with_capacity(n);
            let mut supported = true;
            for i in 0..n {
                let (us, r) = micros(|| e.write(&op(i)));
                match r {
                    None => {
                        supported = false;
                        break;
                    }
                    Some(r) => r?,
                }
                samples.push(us);
            }
            if supported {
                let (median, p95) = stats(&mut samples);
                let en = run.push("writes", name, &e.name(), "watdiv");
                en.median_us = Some(median);
                en.p95_us = Some(p95);
            }
        }
    }
    for e in &all {
        e.sync()?;
        run.value(
            "size",
            "after churn",
            &e.name(),
            "watdiv",
            e.size_bytes() as f64 / data.len() as f64,
            "B/statement",
        );
        if let Some(r) = e.compact() {
            r?;
            run.value(
                "size",
                "after compaction",
                &e.name(),
                "watdiv",
                e.size_bytes() as f64 / data.len() as f64,
                "B/statement",
            );
        }
    }
    drop(all);
    drop(engines);

    // cyclic subgraphs on a uniform and a skewed graph
    let spokes = (cfg.scale / 200).clamp(20, 2_000);
    let graphs = [
        (
            "uniform",
            gen::uniform((cfg.scale / 10).clamp(200, 200_000)),
        ),
        ("skewed", gen::skewed(6, spokes, spokes * 5, cfg.seed)),
    ];
    for (variant, edges) in graphs {
        let gdir = dir.join(variant);
        std::fs::create_dir_all(&gdir).map_err(|e| e.to_string())?;
        let engines = factory(&gdir, Dataset::Cyclic)?;
        let all: Vec<&dyn Engine> = engines.iter().map(|e| e.as_ref()).collect();
        for e in &all {
            versions.entry(e.name()).or_insert_with(|| e.version());
            e.load(&edges, cfg.chunk)?;
        }
        for (name, q) in cyclic_queries() {
            run.query("cyclic", name, variant, &all, &|e| {
                e.sparql(&q, ViewSel::Now, cap)
            })?;
        }
    }

    // @tg: implements:: [[openspec:backend-benchmarks#Reproducible runs]] {since: "2026-10-27"}
    let durability = if versions.contains_key("oxigraph") {
        "tiramemsu: Normal on both backends; Oxigraph: its default (RocksDB WAL)".to_string()
    } else {
        "tiramemsu: Normal on both backends".to_string()
    };
    let meta = Meta {
        scale: cfg.scale,
        reps: cfg.reps,
        seeds: BTreeMap::from([
            ("watdiv".to_string(), cfg.seed),
            ("skewed".to_string(), cfg.seed),
        ]),
        versions,
        machine: machine(),
        command: command.to_string(),
        durability,
        statements: data.len() as u64,
        timeout_s: cfg.timeout_s,
        unix_time: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
    };
    Ok(Results {
        meta,
        entries: run.entries,
    })
}

/// The machine: OS, architecture, CPU model and core count.
pub fn machine() -> String {
    let cpu = std::process::Command::new("sysctl")
        .args(["-n", "machdep.cpu.brand_string"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .or_else(|| {
            std::fs::read_to_string("/proc/cpuinfo").ok().and_then(|s| {
                s.lines()
                    .find(|l| l.starts_with("model name"))
                    .map(|l| l.split(':').nth(1).unwrap_or("").trim().to_string())
            })
        })
        .unwrap_or_else(|| "unknown CPU".to_string());
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    format!(
        "{} {} · {cpu} · {cores} threads",
        std::env::consts::OS,
        std::env::consts::ARCH
    )
}
