//! Golden SQL snapshots (`insta`) for the corpus of spec `sql-execution` "Golden
//! SQL", plus the verbatim-live-predicate checks of `view-scoped-scans`.

mod common;

use std::sync::Arc;

use common::mock::MockPath;
use common::*;
use tiramemsu::ir::builder::IrBuilder;
use tiramemsu::ir::*;
use tiramemsu::ir::{Op, View};
use tiramemsu::*;

fn b() -> IrBuilder {
    IrBuilder::sparql()
}

/// A store in which every constant of the corpus exists (so nothing
/// short-circuits), with a parallel edge so `v:called` needs duplicate removal.
fn corpus_db() -> TestDb {
    let t =
        TestDb::open(OpenOptions::default().with_native_operator(Arc::new(MockPath::default())));
    t.tx(|tx| {
        for p in [
            "worksAt",
            "locatedIn",
            "name",
            "age",
            "email",
            "knows",
            "supportedBy",
            "p",
            "q",
            "confidence",
        ] {
            tx.assert(v("alice"), v(p), v("acme"), Valid::ALWAYS)?;
        }
        tx.create(v("alice"), v("called"), v("bob"), Valid::ALWAYS)?;
        tx.create(v("alice"), v("called"), v("bob"), Valid::ALWAYS)?;
        Ok(())
    });
    t
}

fn snap(t: &TestDb, name: &str, q: &IrQuery) {
    let ex = explain(&t.db.now(), q);
    let sql = ex.sql.unwrap_or_else(|| "-- short-circuit".to_string());
    let text = format!("{}\n-- params: {}", sql, ex.params.len());
    insta::with_settings!({ snapshot_suffix => name, prepend_module_to_snapshot => false }, {
        insta::assert_snapshot!("golden", text);
    });
}

#[test]
fn single_pattern_per_view() {
    let t = corpus_db();
    let d = 1_772_323_200_000;
    let views = [
        ("now", View::NOW),
        ("asof", View::as_of_tx(1)),
        ("history", View::history()),
        ("now_at", View::NOW.valid_at(d)),
        ("asof_at", View::as_of_tx(1).valid_at(d)),
        ("history_at", View::history().valid_at(d)),
    ];
    for (n, view) in views {
        let bb = b().at(view);
        snap(
            &t,
            &format!("single_{n}"),
            &bb.query(bb.triple("v:alice", "v:worksAt", "?o")),
        );
    }
}

#[test]
fn shapes() {
    let t = corpus_db();
    let bg = |ps: &[(&str, &str, &str)]| b().query(b().bgp(ps));
    snap(
        &t,
        "star",
        &bg(&[
            ("?x", "v:name", "?n"),
            ("?x", "v:age", "?a"),
            ("?x", "v:email", "?e"),
        ]),
    );
    snap(
        &t,
        "chain",
        &bg(&[
            ("?a", "v:knows", "?b"),
            ("?b", "v:knows", "?c"),
            ("?c", "v:knows", "?d"),
        ]),
    );
    // layer join under mixed views, eid bound on the first pattern
    let asof = b().at(View::as_of_tx(1));
    let layer = Op::join(vec![
        Op::Triple(b().t("?a", "v:worksAt", "?c").with_eid("?r")),
        asof.triple("?r", "v:confidence", "?conf"),
    ]);
    snap(&t, "layer_join", &b().query(layer));
    let opt = Op::left_join(
        b().triple("?p", "v:name", "?n"),
        b().triple("?p", "v:email", "?e"),
        Some(Expr::gt(Expr::var("e"), Expr::val(Value::Int(1)))),
    );
    snap(&t, "optional", &b().query(opt));
    snap(
        &t,
        "union",
        &b().query(Op::union(vec![
            b().triple("?x", "v:p", "?y"),
            b().triple("?x", "v:q", "?z"),
        ])),
    );
    snap(
        &t,
        "filter",
        &b().query(b().triple("?x", "v:age", "?a").filter(Expr::And(vec![
            Expr::gt(Expr::var("a"), Expr::val(Value::Int(30))),
            Expr::Not(Box::new(Expr::eq(
                Expr::var("a"),
                Expr::val(Value::Int(40)),
            ))),
        ]))),
    );
    snap(
        &t,
        "aggregate",
        &b().query(b().triple("?x", "v:worksAt", "?c").aggregate(
            &["c"],
            vec![
                Agg::count_star("n"),
                Agg::new("m", AggFunc::Max, Expr::var("x")),
            ],
        )),
    );
    snap(
        &t,
        "order_limit",
        &b().query(
            b().triple("?x", "v:name", "?n")
                .order_limit(vec![Key::desc(Expr::var("n"))], Some(2), Some(10))
                .project(&["n"]),
        ),
    );
    let vals = Op::Values(Values {
        vars: vec!["x".into(), "y".into()],
        rows: vec![
            vec![Some(TermOrVar::iri(vi("alice"))), None],
            vec![
                Some(TermOrVar::iri(vi("bob"))),
                Some(TermOrVar::Const(Value::Int(1))),
            ],
        ],
    });
    snap(
        &t,
        "values",
        &b().query(Op::join(vec![vals, b().triple("?x", "v:name", "?n")])),
    );
    snap(
        &t,
        "virtual",
        &b().query(Op::join(vec![
            Op::Triple(b().t("?a", "v:worksAt", "?c").with_eid("?r")),
            b().triple("?r", "tm:txAdded", "?t"),
        ])),
    );
    let c = IrBuilder::cypher();
    snap(
        &t,
        "isomorphism",
        &c.query(Op::join(vec![
            Op::Triple(c.t("?x", "v:knows", "?y").with_eid("?r1").in_group(1)),
            Op::Triple(c.t("?y", "v:knows", "?z").with_eid("?r2").in_group(1)),
        ])),
    );
    snap(&t, "dedup_multi", &bg(&[("v:alice", "v:called", "?x")]));
    let path = Op::join(vec![
        b().triple("?b", "v:supportedBy", "?r"),
        b().path(
            "?r",
            PathExpr::iri(tiramemsu::ir::vocab::SYS_SUBJECT).star(),
            "?x",
            PathMode::Reachability,
        ),
    ]);
    snap(&t, "path_tvf", &b().query(path));
}

// sql-execution "Stable text across constant values" over the corpus
#[test]
fn text_is_stable_across_constants_and_times() {
    let t = corpus_db();
    let text = |who: &str, tx: u64, d: i64| {
        let bb = b().at(View::as_of_tx(tx).valid_at(d));
        explain(&t.db.now(), &bb.query(bb.triple(who, "v:worksAt", "?o")))
            .sql
            .unwrap()
    };
    assert_eq!(text("v:alice", 1, 5), text("v:acme", 9, 77));
}

// view-scoped-scans "Live predicate text" / "Optional pattern keeps its view in ON" /
// "Time values are bound parameters" / "Same text for different t"
#[test]
fn verbatim_live_predicate_and_optional_on() {
    let t = corpus_db();
    let q = b().query(Op::left_join(
        b().triple("?p", "v:name", "?n"),
        b().triple("?p", "v:email", "?e"),
        None,
    ));
    let sql = explain(&t.db.now(), &q).sql.unwrap();
    assert!(sql.contains("t0.t_ret IS NULL"), "{sql}");
    let (from, rest) = sql.split_once(" LEFT JOIN ").expect("left join");
    let (on, wher) = rest.split_once(" WHERE ").unwrap_or((rest, ""));
    assert!(on.contains("t1.t_ret IS NULL"), "ON: {on}");
    assert!(!wher.contains("t1.t_ret"), "WHERE: {wher}");
    assert!(!from.contains("t1.t_ret"));
    // every Now alias text contains `tN.t_ret IS NULL` verbatim
    let multi = b().query(b().bgp(&[
        ("?x", "v:name", "?n"),
        ("?x", "v:age", "?a"),
        ("?x", "v:email", "?e"),
    ]));
    let sql = explain(&t.db.now(), &multi).sql.unwrap();
    for i in 0..3 {
        assert!(sql.contains(&format!("t{i}.t_ret IS NULL")), "{sql}");
    }
    // time values are parameters
    let asof = |n: u64| {
        let bb = b().at(View::as_of_tx(n));
        explain(
            &t.db.now(),
            &bb.query(bb.triple("v:alice", "v:worksAt", "?o")),
        )
    };
    let (a, c) = (asof(10), asof(99));
    assert_eq!(a.sql, c.sql);
    assert!(!a.sql.unwrap().contains("99"));
    assert!(c.params.contains(&SqlValue::Integer(99)) && a.params.contains(&SqlValue::Integer(10)));
}
