//! A focused read benchmark of the native runner against SQLite, for tuning.
//!
//! Loads the WatDiv-style graph (and the skewed cyclic graph) once into
//! `--dir`, reopens the files on every run, checks that both engines agree on
//! every query and prints medians. Unlike `tm bench` it keeps the files, so a
//! code change can be measured in seconds.
//!
//! cargo run --release -p tm-bench --example probe -- --scale 100000 --dir /tmp/probe [--reps 7] [--only F1]

use std::path::{Path, PathBuf};
use std::time::Instant;

use tiramemsu::rdf::{render, write_term};
use tiramemsu::*;
use tm_bench::{gen, suite, Engine, Tiramemsu};

const PREFIX: &str = "PREFIX wd: <urn:wd:> ";

fn arg(name: &str) -> Option<String> {
    let a: Vec<String> = std::env::args().collect();
    a.iter()
        .position(|x| x == name)
        .and_then(|i| a.get(i + 1).cloned())
}

fn ensure(dir: &Path, backend: Backend, data: &[gen::Triple]) -> PathBuf {
    let name = if backend == Backend::Redb {
        "redb"
    } else {
        "sqlite"
    };
    let path = dir.join(format!("{name}.db"));
    if !path.exists() {
        let t = Instant::now();
        let e = Tiramemsu::create(dir, backend, false).expect("create");
        e.load(data, 50_000).expect("load");
        e.sync().expect("sync");
        if backend == Backend::Redb {
            e.db.compact().expect("compact");
        }
        eprintln!("loaded {name} in {:.1} s", t.elapsed().as_secs_f64());
    }
    path
}

enum Q {
    Sparql(String),
    Cypher(String),
}

fn run(db: &Db, q: &Q) -> std::result::Result<Vec<Vec<String>>, String> {
    let cell = |v: &Option<Value>| v.as_ref().map_or(String::new(), |v| write_term(&render(v)));
    let mut rows: Vec<Vec<String>> = match q {
        Q::Sparql(q) => match db.now().sparql(q).map_err(|e| e.to_string())? {
            SparqlResult::Solutions(s) => s
                .rows
                .iter()
                .map(|r| r.iter().map(cell).collect())
                .collect(),
            _ => return Err("not a SELECT".into()),
        },
        Q::Cypher(q) => db
            .now()
            .cypher(q, &CypherParams::new())
            .map_err(|e| e.to_string())?
            .rows
            .iter()
            .map(|r| r.iter().map(|v| v.to_json().to_string()).collect())
            .collect(),
    };
    rows.sort();
    Ok(rows)
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn fmt(us: f64) -> String {
    if us >= 1e6 {
        format!("{:.2} s", us / 1e6)
    } else if us >= 1e3 {
        format!("{:.2} ms", us / 1e3)
    } else {
        format!("{us:.0} µs")
    }
}

fn bench(
    group: &str,
    dbs: &[(&str, &Db)],
    queries: &[(String, Q)],
    reps: usize,
    only: &Option<String>,
) {
    for (name, q) in queries {
        if only.as_ref().is_some_and(|o| !name.contains(o.as_str())) {
            continue;
        }
        let mut first: Option<Vec<Vec<String>>> = None;
        let mut out = Vec::new();
        for (en, db) in dbs {
            let r = run(db, q).unwrap_or_else(|e| panic!("{en} {name}: {e}"));
            match &first {
                None => first = Some(r),
                Some(f) => assert_eq!(f, &r, "{group}/{name}: {en} disagrees"),
            }
            let samples = (0..reps)
                .map(|_| {
                    let t = Instant::now();
                    run(db, q).unwrap();
                    t.elapsed().as_secs_f64() * 1e6
                })
                .collect();
            out.push(median(samples));
        }
        let rows = first.map_or(0, |f| f.len());
        println!(
            "{group:8} {name:16} rows {rows:>6}  sqlite {:>10}  redb {:>10}  redb/sqlite {:.2}",
            fmt(out[0]),
            fmt(out[1]),
            out[1] / out[0]
        );
    }
}

fn main() {
    let scale: u64 = arg("--scale").map_or(100_000, |s| s.parse().unwrap());
    let reps: usize = arg("--reps").map_or(7, |s| s.parse().unwrap());
    let only = arg("--only");
    let dir = PathBuf::from(arg("--dir").expect("--dir"));
    let wdir = dir.join(format!("watdiv-{scale}"));
    let cdir = dir.join(format!("skewed-{scale}"));
    std::fs::create_dir_all(&wdir).unwrap();
    std::fs::create_dir_all(&cdir).unwrap();

    let need_w = !wdir.join("redb.db").exists() || !wdir.join("sqlite.db").exists();
    let data = if need_w {
        gen::watdiv(scale, 7)
    } else {
        Vec::new()
    };
    let ws = ensure(&wdir, Backend::Sqlite, &data);
    let wr = ensure(&wdir, Backend::Redb, &data);
    let spokes = (scale / 200).clamp(20, 2_000);
    let need_c = !cdir.join("redb.db").exists() || !cdir.join("sqlite.db").exists();
    let edges = if need_c {
        gen::skewed(6, spokes, spokes * 5, 7)
    } else {
        Vec::new()
    };
    let cs = ensure(&cdir, Backend::Sqlite, &edges);
    let cr = ensure(&cdir, Backend::Redb, &edges);

    // --par N: the native runner's parallel threshold (0: sequential)
    let par: Option<usize> = arg("--par").map(|s| s.parse().unwrap());
    let open = |p: &Path| {
        let mut o = OpenOptions::default();
        if let Some(t) = par {
            o.parallel_threshold = t;
        }
        Db::open(p, o).expect("open")
    };
    let (ws, wr, cs, cr) = (open(&ws), open(&wr), open(&cs), open(&cr));

    // --loop SECONDS: run the --only queries on redb alone, for a profiler
    let looping: Option<f64> = arg("--loop").map(|s| s.parse().unwrap());

    let mut queries: Vec<(String, Q)> = suite::watdiv_queries()
        .into_iter()
        .map(|(n, q)| (n.to_string(), Q::Sparql(q)))
        .collect();
    queries.push((
        "point".into(),
        Q::Sparql(format!("{PREFIX}SELECT ?p ?o WHERE {{ wd:user7 ?p ?o }}")),
    ));
    queries.push((
        "2-hop".into(),
        Q::Sparql(format!(
            "{PREFIX}SELECT ?f ?p WHERE {{ wd:user7 wd:follows ?f . ?f wd:likes ?p }}"
        )),
    ));
    for (n, q) in [
        ("cy 2-hop", "MATCH (u {name: 'User 7'})-[:follows]->(f)-[:likes]->(p) RETURN f.name AS f, p.title AS p"),
        ("cy star", "MATCH (p:Product)-[:hasGenre]->(g {name: 'Genre 2'}) WHERE p.price < 50 RETURN p.title AS t, p.price AS x"),
        ("cy aggregate", "MATCH (r)-[:about]->(p)-[:hasGenre]->(g) RETURN g.name AS g, count(r) AS n"),
        ("cy shortest", "MATCH p = shortestPath((a {name: 'User 1'})-[:follows*]->(b {name: 'User 2'})) RETURN length(p) AS n"),
        ("cy trail 1..3", "MATCH (a {name: 'User 1'})-[:follows*1..3]->(b) RETURN count(*) AS n"),
    ] {
        queries.push((n.into(), Q::Cypher(q.into())));
    }
    queries.push((
        "reach".into(),
        Q::Sparql(format!(
            "{PREFIX}SELECT (COUNT(DISTINCT ?x) AS ?n) WHERE {{ wd:user1 wd:follows+ ?x }}"
        )),
    ));
    if let Some(secs) = looping {
        let t = Instant::now();
        let mut n = 0u64;
        while t.elapsed().as_secs_f64() < secs {
            for (name, q) in &queries {
                if only.as_ref().is_none_or(|o| name.contains(o.as_str())) {
                    run(&wr, q).unwrap();
                    n += 1;
                }
            }
        }
        eprintln!(
            "{n} runs, {:.1} µs each",
            t.elapsed().as_secs_f64() * 1e6 / n as f64
        );
        return;
    }
    bench(
        "watdiv",
        &[("sqlite", &ws), ("redb", &wr)],
        &queries,
        reps,
        &only,
    );

    let cyc: Vec<(String, Q)> = suite::cyclic_queries()
        .into_iter()
        .filter(|(n, _)| *n == "triangle" || *n == "tailed triangle")
        .map(|(n, q)| (n.to_string(), Q::Sparql(q)))
        .collect();
    bench(
        "skewed",
        &[("sqlite", &cs), ("redb", &cr)],
        &cyc,
        reps,
        &only,
    );
}
