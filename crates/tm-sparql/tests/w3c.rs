//! The W3C SPARQL test-suite subset (tasks 10.1–10.4).
//!
//! The runner reads the `manifest.ttl` of every in-scope category under
//! `tests/w3c/data`, runs each test against a fresh database and compares the
//! result up to blank node isomorphism. A test may fail only if it is listed in
//! `tests/w3c/expected-deviations.toml`, and a listed test that now passes fails
//! the run. `W3C_REPORT=1` prints every failure; `W3C_WRITE_DEVIATIONS=1` writes
//! the current failures as an unclassified draft of the deviations file.
#[path = "w3c/compare.rs"]
mod compare;
#[path = "w3c/manifest.rs"]
mod manifest;
mod support;

use std::collections::BTreeMap;
use std::fs;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};

use compare::*;
use manifest::*;
use support::{load_triples, TestDb};
use tiramemsu::SparqlResult;
use tm_ir::View;
use tm_sparql::env::Env;
use tm_sparql::parse;

/// In-scope categories (design D-tests 10.2); everything else is out of scope
/// (10.3) and never loaded: property-path (until M3), service, entailment,
/// protocol, http-rdf-update, csv-tsv-res, clear, drop, copy, move, add, dataset,
/// graph, and the named-graph cases of the update manifests (skipped per test).
const CATEGORIES: &[&str] = &[
    "sparql11/syntax-query",
    "sparql11/syntax-update-1",
    "sparql11/aggregates",
    "sparql11/bind",
    "sparql11/bindings",
    "sparql11/construct",
    "sparql11/exists",
    "sparql11/functions",
    "sparql11/grouping",
    "sparql11/negation",
    "sparql11/project-expression",
    "sparql11/subquery",
    "sparql11/json-res",
    "sparql11/basic-update",
    "sparql11/delete-data",
    "sparql11/delete-insert",
    "sparql11/delete-where",
    "sparql10/basic",
    "sparql10/triple-match",
    "sparql10/optional",
    "sparql10/optional-filter",
    "sparql10/algebra",
    "sparql10/bound",
    "sparql10/distinct",
    "sparql10/expr-builtin",
    "sparql10/expr-equals",
    "sparql10/expr-ops",
    "sparql10/open-world",
    "sparql10/regex",
    "sparql10/solution-seq",
    "sparql10/sort",
    "sparql10/ask",
    "sparql10/construct",
    "sparql10/boolean-effective-value",
    "sparql10/type-promotion",
    "sparql12/syntax-triple-terms-positive",
    "sparql12/syntax-triple-terms-negative",
    "sparql12/eval-triple-terms",
];

enum Outcome {
    Pass,
    Fail(String),
    Skip(String),
}

fn data_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/w3c/data")
}

fn read(p: &Path) -> String {
    fs::read_to_string(p).unwrap_or_else(|e| panic!("{p:?}: {e}"))
}

fn load_data(db: &tiramemsu::Db, files: &[PathBuf]) -> Result<(), String> {
    for f in files {
        let ext = f.extension().and_then(|e| e.to_str()).unwrap_or("");
        match ext {
            "ttl" | "nt" => {
                // through the engine terms, keeping triple terms and blank nodes
                let base = format!("file://{}", f.display());
                let mut p = oxttl::TurtleParser::new()
                    .with_base_iri(base)
                    .map_err(|e| e.to_string())?;
                if ext == "nt" {
                    p = oxttl::TurtleParser::new();
                }
                let mut ts = Vec::new();
                for t in p.for_reader(fs::read(f).map_err(|e| e.to_string())?.as_slice()) {
                    ts.push(support::rdf_triple(&t.map_err(|e| e.to_string())?));
                }
                load_triples(db, &ts)?;
            }
            other => return Err(format!("skip: data format .{other}")),
        }
    }
    Ok(())
}

fn uses_named_graphs(text: &str) -> bool {
    let up = text.to_ascii_uppercase();
    up.contains("FROM") || up.contains("GRAPH") || up.contains("USING") || up.contains("WITH")
}

fn is_ordered(text: &str) -> bool {
    text.to_ascii_uppercase().contains("ORDER BY")
}

fn run_syntax(t: &Test) -> Outcome {
    let Some(f) = &t.action_file else {
        return Outcome::Skip("no action file".into());
    };
    let text = read(f);
    let env = Env::new(View::NOW);
    let update = t.kind.contains("Update");
    let positive = t.kind.starts_with("Positive");
    let res = if update {
        parse::parse_update_only(&text, &env).map(|_| ())
    } else {
        parse::parse_query_only(&text, &env).map(|_| ())
    };
    match (positive, res) {
        (true, Ok(())) | (false, Err(_)) => Outcome::Pass,
        (true, Err(e)) => Outcome::Fail(format!("syntax rejected: {e}")),
        (false, Ok(())) => Outcome::Fail("invalid syntax accepted".into()),
    }
}

fn run_query(t: &Test) -> Outcome {
    let (Some(q), Some(res)) = (&t.query, &t.result) else {
        return Outcome::Skip("no query or result".into());
    };
    if !t.graph_data.is_empty() {
        return Outcome::Skip("named graph data".into());
    }
    let text = read(q);
    if uses_named_graphs(&text) {
        return Outcome::Skip("named graphs (FROM/GRAPH)".into());
    }
    let expected = match read_expected(res) {
        Ok(e) => e,
        Err(e) => return Outcome::Skip(e),
    };
    let db = TestDb::new();
    if let Err(e) = load_data(&db.db, &t.data) {
        return match e.strip_prefix("skip: ") {
            Some(s) => Outcome::Skip(s.to_string()),
            None => Outcome::Fail(format!("data: {e}")),
        };
    }
    let actual = match db.db.now().sparql(&text) {
        Ok(r) => r,
        Err(e) => return Outcome::Fail(format!("{e}")),
    };
    match (actual, expected) {
        (SparqlResult::Boolean(a), Expected::Boolean(b)) if a == b => Outcome::Pass,
        (SparqlResult::Boolean(a), Expected::Boolean(b)) => {
            Outcome::Fail(format!("ASK: got {a}, want {b}"))
        }
        (SparqlResult::Solutions(s), Expected::Solutions { vars, rows }) => {
            if s.vars.iter().collect::<std::collections::BTreeSet<_>>() != vars.iter().collect() {
                return Outcome::Fail(format!("variables: got {:?}, want {:?}", s.vars, vars));
            }
            let actual: Vec<Row> = s
                .rows
                .iter()
                .map(|r| {
                    s.vars
                        .iter()
                        .zip(r)
                        .filter_map(|(v, c)| {
                            c.as_ref().map(|c| {
                                (v.clone(), rdf_text(&tm_sparql::results::term::render(c)))
                            })
                        })
                        .collect()
                })
                .collect();
            if rows_equal(&actual, &rows, is_ordered(&text)) {
                Outcome::Pass
            } else {
                Outcome::Fail(format!("rows differ\n  got:  {actual:?}\n  want: {rows:?}"))
            }
        }
        (SparqlResult::Graph(g), Expected::Graph(want)) => {
            let got: Vec<[String; 3]> = g
                .iter()
                .map(|t| [rdf_text(&t.s), rdf_text(&t.p), rdf_text(&t.o)])
                .collect();
            if graphs_equal(&got, &want) {
                Outcome::Pass
            } else {
                Outcome::Fail(format!("graphs differ\n  got:  {got:?}\n  want: {want:?}"))
            }
        }
        (a, _) => Outcome::Fail(format!("result kind mismatch: {a:?}")),
    }
}

fn dump_graph(db: &tiramemsu::Db) -> Result<Vec<[String; 3]>, String> {
    match db.now().sparql("CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }") {
        Ok(SparqlResult::Graph(g)) => Ok(g
            .iter()
            .map(|t| [rdf_text(&t.s), rdf_text(&t.p), rdf_text(&t.o)])
            .collect()),
        Ok(other) => Err(format!("dump: {other:?}")),
        Err(e) => Err(format!("dump: {e}")),
    }
}

fn run_update(t: &Test) -> Outcome {
    let Some(req) = &t.request else {
        return Outcome::Skip("no request".into());
    };
    if !t.graph_data.is_empty() || !t.result_graph_data.is_empty() {
        return Outcome::Skip("named graph data".into());
    }
    let text = read(req);
    if uses_named_graphs(&text) {
        return Outcome::Skip("named graphs (GRAPH/USING/WITH)".into());
    }
    let db = TestDb::new();
    if let Err(e) = load_data(&db.db, &t.data) {
        return match e.strip_prefix("skip: ") {
            Some(s) => Outcome::Skip(s.to_string()),
            None => Outcome::Fail(format!("data: {e}")),
        };
    }
    if let Err(e) = db.db.now().sparql(&text) {
        return Outcome::Fail(format!("{e}"));
    }
    let want = match t
        .result_data
        .iter()
        .map(|f| read_graph(f))
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(v) => v.concat(),
        Err(e) => return Outcome::Skip(e),
    };
    match dump_graph(&db.db) {
        Ok(got) if graphs_equal(&got, &want) => Outcome::Pass,
        Ok(got) => Outcome::Fail(format!("store differs\n  got:  {got:?}\n  want: {want:?}")),
        Err(e) => Outcome::Fail(e),
    }
}

fn run_test(t: &Test) -> Outcome {
    if t.approval
        .as_deref()
        .is_some_and(|a| a.ends_with("Rejected") || a.ends_with("Withdrawn"))
    {
        return Outcome::Skip("rejected test".into());
    }
    let r = catch_unwind(AssertUnwindSafe(|| match t.kind.as_str() {
        k if k.contains("SyntaxTest") => run_syntax(t),
        "QueryEvaluationTest" => run_query(t),
        "UpdateEvaluationTest" => run_update(t),
        other => Outcome::Skip(format!("test type {other}")),
    }));
    r.unwrap_or_else(|p| {
        let msg = p
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_default();
        Outcome::Fail(format!("panic: {msg}"))
    })
}

fn deviations() -> BTreeMap<String, String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/w3c/expected-deviations.toml");
    let Ok(text) = fs::read_to_string(&path) else {
        return BTreeMap::new();
    };
    let table: toml::Table = text.parse().expect("expected-deviations.toml");
    let mut out = BTreeMap::new();
    for d in table
        .get("deviation")
        .and_then(|d| d.as_array())
        .cloned()
        .unwrap_or_default()
    {
        let t = d.as_table().expect("table");
        let test = t["test"].as_str().expect("test").to_string();
        let reason = t["reason"].as_str().expect("reason").to_string();
        assert!(
            out.insert(test.clone(), reason).is_none(),
            "duplicate deviation {test}"
        );
    }
    out
}

// @lat: [[tests#Query#SPARQL W3C Subset]]
#[test]
fn w3c_subset() {
    let expected = deviations();
    let root = data_root();
    let (mut pass, mut skipped) = (0usize, BTreeMap::<String, usize>::new());
    let mut failures: Vec<(String, String)> = Vec::new();
    let mut unexpected_pass: Vec<String> = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for cat in CATEGORIES {
        let manifest = root.join(cat).join("manifest.ttl");
        for t in read_manifest(&manifest) {
            if let Ok(only) = std::env::var("W3C_ONLY") {
                if !t.iri.contains(&only) {
                    continue;
                }
                if let Some(q) = t
                    .query
                    .as_ref()
                    .or(t.request.as_ref())
                    .or(t.action_file.as_ref())
                {
                    eprintln!("--- {}\n{}", t.iri, read(q));
                }
            }
            match run_test(&t) {
                Outcome::Pass => {
                    pass += 1;
                    if expected.contains_key(&t.iri) {
                        unexpected_pass.push(t.iri.clone());
                    }
                }
                Outcome::Fail(m) => failures.push((t.iri.clone(), m)),
                Outcome::Skip(why) => *skipped.entry(why).or_default() += 1,
            }
            seen.insert(t.iri.clone());
        }
    }
    let unlisted: Vec<&(String, String)> = failures
        .iter()
        .filter(|(t, _)| !expected.contains_key(t))
        .collect();
    let stale: Vec<&String> = expected.keys().filter(|t| !seen.contains(*t)).collect();
    eprintln!(
        "w3c: {pass} passed, {} failed ({} expected), skipped {skipped:?}",
        failures.len(),
        failures.len() - unlisted.len()
    );
    if std::env::var_os("W3C_REPORT").is_some() {
        for (t, m) in &failures {
            if std::env::var_os("W3C_FULL").is_some() || std::env::var_os("W3C_ONLY").is_some() {
                eprintln!("FAIL {t}: {m}");
            } else {
                eprintln!("FAIL {t}: {}", m.lines().next().unwrap_or(""));
            }
        }
    }
    if std::env::var_os("W3C_WRITE_DEVIATIONS").is_some() {
        let mut out =
            String::from("# Expected deviations of the W3C subset (see tests/w3c.rs).\n\n");
        for (t, m) in &failures {
            let reason = expected.get(t).cloned().unwrap_or_else(|| {
                format!(
                    "unclassified: {}",
                    m.lines().next().unwrap_or("").replace('"', "'")
                )
            });
            out.push_str(&format!(
                "[[deviation]]\ntest = \"{t}\"\nreason = \"{}\"\n\n",
                reason.replace('"', "'")
            ));
        }
        fs::write(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/w3c/expected-deviations.toml"),
            out,
        )
        .unwrap();
        return;
    }
    if std::env::var_os("W3C_ONLY").is_some() {
        return;
    }
    assert!(
        unlisted.is_empty(),
        "{} unexpected failures:\n{}",
        unlisted.len(),
        unlisted
            .iter()
            .map(|(t, m)| format!("{t}: {}", m.lines().next().unwrap_or("")))
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(
        unexpected_pass.is_empty(),
        "listed tests that now pass: {unexpected_pass:?}"
    );
    assert!(stale.is_empty(), "deviations for unknown tests: {stale:?}");
    assert!(pass > 300, "too few passing tests: {pass}");
}
