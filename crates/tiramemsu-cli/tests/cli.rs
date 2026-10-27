//! `tm-cli`: every scenario runs the built `tm` binary.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::{Command, Output};

use tiramemsu::*;

fn tm(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tm"))
        .args(args)
        .output()
        .expect("tm runs")
}

fn ok(args: &[&str]) -> String {
    let o = tm(args);
    assert!(
        o.status.success(),
        "tm {args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8(o.stdout).unwrap()
}

fn info(file: &Path) -> serde_json::Value {
    serde_json::from_str(&ok(&["info", file.to_str().unwrap(), "--json"])).unwrap()
}

const DATA: &str = "<urn:a> <urn:knows> <urn:b> .\n\
<urn:b> <urn:knows> <urn:c> .\n\
<urn:a> <urn:name> \"Alice\"@en .\n\
<urn:b> <urn:name> \"Bob\" .\n\
<urn:a> <urn:age> \"41\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n";

fn p(dir: &Path, name: &str) -> String {
    dir.join(name).to_str().unwrap().to_string()
}

// @tg: verifies:: [[openspec:tm-cli#Create on a chosen backend#Create a redb database]] {since: "2026-10-27", level: e2e}
#[test]
fn create_a_redb_database() {
    let d = tempfile::tempdir().unwrap();
    let f = p(d.path(), "mem.db");
    ok(&["create", &f, "--backend", "redb"]);
    assert_eq!(info(Path::new(&f))["backend"], "redb");
    // an existing file is refused
    let again = tm(&["create", &f, "--backend", "redb"]);
    assert_eq!(again.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&again.stderr).starts_with("AlreadyExists:"));
}

// @tg: verifies:: [[openspec:tm-cli#Inspect a file#Info on a SQLite file]] {since: "2026-10-27", level: e2e}
#[test]
fn info_on_a_sqlite_file() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("m.db");
    {
        let db = Db::create(&f, Backend::Sqlite, OpenOptions::default()).unwrap();
        let v = |s: &str| Value::iri(format!("urn:{s}"));
        db.transact(TxOptions::default(), |tx| {
            let e = tx.assert(v("a"), v("p"), v("b"), Valid::ALWAYS)?.eid();
            tx.assert(v("a"), v("p"), v("c"), Valid::ALWAYS)?;
            tx.assert(v("b"), v("p"), v("c"), Valid::ALWAYS)?;
            tx.retract(e)?;
            Ok(())
        })
        .unwrap();
    }
    let i = info(&f);
    assert_eq!(i["backend"], "sqlite");
    assert_eq!(i["liveStatements"], 2);
    assert_eq!(i["statements"], 3);
}

// @tg: verifies:: [[openspec:tm-cli#Query a file#Same query on both backends]] {since: "2026-10-27", level: e2e}
#[test]
fn same_query_on_both_backends() {
    let d = tempfile::tempdir().unwrap();
    let nt = p(d.path(), "d.nt");
    std::fs::write(&nt, DATA).unwrap();
    let mut outs = Vec::new();
    for backend in ["sqlite", "redb"] {
        let f = p(d.path(), &format!("{backend}.db"));
        ok(&["create", &f, "--backend", backend]);
        ok(&["import", &f, &nt]);
        for q in [
            "SELECT ?x ?y WHERE { ?x <urn:knows> ?y } ORDER BY ?x",
            "SELECT ?x ?n WHERE { ?x <urn:name> ?n OPTIONAL { ?x <urn:age> ?a } } ORDER BY ?n",
        ] {
            outs.push((
                backend,
                ok(&["query", &f, "--sparql", q, "--format", "json"]),
            ));
        }
        outs.push((
            backend,
            ok(&[
                "query",
                &f,
                "--cypher",
                "MATCH (a)-[r]->(b) RETURN count(*) AS n",
                "--format",
                "json",
            ]),
        ));
    }
    let (sql, kv) = outs.split_at(outs.len() / 2);
    for (a, b) in sql.iter().zip(kv) {
        assert_eq!(a.1, b.1);
    }
    assert!(sql[0].1.contains("urn:c"), "{}", sql[0].1);
}

// @tg: verifies:: [[openspec:tm-cli#Query a file#Query error]] {since: "2026-10-27", level: e2e}
#[test]
fn query_error() {
    let d = tempfile::tempdir().unwrap();
    let f = p(d.path(), "m.db");
    ok(&["create", &f]);
    let o = tm(&["query", &f, "--sparql", "SELEC ?x WHERE"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&o.stderr).starts_with("Parse:"),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    // a usage error is exit code 2
    assert_eq!(tm(&["query", &f]).status.code(), Some(2));
}

fn lines(text: &str) -> BTreeSet<String> {
    text.lines()
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

// @tg: verifies:: [[openspec:tm-cli#Import and export#Import then export]] {since: "2026-10-27", level: e2e}
#[test]
fn import_then_export() {
    let d = tempfile::tempdir().unwrap();
    let nt = p(d.path(), "d.nt");
    std::fs::write(&nt, DATA).unwrap();
    for backend in ["sqlite", "redb"] {
        let f = p(d.path(), &format!("{backend}.db"));
        ok(&["create", &f, "--backend", backend]);
        ok(&["import", &f, &nt]);
        let out = ok(&["export", &f, "--format", "ntriples"]);
        assert_eq!(lines(&out), lines(DATA), "{backend}");
    }
    // N-Quads keep the graph
    let nq = p(d.path(), "g.nq");
    std::fs::write(&nq, "<urn:a> <urn:knows> <urn:b> <urn:g> .\n").unwrap();
    let f = p(d.path(), "g.db");
    ok(&["create", &f, "--backend", "redb"]);
    ok(&["import", &f, &nq]);
    let out = ok(&["export", &f]);
    assert!(
        lines(&out).contains("<urn:a> <urn:knows> <urn:b> <urn:g> ."),
        "{out}"
    );
}

// @tg: verifies:: [[openspec:tm-cli#Convert and compact#Convert from SQLite to redb]] {since: "2026-10-27", level: e2e}
#[test]
fn convert_from_sqlite_to_redb() {
    let d = tempfile::tempdir().unwrap();
    let nt = p(d.path(), "d.nt");
    std::fs::write(&nt, DATA).unwrap();
    let a = p(d.path(), "a.sqlite");
    let b = p(d.path(), "b.redb");
    ok(&["create", &a]);
    ok(&["import", &a, &nt]);
    ok(&["convert", &a, &b, "--backend", "redb"]);
    let (ia, ib) = (info(Path::new(&a)), info(Path::new(&b)));
    assert_eq!(
        (ia["backend"].as_str(), ib["backend"].as_str()),
        (Some("sqlite"), Some("redb"))
    );
    for k in [
        "liveStatements",
        "statements",
        "terms",
        "transactions",
        "formatVersion",
    ] {
        assert_eq!(ia[k], ib[k], "{k}");
    }
    let compacted = ok(&["compact", &b]);
    assert!(compacted.starts_with("compacted"), "{compacted}");
    assert_eq!(info(Path::new(&b))["statements"], ia["statements"]);
}
