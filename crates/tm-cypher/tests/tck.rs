//! The openCypher TCK (2024.3, commit 677cbaf) on Tiramemsu: `tests/tck/features`.
//!
//! A small Gherkin runner adapted from oxilite's `tests/tck.rs` (MIT OR Apache-2.0):
//! scenarios and outlines, `having executed` setup through `Db::cypher_write`,
//! parameters, expected result tables in Neo4j notation, expected errors, and side
//! effects computed by diffing the graph before and after the query.
//!
//! Every scenario runs. Scenarios expected to fail are listed in `tests/tck/allowlist.txt`
//! (`<feature path> [n][#k] :: <reason>`); the test fails on an unexpected failure and on
//! an unexpected pass. `TM_TCK_WRITE_ALLOWLIST=<file>` writes the current failures,
//! `TM_TCK_FILTER=<substring>` runs a subset without the two checks, and
//! `TM_TCK_REPORT=1` prints every failure.

mod tck_support;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use tck_support::gherkin::{parse_feature, Scenario, StepArg};
use tck_support::values::{parse_tv, rows_eq, to_tv, TV};
use tiramemsu::{Db, Error, OpenOptions, TxOptions, Value};
use tm_cypher::{CypherParams, CypherResult, CypherValue};

fn tck_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/tck")
}

fn from_tv(v: &TV) -> CypherValue {
    match v {
        TV::Null => CypherValue::Null,
        TV::Bool(b) => CypherValue::Boolean(*b),
        TV::Int(i) => CypherValue::Integer(*i),
        TV::Float(f) => CypherValue::Float(*f),
        TV::Str(s) => CypherValue::String(s.clone()),
        TV::List(l) => CypherValue::List(l.iter().map(from_tv).collect()),
        TV::Map(m) => CypherValue::Map(m.iter().map(|(k, v)| (k.clone(), from_tv(v))).collect()),
        _ => CypherValue::Null,
    }
}

fn cypher_params(table: &[Vec<String>]) -> Result<CypherParams, String> {
    let mut p = CypherParams::new();
    for row in table {
        let [k, v] = row.as_slice() else {
            return Err("bad parameter row".into());
        };
        let tv = parse_tv(v).ok_or_else(|| format!("cannot parse parameter {v}"))?;
        p.insert(k.clone(), from_tv(&tv));
    }
    Ok(p)
}

struct Store {
    _dir: tempfile::TempDir,
    db: Db,
}

impl Store {
    fn new() -> Store {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(
            dir.path().join("tck.db"),
            OpenOptions {
                readers: 0,
                ..OpenOptions::default()
            },
        )
        .unwrap();
        Store { _dir: dir, db }
    }

    /// Runs a query: reads on the now view, writes in one transaction.
    fn run(&self, text: &str, params: &CypherParams) -> Result<CypherResult, Error> {
        match self.db.now().cypher(text, params) {
            Err(Error::Unsupported { feature }) if feature.contains("read-only view") => {
                self.db.cypher_write(TxOptions::default(), text, params)
            }
            other => other,
        }
    }
}

#[derive(Default, Debug)]
struct Snapshot {
    nodes: BTreeSet<String>,
    rels: BTreeSet<String>,
    labels: BTreeSet<String>,
    props: Vec<String>,
}

fn snapshot(store: &Store) -> Snapshot {
    let mut s = Snapshot::default();
    let none = CypherParams::new();
    let q = |c: &str| store.run(c, &none).map(|r| r.rows).unwrap_or_default();
    for r in q("MATCH (n) RETURN elementId(n)") {
        s.nodes.insert(format!("{:?}", r[0]));
    }
    for r in q("MATCH ()-[r]->() RETURN elementId(r)") {
        s.rels.insert(format!("{:?}", r[0]));
    }
    for r in q("CALL db.labels() YIELD label RETURN label") {
        s.labels.insert(format!("{:?}", r[0]));
    }
    let view = store.db.now();
    for t in view.triples(None, None, None).unwrap_or_default() {
        let (Ok(p), Ok(o), Ok(sub)) = (view.decode(t.p), view.decode(t.o), view.decode(t.s)) else {
            continue;
        };
        let Value::Iri(p) = p else { continue };
        if p.starts_with("urn:tiramemsu:sys:")
            || p.ends_with("#type")
            || matches!(sub, Value::Tx(_))
        {
            continue;
        }
        if !matches!(
            o,
            Value::Iri(_) | Value::Node(_) | Value::BNode(_) | Value::Stmt(_) | Value::Tx(_)
        ) {
            s.props.push(format!("{sub} {p} {o}"));
        }
    }
    s.props.sort();
    s
}

fn diff_count<T: Ord + Clone>(a: &[T], b: &[T]) -> (i64, i64) {
    let (mut plus, mut minus, mut i, mut j) = (0, 0, 0, 0);
    while i < a.len() || j < b.len() {
        match (a.get(i), b.get(j)) {
            (Some(x), Some(y)) if x == y => {
                i += 1;
                j += 1;
            }
            (Some(x), Some(y)) if x < y => {
                minus += 1;
                i += 1;
            }
            (Some(_), Some(_)) => {
                plus += 1;
                j += 1;
            }
            (Some(_), None) => {
                minus += 1;
                i += 1;
            }
            (None, Some(_)) => {
                plus += 1;
                j += 1;
            }
            (None, None) => break,
        }
    }
    (plus, minus)
}

fn side_effects(before: &Snapshot, after: &Snapshot) -> BTreeMap<String, i64> {
    let set = |a: &BTreeSet<String>| a.iter().cloned().collect::<Vec<_>>();
    let mut out = BTreeMap::new();
    for (name, (p, m)) in [
        ("nodes", diff_count(&set(&before.nodes), &set(&after.nodes))),
        (
            "relationships",
            diff_count(&set(&before.rels), &set(&after.rels)),
        ),
        (
            "labels",
            diff_count(&set(&before.labels), &set(&after.labels)),
        ),
        ("properties", diff_count(&before.props, &after.props)),
    ] {
        if p > 0 {
            out.insert(format!("+{name}"), p);
        }
        if m > 0 {
            out.insert(format!("-{name}"), m);
        }
    }
    out
}

enum Outcome {
    Pass,
    Fail(String),
}

fn show(v: &CypherValue) -> String {
    v.to_json().to_string()
}

fn run_scenario(sc: &Scenario) -> Outcome {
    let mut store = Store::new();
    let mut params = CypherParams::new();
    let mut result: Option<Result<CypherResult, Error>> = None;
    let mut before = Snapshot::default();
    let mut after = Snapshot::default();
    for st in &sc.steps {
        let t = st.text.as_str();
        let doc = match &st.arg {
            StepArg::Doc(d) => d.clone(),
            _ => String::new(),
        };
        let table = match &st.arg {
            StepArg::Table(t) => t.clone(),
            _ => Vec::new(),
        };
        if t == "an empty graph" || t == "any graph" {
            store = Store::new();
        } else if let Some(g) = t
            .strip_prefix("the ")
            .and_then(|x| x.strip_suffix(" graph"))
        {
            let path = tck_dir().join("graphs").join(g).join(format!("{g}.cypher"));
            let Ok(src) = std::fs::read_to_string(&path) else {
                return Outcome::Fail(format!("missing graph {g}"));
            };
            if let Err(e) = store.run(&src, &CypherParams::new()) {
                return Outcome::Fail(format!("graph {g}: {e}"));
            }
        } else if t == "having executed" {
            if let Err(e) = store.run(&doc, &CypherParams::new()) {
                return Outcome::Fail(format!("setup failed: {e}"));
            }
        } else if t == "parameters are" {
            match cypher_params(&table) {
                Ok(p) => params = p,
                Err(e) => return Outcome::Fail(e),
            }
        } else if t.starts_with("there exists a procedure") {
            return Outcome::Fail("user-defined procedures are not supported".into());
        } else if t == "executing query" || t == "executing control query" {
            before = snapshot(&store);
            result = Some(store.run(&doc, &params));
            after = snapshot(&store);
        } else if t == "the result should be empty" {
            match &result {
                Some(Ok(r)) if r.rows.is_empty() => {}
                Some(Ok(r)) => {
                    return Outcome::Fail(format!("expected no rows, got {} row(s)", r.rows.len()))
                }
                Some(Err(e)) => return Outcome::Fail(format!("error: {e}")),
                None => return Outcome::Fail("no query".into()),
            }
        } else if let Some(mode) = t.strip_prefix("the result should be") {
            let r = match &result {
                Some(Ok(r)) => r,
                Some(Err(e)) => return Outcome::Fail(format!("error: {e}")),
                None => return Outcome::Fail("no query".into()),
            };
            let ordered = mode.contains("in order") && !mode.contains("any order");
            let unordered_lists = mode.contains("ignoring element order for lists");
            let mut rows = table.clone();
            if rows.is_empty() {
                return Outcome::Fail("empty expectation table".into());
            }
            let header = rows.remove(0);
            if header != r.columns {
                return Outcome::Fail(format!("columns {:?}, expected {header:?}", r.columns));
            }
            let mut expected = Vec::new();
            for row in &rows {
                let mut e = Vec::new();
                for c in row {
                    match parse_tv(c) {
                        Some(v) => e.push(v),
                        None => return Outcome::Fail(format!("cannot parse expected value {c}")),
                    }
                }
                expected.push(e);
            }
            let actual: Vec<Vec<TV>> = r
                .rows
                .iter()
                .map(|row| row.iter().map(to_tv).collect())
                .collect();
            if !rows_eq(&actual, &expected, ordered, unordered_lists) {
                return Outcome::Fail(format!(
                    "rows differ\n  actual:   {}\n  expected: {}",
                    r.rows
                        .iter()
                        .map(|row| format!(
                            "[{}]",
                            row.iter().map(show).collect::<Vec<_>>().join(", ")
                        ))
                        .collect::<Vec<_>>()
                        .join(" "),
                    rows.iter()
                        .map(|row| format!("[{}]", row.join(", ")))
                        .collect::<Vec<_>>()
                        .join(" ")
                ));
            }
        } else if t.starts_with("a ") && t.contains("should be raised") {
            match &result {
                Some(Err(_)) => {}
                Some(Ok(r)) => {
                    return Outcome::Fail(format!("expected {t}, got {} row(s)", r.rows.len()))
                }
                None => return Outcome::Fail("no query".into()),
            }
        } else if t == "no side effects" {
            if matches!(result, Some(Err(_))) {
                continue;
            }
            let fx = side_effects(&before, &after);
            if !fx.is_empty() {
                return Outcome::Fail(format!("unexpected side effects {fx:?}"));
            }
        } else if t == "the side effects should be" {
            let fx = side_effects(&before, &after);
            let mut expected = BTreeMap::new();
            for row in &table {
                if let [k, v] = row.as_slice() {
                    expected.insert(k.clone(), v.parse::<i64>().unwrap_or(0));
                }
            }
            expected.retain(|_, v| *v != 0);
            if fx != expected {
                return Outcome::Fail(format!("side effects {fx:?}, expected {expected:?}"));
            }
        } else {
            return Outcome::Fail(format!("unknown step: {t}"));
        }
    }
    Outcome::Pass
}

fn is_read_only(sc: &Scenario) -> bool {
    sc.steps.iter().all(|st| {
        if st.text != "executing query" {
            return true;
        }
        let StepArg::Doc(d) = &st.arg else {
            return true;
        };
        let u = d.to_uppercase();
        !["CREATE", "MERGE", "SET ", "DELETE", "REMOVE"]
            .iter()
            .any(|k| u.contains(k))
    })
}

/// True when a setup or query statement creates a node without label or property
/// (`CREATE ()`, `CREATE (a)`): such nodes are not persisted (C11).
fn creates_empty_node(sc: &Scenario) -> bool {
    let bare = regex::Regex::new(r"\(\s*[A-Za-z_][A-Za-z0-9_]*?\s*\)|\(\s*\)").unwrap();
    let seg = regex::Regex::new(r"(?is)\b(CREATE|MERGE)\b(.*?)(\bMATCH\b|\bWITH\b|\bRETURN\b|\bSET\b|\bDELETE\b|\bREMOVE\b|\bUNWIND\b|\bWHERE\b|\bCALL\b|\bUNION\b|\bON\b|;|$)").unwrap();
    sc.steps.iter().any(|st| {
        if !(st.text == "having executed" || st.text.starts_with("executing")) {
            return false;
        }
        let StepArg::Doc(d) = &st.arg else {
            return false;
        };
        seg.captures_iter(d).any(|c| bare.is_match(&c[2]))
    })
}

fn load_allowlist() -> BTreeMap<String, String> {
    let path = tck_dir().join("allowlist.txt");
    let mut out = BTreeMap::new();
    if let Ok(text) = std::fs::read_to_string(path) {
        for l in text.lines() {
            if l.starts_with('#') || l.trim().is_empty() {
                continue;
            }
            let (id, reason) = l.split_once(" :: ").unwrap_or((l, ""));
            out.insert(id.trim().to_string(), reason.trim().to_string());
        }
    }
    out
}

// @lat: [[tests#Query#openCypher TCK]]
#[test]
fn opencypher_tck() {
    let run = std::thread::Builder::new()
        .name("opencypher_tck".into())
        .stack_size(64 << 20)
        .spawn(run_tck)
        .unwrap();
    if let Err(e) = run.join() {
        std::panic::resume_unwind(e);
    }
}

fn run_tck() {
    let root = tck_dir();
    let dir = root.join("features");
    let mut files = Vec::new();
    let mut stack = vec![dir];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "feature") {
                files.push(p);
            }
        }
    }
    files.sort();
    let filter = std::env::var("TM_TCK_FILTER").ok();
    let report = std::env::var("TM_TCK_REPORT").is_ok();
    let allow = load_allowlist();
    let (mut total, mut passed, mut ro_total, mut ro_passed) = (0, 0, 0, 0);
    let mut by_dir: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut unexpected_fail = Vec::new();
    let mut unexpected_pass = Vec::new();
    let mut failures = Vec::new();
    for f in &files {
        for sc in parse_feature(&root, f) {
            let id = format!("{} :: {}", sc.feature, sc.name);
            if filter.as_ref().is_some_and(|x| !id.contains(x.as_str())) {
                continue;
            }
            let ro = is_read_only(&sc);
            let outcome = std::panic::catch_unwind(|| run_scenario(&sc))
                .unwrap_or_else(|_| Outcome::Fail("panic".into()));
            total += 1;
            let dir_key = sc
                .feature
                .rsplit_once('/')
                .map_or(sc.feature.clone(), |(d, _)| d.to_string());
            let e = by_dir.entry(dir_key).or_default();
            e.0 += 1;
            if ro {
                ro_total += 1;
            }
            let num = sc.name.split(' ').next().unwrap_or("");
            let example = sc
                .name
                .rsplit_once(" #")
                .map(|(_, k)| format!("#{k}"))
                .unwrap_or_default();
            let key = format!("{} {num}{example}", sc.feature);
            match outcome {
                Outcome::Pass => {
                    passed += 1;
                    e.1 += 1;
                    if ro {
                        ro_passed += 1;
                    }
                    if allow.contains_key(&key) {
                        unexpected_pass.push(key);
                    }
                }
                Outcome::Fail(why) => {
                    let mut why1 = why.lines().next().unwrap_or("").to_string();
                    if creates_empty_node(&sc) {
                        why1.push_str(" [empty-node]");
                    }
                    failures.push(format!("{key} :: {why1}"));
                    if report {
                        let q = sc
                            .steps
                            .iter()
                            .filter(|st| st.text.starts_with("executing"))
                            .filter_map(|st| match &st.arg {
                                StepArg::Doc(d) => Some(d.replace('\n', " ")),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join(" ; ");
                        println!("FAIL {id}\n  query: {q}\n  {why}\n");
                    }
                    if !allow.contains_key(&key) {
                        unexpected_fail.push(format!("{key} :: {why1}"));
                    }
                }
            }
        }
    }
    println!(
        "openCypher TCK: {passed}/{total} scenarios pass ({:.1}%)",
        100.0 * passed as f64 / total.max(1) as f64
    );
    println!(
        "  read-only scenarios: {ro_passed}/{ro_total} ({:.1}%)",
        100.0 * ro_passed as f64 / ro_total.max(1) as f64
    );
    for (d, (t, p)) in &by_dir {
        println!("  {d:<40} {p:>4}/{t:<4}");
    }
    if let Ok(path) = std::env::var("TM_TCK_WRITE_ALLOWLIST") {
        std::fs::write(path, failures.join("\n") + "\n").unwrap();
    }
    if filter.is_none() {
        assert!(
            unexpected_fail.is_empty(),
            "{} scenario(s) fail and are not allow-listed:\n{}",
            unexpected_fail.len(),
            unexpected_fail.join("\n")
        );
        assert!(
            unexpected_pass.is_empty(),
            "{} allow-listed scenario(s) now pass, remove them from allowlist.txt:\n{}",
            unexpected_pass.len(),
            unexpected_pass.join("\n")
        );
    }
}
