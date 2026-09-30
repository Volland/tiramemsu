//! Native path regions: routing, `tm_path` composition with a mock operator.

mod common;

use std::sync::Arc;

use common::mock::MockPath;
use common::probe::ProbeHost;
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

// sql-execution "No path operator registered"
#[test]
fn no_operator_is_unsupported_without_sql() {
    let d = tempfile::tempdir().unwrap();
    let host = ProbeHost::new();
    let db =
        Db::open_with_host(host.clone(), d.path().join("p.db"), OpenOptions::default()).unwrap();
    db.transact(TxOptions::default(), |tx| {
        tx.assert(v("a"), v("p"), v("b"), Valid::ALWAYS).map(|_| ())
    })
    .unwrap();
    let q = b().query(Op::join(vec![
        b().triple("?b", "v:p", "?r"),
        supported_by_path(&b()),
    ]));
    host.log.clear();
    match db.now().execute_ir(&q, &Params::new()) {
        Err(Error::Unsupported { feature }) => {
            assert!(feature.contains("path patterns"), "{feature}")
        }
        other => panic!("{other:?}"),
    }
    assert!(host
        .log
        .statements()
        .iter()
        .all(|s| !s.contains("FROM triple")));
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
        ex.params.contains(&SqlValue::Text("^v:worksAt+".into())),
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
