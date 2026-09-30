//! Native path regions: routing, `tm_path` composition with a mock operator.

mod common;

use std::sync::Arc;

use common::mock::MockPath;
use common::*;
use tiramemsu::ir::builder::IrBuilder;
use tiramemsu::ir::{Op, PathExpr, PathMode, PathPattern, TermOrVar, View};
use tiramemsu::*;

fn b() -> IrBuilder {
    IrBuilder::sparql()
}

fn with_mock() -> (TestDb, Arc<MockPath>) {
    let m = Arc::new(MockPath::default());
    let t = TestDb::open(OpenOptions::default().with_native_operator(m.clone()));
    (t, m)
}

fn supported_by_path(bb: &IrBuilder) -> Op {
    bb.path(
        "?r",
        PathExpr::iri(tiramemsu::ir::vocab::SYS_SUBJECT).star(),
        "?x",
        PathMode::Reachability,
    )
}

// sql-execution "No path operator registered": a bare engine (an embedder that
// registered no operator) still refuses before running any SQL. A `Db` always
// registers the real `tm_path` operator now (add-path-engine).
#[test]
fn no_operator_is_unsupported_without_sql() {
    let engine = tm_exec::QueryEngine::new(
        tm_exec::PlannerOptions::default(),
        tm_exec::OperatorRegistry::new(),
        16,
    );
    let q = b().query(Op::join(vec![
        b().triple("?b", "v:p", "?r"),
        supported_by_path(&b()),
    ]));
    match engine.prepare(&q, &Params::new()) {
        Err(Error::Unsupported { feature }) => {
            assert!(feature.contains("path patterns"), "{feature}")
        }
        other => panic!("{other:?}"),
    }
}

// the default database routes the same query to the real operator (task 10.4)
#[test]
fn default_database_routes_paths_to_tm_path() {
    let t = TestDb::new();
    t.tx(|tx| tx.assert(v("a"), v("p"), v("b"), Valid::ALWAYS).map(|_| ()));
    let q = b().query(Op::join(vec![
        b().triple("?b", "v:p", "?r"),
        supported_by_path(&b()),
    ]));
    let ex = explain(&t.db.now(), &q);
    assert!(ex.regions.iter().any(|r| r.kind == RegionKind::NativePath));
    assert!(ex.sql.unwrap().contains("tm_path("));
}

// sql-execution "Path composes as a table-valued function"
// virtual-predicates "Path expression with a virtual hop"
#[test]
fn path_composes_as_tvf() {
    let (t, m) = with_mock();
    let mut e1 = None;
    t.tx(|tx| {
        e1 = Some(
            tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?
                .eid(),
        );
        tx.assert(v("belief9"), v("supportedBy"), e1.unwrap(), Valid::ALWAYS)?;
        Ok(())
    });
    let bb = b().at(View::as_of_tx(1));
    let root = Op::join(vec![
        b().triple("?b", "v:supportedBy", "?r"),
        bb.path(
            "?r",
            PathExpr::iri(tiramemsu::ir::vocab::SYS_SUBJECT),
            "?x",
            PathMode::Reachability,
        ),
    ]);
    let q = b().query(root);
    let ex = explain(&t.db.now(), &q);
    let sql = ex.sql.clone().unwrap();
    assert_eq!(sql.matches("tm_path(").count(), 1, "{sql}");
    assert!(
        sql.contains("tm_path(t0.o,"),
        "start is the column bound to ?r: {sql}"
    );
    assert!(ex.regions.iter().any(|r| r.kind == RegionKind::NativePath));
    assert!(
        ex.params.contains(&SqlValue::Text("asOf/1".into())),
        "{:?}",
        ex.params
    );
    assert!(ex.params.contains(&SqlValue::Text("sys:subject".into())));
    assert!(ex.params.contains(&SqlValue::Text("REACH".into())));
    let r = run(&t.db.now(), &q);
    assert_eq!(r.len(), 1);
    assert_eq!(r.get(0, "x"), Some(&Value::Stmt(e1.unwrap())));
    assert_eq!(m.calls.lock().unwrap().len(), 1);
}

// sql-execution "End-bound path"
#[test]
fn end_bound_path_is_inverted() {
    let (t, m) = with_mock();
    t.tx(|tx| tx.assert(v("a"), v("p"), v("b"), Valid::ALWAYS).map(|_| ()));
    let p = Op::Path(PathPattern {
        start: TermOrVar::var("s"),
        end: TermOrVar::iri(vi("acme")),
        path: PathExpr::iri(vi("worksAt")).plus(),
        mode: PathMode::Reachability,
        max_hops: Some(15),
        bind_path: None,
        view: View::NOW,
        graph: tm_ir::GraphSel::Any,
    });
    let q = b().query(p);
    // acme is not in the dictionary yet: the pattern short-circuits
    assert!(explain(&t.db.now(), &q).short_circuit);
    assert!(run(&t.db.now(), &q).is_empty());
    t.tx(|tx| {
        tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)
            .map(|_| ())
    });
    let ex = explain(&t.db.now(), &q);
    assert!(ex.regions.iter().any(|r| r.note == RouteNote::PathInverted));
    assert!(
        ex.params
            .contains(&SqlValue::Text("^<urn:tiramemsu:v:worksAt>+".into())),
        "{:?}",
        ex.params
    );
    assert!(ex.params.contains(&SqlValue::Integer(15)));
    // the mock (start = the bound end) binds ?s from the `end` column
    let r = run(&t.db.now(), &q);
    assert_eq!(rows(&r), expect(&[&["acme"]]));
    assert_eq!(m.calls.lock().unwrap().len(), 1);
}

// sql-execution "Plan per region"
#[test]
fn plan_per_region() {
    let (t, _) = with_mock();
    t.tx(|tx| {
        tx.assert(v("b1"), v("supportedBy"), v("r1"), Valid::ALWAYS)?;
        tx.assert(v("b1"), v("kind"), v("k"), Valid::ALWAYS)?;
        Ok(())
    });
    let q = b().query(Op::join(vec![
        b().triple("?b", "v:supportedBy", "?r"),
        b().triple("?b", "v:kind", "?k"),
        supported_by_path(&b()),
    ]));
    let ex = explain(&t.db.now(), &q);
    let sql = ex.sql_region().expect("sql region");
    assert!(!sql.query_plan.is_empty());
    assert!(
        sql.query_plan
            .iter()
            .all(|l| l.contains("t0") || l.contains("t1")),
        "{:?}",
        sql.query_plan
    );
    assert_eq!(sql.aliases.len(), 2);
}

// sql-execution "Explain a normal query" / "Explain a short-circuited query"
#[test]
fn explain_normal_and_short_circuit() {
    let t = TestDb::new();
    let mut e = None;
    t.tx(|tx| {
        e = Some(
            tx.assert(v("a"), v("status"), v("on"), Valid::ALWAYS)?
                .eid(),
        );
        Ok(())
    });
    t.tx(|tx| tx.retract(e.unwrap()).map(|_| ()));
    t.tx(|tx| {
        tx.assert(v("a"), v("status"), v("off"), Valid::ALWAYS)
            .map(|_| ())
    });
    let q = b().query(b().triple("?s", "v:status", "?o"));
    let ex = explain(&t.db.now(), &q);
    assert!(!ex.short_circuit);
    assert_eq!(
        ex.regions
            .iter()
            .filter(|r| r.kind == RegionKind::Sql)
            .count(),
        1
    );
    assert!(ex.sql.is_some() && !ex.params.is_empty());
    assert!(
        ex.query_plan.join("\n").contains("live_"),
        "{:?}",
        ex.query_plan
    );
    let q = b().query(b().triple("?s", "urn:never-seen", "?o"));
    let ex = explain(&t.db.now(), &q);
    assert!(ex.short_circuit && ex.sql.is_none());
}

fn path_nodes(text: &str) -> Vec<String> {
    // the decoded path text is `{"nodes":["urn:…",…],"edges":[…]}`
    let nodes = text
        .split("\"nodes\":[")
        .nth(1)
        .unwrap()
        .split(']')
        .next()
        .unwrap();
    nodes
        .split(',')
        .map(|n| {
            n.trim_matches('"')
                .strip_prefix("urn:tiramemsu:v:")
                .unwrap()
                .to_string()
        })
        .collect()
}

// path-evaluation "Only the end is bound" (through the IR) / task 10.2: the far end,
// `"end" = ?` pushdown, and the path of an end-bound pattern in start-to-end order
#[test]
fn end_bound_pattern_binds_the_path_forwards() {
    let t = TestDb::new();
    t.tx(|tx| {
        tx.assert(v("a"), v("knows"), v("b"), Valid::ALWAYS)?;
        tx.assert(v("b"), v("knows"), v("c"), Valid::ALWAYS)?;
        Ok(())
    });
    let mut pat = PathPattern {
        start: TermOrVar::var("s"),
        end: TermOrVar::iri(vi("c")),
        path: PathExpr::iri(vi("knows")).plus(),
        mode: PathMode::Trail,
        max_hops: Some(15),
        bind_path: Some("p".into()),
        view: View::NOW,
        graph: tm_ir::GraphSel::Any,
    };
    let q = b().query(Op::Path(pat.clone()));
    let ex = explain(&t.db.now(), &q);
    assert!(ex.regions.iter().any(|r| r.note == RouteNote::PathInverted));
    assert!(ex
        .params
        .contains(&SqlValue::Text("^<urn:tiramemsu:v:knows>+".into())));
    let r = run(&t.db.now(), &q);
    let mut got: Vec<(String, Vec<String>)> = (0..r.len())
        .map(|i| {
            let Some(Value::Str(p)) = r.get(i, "p") else {
                panic!("path text")
            };
            (short(r.get(i, "s").unwrap()), path_nodes(p))
        })
        .collect();
    got.sort();
    assert_eq!(
        got,
        vec![
            ("a".to_string(), vec!["a".into(), "b".into(), "c".into()]),
            ("b".to_string(), vec!["b".into(), "c".into()]),
        ]
    );
    // both endpoints constant and the same variable at both ends
    pat.start = TermOrVar::iri(vi("a"));
    pat.bind_path = None;
    pat.mode = PathMode::Reachability;
    let both = b().query(Op::Path(pat.clone()));
    let sql = explain(&t.db.now(), &both).sql.unwrap();
    assert!(
        sql.contains("\"end\" = ?"),
        "the end constraint is pushed down: {sql}"
    );
    assert_eq!(run(&t.db.now(), &both).len(), 1);
    t.tx(|tx| {
        tx.assert(v("c"), v("knows"), v("a"), Valid::ALWAYS)
            .map(|_| ())
    });
    let same = b().query(Op::join(vec![
        Op::Values(tiramemsu::ir::Values {
            vars: vec!["x".into()],
            rows: vec![vec![Some(TermOrVar::iri(vi("a")))]],
        }),
        Op::Path(PathPattern {
            start: TermOrVar::var("x"),
            end: TermOrVar::var("x"),
            path: PathExpr::iri(vi("knows")).plus(),
            mode: PathMode::Reachability,
            max_hops: None,
            bind_path: None,
            view: View::NOW,
            graph: tm_ir::GraphSel::Any,
        }),
    ]));
    assert_eq!(run(&t.db.now(), &same).len(), 1);
}

// task 10.3: a nullable path from a term that is in no statement
#[test]
fn zero_length_match_of_an_unknown_constant() {
    let t = TestDb::new();
    t.tx(|tx| {
        tx.assert(v("a"), v("knows"), v("b"), Valid::ALWAYS)
            .map(|_| ())
    });
    let star = |s: TermOrVar, e: TermOrVar, p: PathExpr| {
        b().query(Op::Path(PathPattern {
            start: s,
            end: e,
            path: p,
            mode: PathMode::Reachability,
            max_hops: None,
            bind_path: None,
            view: View::NOW,
            graph: tm_ir::GraphSel::Any,
        }))
    };
    let nobody = || TermOrVar::iri(vi("nobody"));
    let knows = || PathExpr::iri(vi("knows"));
    let r = run(
        &t.db.now(),
        &star(nobody(), TermOrVar::var("x"), knows().star()),
    );
    assert_eq!(rows(&r), expect(&[&["nobody"]]));
    let r = run(
        &t.db.now(),
        &star(TermOrVar::var("x"), nobody(), knows().star()),
    );
    assert_eq!(rows(&r), expect(&[&["nobody"]]));
    assert!(run(
        &t.db.now(),
        &star(nobody(), TermOrVar::var("x"), knows().plus())
    )
    .is_empty());
    let both = run(&t.db.now(), &star(nobody(), nobody(), knows().star()));
    assert_eq!(both.len(), 1);
    assert!(run(
        &t.db.now(),
        &star(nobody(), TermOrVar::iri(vi("other")), knows().star())
    )
    .is_empty());
}

// task 10.1: golden `explain_ir` output of the four anchor shapes
#[test]
fn anchor_shapes_golden() {
    let t = TestDb::new();
    t.tx(|tx| {
        tx.assert(v("a"), v("knows"), v("b"), Valid::ALWAYS)
            .map(|_| ())
    });
    let pat = |s: TermOrVar, e: TermOrVar, max: Option<u32>| {
        Op::Path(PathPattern {
            start: s,
            end: e,
            path: PathExpr::iri(vi("knows")).plus(),
            mode: PathMode::Trail,
            max_hops: max,
            bind_path: None,
            view: View::as_of_tx(1),
            graph: tm_ir::GraphSel::Any,
        })
    };
    let a = || TermOrVar::iri(vi("a"));
    let bb = || TermOrVar::iri(vi("b"));
    let x = || TermOrVar::var("x");
    let cases = [
        ("start bound", pat(a(), x(), None)),
        ("end bound", pat(x(), bb(), Some(15))),
        ("both bound", pat(a(), bb(), None)),
        (
            "same variable",
            Op::join(vec![
                Op::Values(tiramemsu::ir::Values {
                    vars: vec!["x".into()],
                    rows: vec![vec![Some(a())]],
                }),
                pat(x(), x(), None),
            ]),
        ),
    ];
    let mut out = String::new();
    for (name, op) in cases {
        let ex = explain(&t.db.now(), &b().query(op));
        out.push_str(&format!(
            "## {name}\n{}\n{:?}\n{:?}\n\n",
            ex.sql.unwrap(),
            ex.params,
            ex.regions
                .iter()
                .map(|r| (r.kind, r.note))
                .collect::<Vec<_>>()
        ));
    }
    insta::assert_snapshot!(out);
}
