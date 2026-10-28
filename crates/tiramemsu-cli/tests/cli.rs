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

#[cfg(feature = "n3")]
const SOCRATES_NT: &str = "<urn:tiramemsu:v:socrates> \
<http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <urn:tiramemsu:v:Human> .\n";

#[cfg(feature = "n3")]
const SOCRATES_N3: &str = "{ ?x a v:Human } => { ?x a v:Mortal } .\n";

/// A file holding `(v:socrates rdf:type v:Human)` on `backend`, and a rule file.
#[cfg(feature = "n3")]
fn socrates(dir: &Path, backend: &str) -> (String, String) {
    let nt = p(dir, "s.nt");
    std::fs::write(&nt, SOCRATES_NT).unwrap();
    let f = p(dir, &format!("{backend}.db"));
    ok(&["create", &f, "--backend", backend]);
    ok(&["import", &f, &nt]);
    let rules = p(dir, "socrates.n3");
    std::fs::write(&rules, SOCRATES_N3).unwrap();
    (f, rules)
}

// n3-reasoning "CLI reasoning", on both backends.
// @tg: verifies:: [[openspec:n3-reasoning#Reasoning surfaces#CLI reasoning]] {since: "2026-10-28", level: e2e}
#[cfg(feature = "n3")]
#[test]
fn cli_reasoning() {
    let d = tempfile::tempdir().unwrap();
    for backend in ["sqlite", "redb"] {
        let (f, rules) = socrates(d.path(), backend);
        let before = info(Path::new(&f));
        let out = ok(&["reason", &f, "--rules", &rules]);
        assert_eq!(
            out.lines().collect::<Vec<_>>(),
            ["(v:socrates rdf:type v:Mortal)  always"],
            "{backend}"
        );
        // writes nothing
        assert_eq!(info(Path::new(&f)), before);
        let proof = ok(&["reason", &f, "--rules", &rules, "--proof"]);
        assert!(proof.contains("by rule 0 from stmt 1"), "{proof}");
        let j: serde_json::Value =
            serde_json::from_str(&ok(&["reason", &f, "--rules", &rules, "--json"])).unwrap();
        assert_eq!(
            j["conclusions"][0]["o"],
            serde_json::json!({ "iri": "urn:tiramemsu:v:Mortal" })
        );
        // as of before the import there is nothing to conclude
        let empty = ok(&["reason", &f, "--rules", &rules, "--as-of", "0"]);
        assert_eq!(empty, "", "{backend}");
        let snap = ok(&["reason", &f, "--rules", &rules, "--valid-at", "2024-06-01"]);
        assert!(
            snap.contains("[2024-06-01T00:00:00Z, 2024-06-01T00:00:00.001Z)"),
            "{snap}"
        );
    }
}

// `tm rules put` stores (and re-storing changes nothing), `tm rules show`
// prints it, and `tm reason --rule-set` runs it.
// @tg: verifies:: [[openspec:n3-rule-sets#Rule-set surfaces]] {since: "2026-10-28", level: e2e}
#[cfg(feature = "n3")]
#[test]
fn cli_rules_put_show_and_reason() {
    let d = tempfile::tempdir().unwrap();
    let (f, rules) = socrates(d.path(), "sqlite");
    let iri = "urn:tiramemsu:v:mortality";
    let put = ok(&["rules", "put", &f, iri, &rules]);
    assert!(put.contains("stored as statement"), "{put}");
    assert!(put.contains("1 rules, monotonic"), "{put}");
    let again = ok(&["rules", "put", &f, iri, &rules]);
    assert!(again.contains("unchanged"), "{again}");
    let show = ok(&["rules", "show", &f, iri]);
    assert!(show.ends_with(&format!("{SOCRATES_N3}\n")), "{show}");
    let j: serde_json::Value =
        serde_json::from_str(&ok(&["rules", "show", &f, iri, "--json"])).unwrap();
    assert_eq!(j["text"], SOCRATES_N3);
    let out = ok(&["reason", &f, "--rule-set", iri]);
    assert!(out.contains("(v:socrates rdf:type v:Mortal)"), "{out}");
    let missing = tm(&["reason", &f, "--rule-set", "urn:tiramemsu:v:none"]);
    assert_eq!(missing.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&missing.stderr).starts_with("UnknownRuleSet:"));
    let bad = p(d.path(), "bad.n3");
    std::fs::write(&bad, "{ ?x a").unwrap();
    let parse = tm(&["rules", "put", &f, iri, &bad]);
    assert!(String::from_utf8_lossy(&parse.stderr).starts_with("N3Parse:"));
    // exactly one of --rules and --rule-set
    assert_eq!(tm(&["reason", &f]).status.code(), Some(2));
    let window = ok(&[
        "reason",
        &f,
        "--rules",
        &rules,
        "--valid-during",
        "2020-01-01..",
        "--graph",
        "urn:tiramemsu:v:none",
    ]);
    assert_eq!(window, "");
}

// n3-materialization "CLI dry run"
// @tg: verifies:: [[openspec:n3-materialization#Materialization surfaces#CLI dry run]] {since: "2026-10-28", level: e2e}
#[cfg(feature = "n3")]
#[test]
fn cli_dry_run() {
    let d = tempfile::tempdir().unwrap();
    for backend in ["sqlite", "redb"] {
        let dir = d.path().join(backend);
        std::fs::create_dir(&dir).unwrap();
        let (f, rules) = socrates(&dir, backend);
        let iri = "urn:tiramemsu:v:mortality";
        ok(&["rules", "put", &f, iri, &rules]);
        // A discarded write may grow a redb file (pages are reserved), so the
        // dry run is checked on the logical counts, not the file size.
        let logical = |f: &str| {
            let mut v = info(Path::new(f));
            v.as_object_mut().unwrap().remove("fileSize");
            v
        };
        let before = logical(&f);
        let out = ok(&["materialize", &f, iri, "--dry-run"]);
        assert!(out.contains("(v:socrates rdf:type v:Mortal)"), "{out}");
        assert!(out.contains("would derive 1"), "{out}");
        assert_eq!(logical(&f), before, "{backend}");
        let done = ok(&["materialize", &f, iri, "--require-complete"]);
        assert!(done.contains("derived 1"), "{done}");
        let again = ok(&["materialize", &f, iri]);
        assert!(again.contains("derived 0, existing 1"), "{again}");
    }
}

#[cfg(feature = "datalog")]
const CHAIN_NT: &str =
    "<urn:tiramemsu:v:alice> <urn:tiramemsu:v:manager> <urn:tiramemsu:v:bob> .\n\
<urn:tiramemsu:v:bob> <urn:tiramemsu:v:manager> <urn:tiramemsu:v:carol> .\n";

#[cfg(feature = "datalog")]
const REPORTING_DL: &str =
    "reports(X, Y) :- v:manager(X, Y).\nreports(X, Z) :- reports(X, Y), v:manager(Y, Z).\n";

#[cfg(feature = "datalog")]
const POLICIES_DL: &str = "constraint no_self error :- reports(X, X).\n\
constraint flat warn :- reports(X, Y), reports(Y, Z).\n";

/// A file holding `alice -manager-> bob -manager-> carol` on `backend`, and
/// `reporting.dl`.
#[cfg(feature = "datalog")]
fn reporting(dir: &Path, backend: &str) -> (String, String) {
    let nt = p(dir, "chain.nt");
    std::fs::write(&nt, CHAIN_NT).unwrap();
    let f = p(dir, &format!("{backend}.db"));
    ok(&["create", &f, "--backend", backend]);
    ok(&["import", &f, &nt]);
    let program = p(dir, "reporting.dl");
    std::fs::write(&program, REPORTING_DL).unwrap();
    (f, program)
}

// datalog-programs "CLI goal query", on both backends.
// @tg: verifies:: [[openspec:datalog-programs#Datalog read surfaces#CLI goal query]] {level: integration}
#[cfg(feature = "datalog")]
#[test]
fn cli_goal_query() {
    let d = tempfile::tempdir().unwrap();
    for backend in ["sqlite", "redb"] {
        let dir = d.path().join(backend);
        std::fs::create_dir(&dir).unwrap();
        let (f, program) = reporting(&dir, backend);
        let before = info(Path::new(&f));
        let o = tm(&["datalog", &f, &program, "reports(v:alice, X)"]);
        assert_eq!(
            o.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&o.stderr)
        );
        let out = String::from_utf8(o.stdout).unwrap();
        assert_eq!(
            out.lines().collect::<Vec<_>>(),
            ["X=v:bob  always", "X=v:carol  always"],
            "{backend}"
        );
        // writes nothing
        assert_eq!(info(Path::new(&f)), before);
        let proof = ok(&["datalog", &f, &program, "reports(v:alice, X)", "--proof"]);
        assert!(proof.contains("by rule 0 from stmt 1"), "{proof}");
        let j: serde_json::Value = serde_json::from_str(&ok(&[
            "datalog",
            &f,
            &program,
            "reports(v:alice, X)",
            "--json",
        ]))
        .unwrap();
        assert_eq!(j["status"], "complete");
        assert_eq!(
            j["rows"][1]["values"],
            serde_json::json!([{ "iri": "urn:tiramemsu:v:carol" }])
        );
        // as of before the import there is nothing to report
        let empty = ok(&[
            "datalog",
            &f,
            &program,
            "reports(v:alice, X)",
            "--as-of",
            "0",
        ]);
        assert_eq!(empty, "", "{backend}");
        let snap = ok(&[
            "datalog",
            &f,
            &program,
            "reports(v:alice, X)",
            "--valid-at",
            "2024-06-01",
        ]);
        assert!(
            snap.contains("X=v:bob  [2024-06-01T00:00:00Z, 2024-06-01T00:00:00.001Z)"),
            "{snap}"
        );
        let plan = ok(&["datalog", &f, &program, "reports(v:alice, X)", "--explain"]);
        assert!(plan.lines().any(|l| l.starts_with("access")), "{plan}");
    }
}

// `tm program put` stores (and re-storing changes nothing), `tm program show`
// prints it, `tm datalog` runs it by IRI, and `tm check` prints violations.
// @tg: verifies:: [[openspec:datalog-programs#Datalog read surfaces]] {level: integration}
#[cfg(feature = "datalog")]
#[test]
fn cli_program_put_show_check() {
    let d = tempfile::tempdir().unwrap();
    let (f, program) = reporting(d.path(), "sqlite");
    let policies = p(d.path(), "policies.dl");
    std::fs::write(&policies, format!("{REPORTING_DL}{POLICIES_DL}")).unwrap();
    let iri = "urn:tiramemsu:v:reporting";
    let put = ok(&["program", "put", &f, iri, &policies]);
    assert!(put.contains("stored as statement"), "{put}");
    assert!(put.contains("2 rules, 0 goals, 2 constraints"), "{put}");
    let again = ok(&["program", "put", &f, iri, &policies]);
    assert!(again.contains("unchanged"), "{again}");
    let show = ok(&["program", "show", &f, iri]);
    assert!(
        show.ends_with(&format!("{REPORTING_DL}{POLICIES_DL}\n")),
        "{show}"
    );
    let j: serde_json::Value =
        serde_json::from_str(&ok(&["program", "show", &f, iri, "--json"])).unwrap();
    assert_eq!(j["rules"], 2);
    assert_eq!(j["constraints"][0]["name"], "no_self");
    let out = ok(&["datalog", &f, iri, "reports(X, v:carol)"]);
    assert_eq!(
        out.lines().collect::<Vec<_>>(),
        ["X=v:alice  always", "X=v:bob  always"]
    );
    // a violation is a result: printed, exit status 0
    let check = ok(&["check", &f, iri]);
    assert_eq!(
        check.lines().collect::<Vec<_>>(),
        [
            "flat (warn): 1 violations",
            "    X=v:alice Y=v:bob Z=v:carol  always  stmts 1, 2",
        ]
    );
    assert_eq!(ok(&["check", &f, &policies, "--only", "no_self"]), "");
    let cj: serde_json::Value = serde_json::from_str(&ok(&["check", &f, iri, "--json"])).unwrap();
    assert_eq!(cj["checked"], serde_json::json!(["no_self", "flat"]));
    // a new version supersedes the old one
    let new = ok(&["program", "put", &f, iri, &program]);
    assert!(new.contains("new version"), "{new}");
    assert_eq!(ok(&["check", &f, iri]), "");
    let err = |args: &[&str], code: &str| {
        let o = tm(args);
        assert_eq!(o.status.code(), Some(1), "{args:?}");
        let e = String::from_utf8_lossy(&o.stderr).to_string();
        assert!(e.starts_with(&format!("{code}:")), "{e}");
    };
    err(
        &["datalog", &f, "urn:tiramemsu:v:none", "p(X)"],
        "UnknownProgram",
    );
    err(
        &["program", "show", &f, "urn:tiramemsu:v:none"],
        "UnknownProgram",
    );
    let bad = p(d.path(), "bad.dl");
    std::fs::write(&bad, "p(X) :- v:q(X, Y").unwrap();
    err(&["program", "put", &f, iri, &bad], "DatalogParse");
    err(&["datalog", &f, &bad], "DatalogParse");
    err(&["datalog", &f, "missing.dl", "p(X)"], "InvalidQuery");
    // the program is required
    assert_eq!(tm(&["datalog", &f]).status.code(), Some(2));
    let window = ok(&[
        "datalog",
        &f,
        &program,
        "reports(v:alice, X)",
        "--valid-during",
        "2020-01-01..",
        "--graph",
        "urn:tiramemsu:v:none",
    ]);
    assert_eq!(window, "");
}

#[cfg(feature = "datalog")]
const ONE_CEO_DL: &str =
    "constraint one_ceo error :- v:role(X, v:CEO), v:role(Y, v:CEO), X != Y.\n";

// `tm program enforce` on clean data exits 0 and `tm program show` reports the
// program enforced; a violating import is then refused, `tm program unenforce`
// turns it off, and enforcing over violating data fails with the violations.
#[cfg(feature = "datalog")]
// @tg: verifies:: [[openspec:datalog-constraints#Constraint surfaces#CLI enforce]] {level: integration}
#[test]
fn cli_program_enforce() {
    let d = tempfile::tempdir().unwrap();
    let f = p(d.path(), "mem.db");
    ok(&["create", &f]);
    let ceo = |who: &str| {
        let nt = p(d.path(), &format!("{who}.nt"));
        std::fs::write(
            &nt,
            format!("<urn:tiramemsu:v:{who}> <urn:tiramemsu:v:role> <urn:tiramemsu:v:CEO> .\n"),
        )
        .unwrap();
        nt
    };
    ok(&["import", &f, &ceo("a")]);
    let policies = p(d.path(), "policies.dl");
    std::fs::write(&policies, ONE_CEO_DL).unwrap();
    let iri = "urn:tiramemsu:v:policies";
    ok(&["program", "put", &f, iri, &policies]);
    let show = ok(&["program", "show", &f, iri]);
    assert!(
        show.lines().next().unwrap().ends_with("not enforced"),
        "{show}"
    );

    let o = tm(&["program", "enforce", &f, iri]);
    assert_eq!(o.status.code(), Some(0), "{o:?}");
    let out = String::from_utf8_lossy(&o.stdout).to_string();
    assert!(out.contains("enforced as statement"), "{out}");
    let show = ok(&["program", "show", &f, iri]);
    assert!(
        show.lines()
            .next()
            .unwrap()
            .contains(", enforced (statement"),
        "{show}"
    );
    let j: serde_json::Value =
        serde_json::from_str(&ok(&["program", "show", &f, iri, "--json"])).unwrap();
    assert_eq!(j["enforced"], true);
    assert!(j["enforcedStatement"].as_u64().is_some());
    let again = ok(&["program", "enforce", &f, iri]);
    assert!(again.contains("already enforced"), "{again}");

    // a second CEO is refused, with the violations on stderr
    let refused = tm(&["import", &f, &ceo("b")]);
    assert_eq!(refused.status.code(), Some(1));
    let e = String::from_utf8_lossy(&refused.stderr).to_string();
    assert!(e.starts_with("ConstraintViolated:"), "{e}");
    assert!(e.contains("X=v:a Y=v:b"), "{e}");

    // turned off, the import commits; enforcing again is refused
    let off = ok(&["program", "unenforce", &f, iri]);
    assert!(off.contains("no longer enforced"), "{off}");
    assert!(ok(&["program", "unenforce", &f, iri]).contains("not enforced"));
    ok(&["import", &f, &ceo("b")]);
    let o = tm(&["program", "enforce", &f, iri]);
    assert_eq!(o.status.code(), Some(1));
    let e = String::from_utf8_lossy(&o.stderr).to_string();
    assert!(e.starts_with("ConstraintViolated:"), "{e}");
    assert!(ok(&["program", "show", &f, iri])
        .lines()
        .next()
        .unwrap()
        .ends_with("not enforced"));
}
