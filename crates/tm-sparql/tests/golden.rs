//! Golden tests (tasks 9.1–9.3): every `tests/golden/<name>.rq` / `.ru` runs against
//! a fresh temporary database.
//!
//! Sibling files: `.ttl` (Turtle fixture), `.fixture` (name of an API-built
//! fixture), `.ir` (the lowered IR text), `.srj` (expected SPARQL JSON), `.nt`
//! (expected `CONSTRUCT` triples), `.err` (the expected error), `.check` (a query
//! run after an update, compared with `.srj`). `UPDATE_GOLDEN=1` rewrites the
//! `.ir`, `.srj` and `.nt` files from the actual output.
mod support;

use std::fs;
use std::path::{Path, PathBuf};

use support::*;
use tiramemsu::{Error, SparqlResult, Valid, Value};
use tm_ir::View;
use tm_sparql::env::Env;
use tm_sparql::{prepare, Prepared};

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

fn update_mode() -> bool {
    std::env::var_os("UPDATE_GOLDEN").is_some()
}

fn v(local: &str) -> Value {
    Value::iri(format!("urn:tiramemsu:v:{local}"))
}

/// API-built fixtures for what Turtle cannot express (valid time, parallel edges,
/// anonymous nodes).
fn api_fixture(name: &str, t: &TestDb) {
    let ms = |d: &str| tiramemsu::value::parse_datetime(d).unwrap().0;
    match name {
        "two_episodes" => {
            t.db.transact(Default::default(), |tx| {
                tx.assert(
                    v("alice"),
                    v("worksAt"),
                    v("acme"),
                    Valid::between(ms("2020-01-01T00:00:00Z"), ms("2022-01-01T00:00:00Z")),
                )?;
                tx.assert(
                    v("alice"),
                    v("worksAt"),
                    v("acme"),
                    Valid::from(ms("2024-01-01T00:00:00Z")),
                )?;
                Ok(())
            })
            .unwrap();
        }
        "parallel_edges" => {
            t.db.transact(Default::default(), |tx| {
                tx.create(v("alice"), v("called"), v("bob"), Valid::ALWAYS)?;
                tx.create(v("alice"), v("called"), v("bob"), Valid::ALWAYS)?;
                Ok(())
            })
            .unwrap();
        }
        "anon_node" => {
            t.db.transact(Default::default(), |tx| {
                let n = tx.new_node()?;
                tx.assert(n, v("name"), Value::str("anon"), Valid::ALWAYS)?;
                Ok(())
            })
            .unwrap();
        }
        "graphs" => {
            // alice in two graphs, bob in one, carol in none
            t.db.transact(Default::default(), |tx| {
                let g = |n: &str| Value::iri(format!("http://example.org/{n}"));
                let a = tx
                    .assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?
                    .eid();
                let b = tx
                    .assert(v("bob"), v("worksAt"), v("acme"), Valid::ALWAYS)?
                    .eid();
                tx.assert(v("carol"), v("name"), Value::str("Carol"), Valid::ALWAYS)?;
                tx.add_to_graph(a, g("g"), Default::default())?;
                tx.add_to_graph(a, g("graph1"), Default::default())?;
                tx.add_to_graph(b, g("g"), Default::default())?;
                Ok(())
            })
            .unwrap();
        }
        other => panic!("unknown fixture {other}"),
    }
}

fn check_text(path: &Path, actual: &str) {
    if update_mode() {
        fs::write(path, actual).unwrap();
        return;
    }
    let want = fs::read_to_string(path).unwrap_or_else(|_| panic!("missing golden {path:?}"));
    assert_eq!(actual.trim_end(), want.trim_end(), "{path:?}");
}

fn error_text(e: &Error) -> String {
    match e {
        Error::Unsupported { feature } => format!("Unsupported: {feature}"),
        Error::Parse { span: Some(s), .. } => format!("Parse: {}:{}", s.line, s.column),
        Error::Parse { msg, .. } => format!("Parse: ~{msg}"),
        other => {
            let dbg = format!("{other:?}");
            let name: String = dbg.chars().take_while(|c| c.is_alphanumeric()).collect();
            format!("Error: {name}")
        }
    }
}

/// True when the expected error text matches the actual one.
fn error_matches(want: &str, e: &Error) -> bool {
    let got = error_text(e);
    match want.strip_prefix("Parse: ~") {
        Some(sub) => matches!(e, Error::Parse { msg, .. } if msg.contains(sub)),
        None => got == want,
    }
}

fn run_case(rq: &Path) {
    let name = rq.file_stem().unwrap().to_str().unwrap().to_string();
    let dir = rq.parent().unwrap();
    let sibling = |ext: &str| dir.join(format!("{name}.{ext}"));
    let text = fs::read_to_string(rq).unwrap();
    let is_update = rq.extension().unwrap() == "ru";
    let t = TestDb::new();
    if sibling("ttl").exists() {
        load_turtle(&t.db, &fs::read_to_string(sibling("ttl")).unwrap(), None)
            .unwrap_or_else(|e| panic!("{name}: fixture: {e}"));
    }
    if sibling("fixture").exists() {
        api_fixture(fs::read_to_string(sibling("fixture")).unwrap().trim(), &t);
    }
    let t_before = t.last_t();
    let runs_before = t.runs();

    if sibling("ir").exists() || (update_mode() && !is_update && !sibling("err").exists()) {
        let env = Env::new(View::NOW);
        if let Ok(Prepared::Query(p)) = prepare(&text, &env) {
            check_text(&sibling("ir"), &p.query.to_string());
        }
    }

    let result = t.db.now().sparql(&text);
    if sibling("err").exists() {
        let mut want = fs::read_to_string(sibling("err")).unwrap();
        if want.trim() == "?" && update_mode() {
            let e = result.expect_err(&format!("{name}: expected an error"));
            fs::write(sibling("err"), format!("{}\n", error_text(&e))).unwrap();
            return;
        }
        want = want.trim().to_string();
        let e = result.expect_err(&format!("{name}: expected an error"));
        assert!(error_matches(&want, &e), "{name}: want {want:?}, got {e:?}");
        // rejected before any SQL: no query ran, no transaction opened
        assert_eq!(t.runs(), runs_before, "{name}: SQL ran");
        assert_eq!(t.last_t(), t_before, "{name}: a transaction was opened");
        return;
    }
    let result = result.unwrap_or_else(|e| panic!("{name}: {e}"));
    let actual = if is_update {
        assert!(matches!(result, SparqlResult::Update(_)), "{name}");
        match fs::read_to_string(sibling("check")) {
            Ok(check) => {
                let r =
                    t.db.now()
                        .sparql(&check)
                        .unwrap_or_else(|e| panic!("{name}: check: {e}"));
                to_srj(&r).expect("check result")
            }
            Err(_) => return,
        }
    } else {
        match &result {
            SparqlResult::Graph(g) => {
                let mut lines: Vec<String> = tm_sparql::results::nt::write(g)
                    .lines()
                    .map(|l| {
                        // blank node labels are fresh per solution: normalise them
                        let mut out = String::new();
                        let mut chars = l.chars().peekable();
                        while let Some(c) = chars.next() {
                            if c == '_' && chars.peek() == Some(&':') {
                                out.push_str("_:b");
                                chars.next();
                                while chars
                                    .peek()
                                    .is_some_and(|c| c.is_alphanumeric() || *c == '_')
                                {
                                    chars.next();
                                }
                            } else {
                                out.push(c);
                            }
                        }
                        out
                    })
                    .collect();
                lines.sort();
                let actual = lines.join("\n");
                check_text(&sibling("nt"), &actual);
                return;
            }
            other => to_srj(other).expect("solutions or boolean"),
        }
    };
    let path = sibling("srj");
    if update_mode() {
        fs::write(&path, format!("{actual}\n")).unwrap();
        return;
    }
    let want = fs::read_to_string(&path).unwrap_or_else(|_| panic!("{name}: missing .srj"));
    let (a, w) = (parse_srj(&actual), parse_srj(&want));
    let ordered = text.to_uppercase().contains("ORDER BY") || check_is_ordered(&sibling("check"));
    match (a, w) {
        (
            Srj::Solutions {
                vars: va,
                rows: mut ra,
            },
            Srj::Solutions {
                vars: vw,
                rows: mut rw,
            },
        ) => {
            assert_eq!(va, vw, "{name}: variables");
            if !ordered {
                ra.sort();
                rw.sort();
            }
            assert_eq!(ra, rw, "{name}: rows\nactual: {actual}");
        }
        (a, w) => assert_eq!(a, w, "{name}"),
    }
}

fn check_is_ordered(p: &Path) -> bool {
    fs::read_to_string(p).is_ok_and(|s| s.to_uppercase().contains("ORDER BY"))
}

// @lat: [[tests#Query#SPARQL Golden Cases]]
#[test]
fn golden_cases() {
    let mut cases: Vec<PathBuf> = fs::read_dir(golden_dir())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| matches!(p.extension().and_then(|e| e.to_str()), Some("rq" | "ru")))
        .collect();
    cases.sort();
    assert!(cases.len() >= 20, "golden cases missing: {}", cases.len());
    for c in &cases {
        run_case(c);
    }
}
