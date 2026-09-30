//! Spec `query-ir`: operators, per-pattern views, time references, semantic
//! flags, eid binding, constants at plan time, short-circuiting, parameters and
//! structural validation.

mod common;

use common::probe::ProbeHost;
use common::*;
use tiramemsu::ir::builder::IrBuilder;
use tiramemsu::ir::*;
use tiramemsu::ir::{Op, View};
use tiramemsu::*;

fn b() -> IrBuilder {
    IrBuilder::sparql()
}

fn people() -> TestDb {
    let t = TestDb::new();
    t.tx(|tx| {
        tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?;
        tx.assert(v("acme"), v("name"), s("Acme Corp"), Valid::ALWAYS)?;
        tx.assert(v("alice"), v("name"), s("Alice"), Valid::ALWAYS)?;
        tx.assert(v("bob"), v("name"), s("Bob"), Valid::ALWAYS)?;
        tx.assert(v("alice"), v("age"), Value::Int(30), Valid::ALWAYS)?;
        tx.assert(v("bob"), v("age"), Value::Int(25), Valid::ALWAYS)?;
        tx.assert(v("alice"), v("email"), s("a@example.org"), Valid::ALWAYS)?;
        tx.assert(
            v("alice"),
            v("email"),
            s("alice@work.example"),
            Valid::ALWAYS,
        )?;
        Ok(())
    });
    t
}

// query-ir "Basic graph pattern as a join of triple patterns"
#[test]
fn basic_graph_pattern() {
    let t = people();
    let q = b().query(b().bgp(&[("?a", "v:worksAt", "?c"), ("?c", "v:name", "?n")]));
    let r = run(&t.db.now(), &q);
    assert_eq!(cols(&r), ["a", "c", "n"]);
    assert_eq!(rows(&r), expect(&[&["alice", "acme", "Acme Corp"]]));
}

// query-ir "Nested composition"
#[test]
fn nested_composition() {
    let t = people();
    let root = Op::left_join(
        b().bgp(&[("?a", "v:name", "?n")]),
        Op::union(vec![
            b().triple("?a", "v:age", "?x"),
            b().triple("?a", "v:email", "?x"),
        ]),
        None,
    )
    .filter(Expr::ne(Expr::var("n"), Expr::val(s("x"))))
    .project(&["a"]);
    let r = run(&t.db.now(), &b().query(root));
    assert_eq!(cols(&r), ["a"]);
    // acme: no age/email (kept once); alice: age + 2 emails; bob: age
    assert_eq!(
        sorted(&r),
        expect(&[&["acme"], &["alice"], &["alice"], &["alice"], &["bob"]])
    );
}

// query-ir "Empty join is the unit relation"
#[test]
fn empty_join_is_unit() {
    let t = people();
    let q = b().query(Op::unit().extend("x", Expr::val(Value::Int(1))));
    let r = run(&t.db.now(), &q);
    assert_eq!(rows(&r), expect(&[&["1"]]));
}

fn values(vars: &[&str], rows: Vec<Vec<Option<Value>>>) -> Op {
    Op::Values(Values {
        vars: vars.iter().map(Var::new).collect(),
        rows: rows
            .into_iter()
            .map(|r| r.into_iter().map(|c| c.map(TermOrVar::Const)).collect())
            .collect(),
    })
}

// query-ir "Values provides inline bindings"
#[test]
fn values_inline_bindings() {
    let t = TestDb::new();
    t.tx(|tx| {
        tx.assert(v("alice"), v("age"), Value::Int(30), Valid::ALWAYS)?;
        tx.assert(v("bob"), v("name"), s("Bob"), Valid::ALWAYS)?;
        Ok(())
    });
    let q = b().query(Op::join(vec![
        values(&["p"], vec![vec![Some(v("alice"))], vec![Some(v("bob"))]]),
        b().triple("?p", "v:age", "?age"),
    ]));
    assert_eq!(rows(&run(&t.db.now(), &q)), expect(&[&["alice", "30"]]));
}

// query-ir "Values with an undefined cell"
#[test]
fn values_undefined_cell() {
    let t = people();
    let q = b().query(values(
        &["x", "y"],
        vec![
            vec![Some(v("alice")), None],
            vec![Some(v("bob")), Some(Value::Int(1))],
        ],
    ));
    let r = run(&t.db.now(), &q);
    assert_eq!(sorted(&r), expect(&[&["alice", "-"], &["bob", "1"]]));
    assert_eq!(r.rows[0][1], None);
    // an unknown value in a Values cell is returned as-is
    let q = b().query(values(&["x"], vec![vec![Some(s("never stored anywhere"))]]));
    assert_eq!(
        rows(&run(&t.db.now(), &q)),
        expect(&[&["never stored anywhere"]])
    );
}

// query-ir "Semi-join"
#[test]
fn exists_semi_join() {
    let t = people();
    let q = b().query(
        b().triple("?p", "v:name", "?n")
            .filter(Expr::exists(b().triple("?p", "v:email", "?e"))),
    );
    let r = run(&t.db.now(), &q);
    assert_eq!(cols(&r), ["p", "n"], "no ?e column");
    assert_eq!(rows(&r), expect(&[&["alice", "Alice"]]));
}

// query-ir "Anti-join"
#[test]
fn exists_anti_join() {
    let t = people();
    let q = b().query(
        b().triple("?p", "v:name", "?n")
            .filter(Expr::not_exists(b().triple("?p", "v:email", "?e"))),
    );
    let r = run(&t.db.now(), &q);
    assert_eq!(
        sorted(&r),
        expect(&[&["acme", "Acme Corp"], &["bob", "Bob"]])
    );
}

// query-ir "Correlation through shared variables only"
#[test]
fn exists_uncorrelated() {
    let t = people();
    let yes = b().query(
        b().triple("?p", "v:name", "?n")
            .filter(Expr::exists(b().triple("?x", "v:worksAt", "?y"))),
    );
    assert_eq!(run(&t.db.now(), &yes).len(), 3);
    let no = b().query(
        b().triple("?p", "v:name", "?n")
            .filter(Expr::exists(b().triple("?x", "v:age", Value::Int(99)))),
    );
    assert_eq!(run(&t.db.now(), &no).len(), 0);
}

// query-ir "Nested tree with a constant missing from the dictionary"
#[test]
fn exists_with_unknown_constant() {
    let t = people();
    let inner = || b().triple("?p", "urn:never-seen", "?x");
    let q = b().query(
        b().triple("?p", "v:name", "?n")
            .filter(Expr::exists(inner())),
    );
    assert_eq!(run(&t.db.now(), &q).len(), 0);
    let q = b().query(
        b().triple("?p", "v:name", "?n")
            .filter(Expr::not_exists(inner())),
    );
    assert_eq!(run(&t.db.now(), &q).len(), 3);
}

// query-ir "Project fixes column order" / "Implicit column order"
#[test]
fn column_order() {
    let t = people();
    let q = b().query(b().triple("?a", "v:name", "?n").project(&["n", "a"]));
    assert_eq!(cols(&run(&t.db.now(), &q)), ["n", "a"]);
    let q = b().query(b().bgp(&[("?a", "v:worksAt", "?b"), ("?b", "v:name", "?c")]));
    assert_eq!(cols(&run(&t.db.now(), &q)), ["a", "b", "c"]);
}

/// Alice works at acme until tx 200, which supersedes it into globex.
fn superseded() -> (TestDb, Eid) {
    let t = TestDb::new();
    let mut e1 = None;
    t.tx(|tx| {
        e1 = Some(
            tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?
                .eid(),
        );
        Ok(())
    });
    t.advance_to(199);
    let e1 = e1.unwrap();
    t.tx(|tx| {
        tx.supersede(e1, Patch::object(v("globex")))?;
        Ok(())
    });
    assert_eq!(t.last_t(), 200);
    (t, e1)
}

// query-ir "Two patterns under different views in one query"
#[test]
fn per_pattern_time_scopes() {
    let (t, _) = superseded();
    let before = b()
        .at(View::as_of_tx(150))
        .triple("v:alice", "v:worksAt", "?before");
    let after = b().triple("v:alice", "v:worksAt", "?after");
    let q = b().query(
        Op::join(vec![before, after]).filter(Expr::ne(Expr::var("before"), Expr::var("after"))),
    );
    let r = run(&t.db.now(), &q);
    assert_eq!(rows(&r), expect(&[&["acme", "globex"]]));
    // the explained SQL has one AsOf alias and one Now alias
    let sql = explain(&t.db.now(), &q).sql.unwrap();
    assert!(sql.contains("t0.t_add <= ?"), "{sql}");
    assert!(sql.contains("t1.t_ret IS NULL"), "{sql}");
    // three views in one query: Now, AsOf(Tx(3)) and {History, At(d)}
    let three = b().query(Op::join(vec![
        b().triple("v:alice", "v:worksAt", "?now"),
        b().at(View::as_of_tx(3))
            .triple("v:alice", "v:worksAt", "?then"),
        b().at(View::history().valid_at(5))
            .triple("v:alice", "v:worksAt", "?ever"),
    ]));
    let r = run(&t.db.now(), &three);
    assert_eq!(
        sorted(&r),
        expect(&[&["globex", "acme", "acme"], &["globex", "acme", "globex"]])
    );
    let sql = explain(&t.db.now(), &three).sql.unwrap();
    assert!(sql.contains("t0.t_ret IS NULL"));
    assert!(sql.contains("t1.t_add <= ?") && !sql.contains("t0.t_add"));
    assert!(sql.contains("t2.v_from") && !sql.contains("t2.t_ret"));
    assert!(!sql.contains("t0.v_from") && !sql.contains("t1.v_from"));
}

// query-ir "The handle view does not override explicit pattern views"
#[test]
fn handle_view_does_not_override() {
    let (t, _) = superseded();
    let q = b().query(b().triple("v:alice", "v:worksAt", "?c"));
    let r = run(&t.db.as_of(TimeRef::Tx(10)), &q);
    assert_eq!(rows(&r), expect(&[&["globex"]]));
}

// query-ir "Handle view as the lowering default"
#[test]
fn handle_descriptor() {
    let t = TestDb::new();
    let d = 1_740_787_200_000;
    let v = t.db.as_of(TimeRef::Tx(10)).valid_at(d).descriptor();
    assert_eq!(
        v,
        View {
            tx: TxSel::AsOf(TimeRef::Tx(10)),
            valid: ValidSel::At(d)
        }
    );
    assert_eq!(t.db.now().descriptor(), View::NOW);
}

// query-ir "Instant resolves to a transaction" / "Instant before the first transaction"
#[test]
fn instants_resolve_at_plan_time() {
    let t = TestDb::new();
    for (i, ms) in [1000, 2000, 3000].into_iter().enumerate() {
        t.clock.set(ms);
        t.tx(|tx| {
            tx.assert(v("x"), v("n"), Value::Int(i as i64 + 1), Valid::ALWAYS)?;
            Ok(())
        });
    }
    let q = |at: TimeRef| b().query(b().at(View::as_of(at)).triple("v:x", "v:n", "?n"));
    let r = run(&t.db.now(), &q(TimeRef::Instant(2500)));
    assert_eq!(sorted(&r), expect(&[&["1"], &["2"]]));
    assert_eq!(sorted(&r), sorted(&run(&t.db.now(), &q(TimeRef::Tx(2)))));
    let r = run(&t.db.now(), &q(TimeRef::Instant(500)));
    assert!(r.is_empty());
    assert!(!r.stats.sql_executed, "no SQL for a pre-history pattern");
}

// query-ir "Same tree, different graph_set"
#[test]
fn same_tree_different_graph_set() {
    let t = TestDb::new();
    t.tx(|tx| {
        tx.create(v("alice"), v("knows"), v("bob"), Valid::ALWAYS)?;
        tx.create(v("alice"), v("knows"), v("bob"), Valid::ALWAYS)?;
        Ok(())
    });
    let op = b().triple("?a", "v:knows", "?b");
    let set = IrQuery::new(op.clone(), Semantics::sparql());
    let bag = IrQuery::new(op, Semantics::sparql().with_graph_set(GraphSet::BagOfEids));
    assert_eq!(run(&t.db.now(), &set).len(), 1);
    assert_eq!(run(&t.db.now(), &bag).len(), 2);
}

// query-ir "Annotation on a statement" / "A belief referencing a statement"
#[test]
fn eid_binding_reaches_layers() {
    let f = layers();
    let e1 = f.eid("e1");
    let q = b().query(Op::join(vec![
        Op::Triple(b().t("v:alice", "v:worksAt", "?c").with_eid("?r")),
        b().triple("?r", "v:confidence", "?conf"),
    ]));
    let r = run(&f.db().now(), &q);
    assert_eq!(r.len(), 1);
    assert_eq!(r.get(0, "r"), Some(&Value::Stmt(e1)));
    assert_eq!(r.get(0, "conf"), Some(&Value::Double(0.8)));
    let q = b().query(Op::join(vec![
        Op::Triple(b().t("?a", "v:worksAt", "?c").with_eid("?r")),
        b().triple("?b", "v:supportedBy", "?r"),
    ]));
    let r = run(&f.db().now(), &q);
    assert_eq!(
        rows(&r),
        expect(&[&["alice", "acme", &e1.to_string(), "belief9"]])
    );
}

// query-ir "Same eid variable in two patterns" / "Eid equal to a constant"
#[test]
fn eid_variable_shared_and_constant() {
    let f = layers();
    let q = b().query(Op::join(vec![
        Op::Triple(b().t("?s", "v:worksAt", "?o").with_eid("?r")),
        Op::Triple(b().t("?s", "v:name", "?o").with_eid("?r")),
    ]));
    assert!(run(&f.db().now(), &q).is_empty());
    let e1 = f.eid("e1");
    let q = b().query(
        Op::Triple(b().t("?s", "?p", "?o").with_eid("?r"))
            .filter(Expr::eq(Expr::var("r"), Expr::val(Value::Stmt(e1)))),
    );
    let r = run(&f.db().now(), &q);
    assert_eq!(
        rows(&r),
        expect(&[&["alice", "worksAt", "acme", &e1.to_string()]])
    );
    // not visible before it existed
    let asof = b().at(View::as_of_tx(0));
    let q = asof.query(
        Op::Triple(asof.t("?s", "?p", "?o").with_eid("?r"))
            .filter(Expr::eq(Expr::var("r"), Expr::val(Value::Stmt(e1)))),
    );
    assert!(run(&f.db().now(), &q).is_empty());
}

fn probe_db() -> (tempfile::TempDir, Db, ProbeHost) {
    let dir = tempfile::tempdir().unwrap();
    let host = ProbeHost::new();
    let db = Db::open_with_host(
        host.clone(),
        dir.path().join("p.db"),
        OpenOptions::default(),
    )
    .unwrap();
    db.transact(TxOptions::default(), |tx| {
        tx.assert(v("x"), v("worksAt"), Value::Int(42), Valid::ALWAYS)?;
        tx.assert(v("y"), v("worksAt"), v("acme"), Valid::ALWAYS)?;
        Ok(())
    })
    .unwrap();
    (dir, db, host)
}

// query-ir "Inline constant needs no dictionary"
#[test]
fn inline_constant_needs_no_dictionary() {
    let (_d, db, host) = probe_db();
    let q = b().query(Op::Triple(TriplePattern::new(
        "?s",
        TermOrVar::Id(db.now().encode(&v("worksAt")).unwrap().unwrap()),
        Value::Int(42),
        View::NOW,
    )));
    host.log.clear();
    let r = run(&db.now(), &q);
    assert_eq!(rows(&r), expect(&[&["x"]]));
    // planning (everything before the query statement) reads no term
    let log = host.log.statements();
    let planning: Vec<&String> = log
        .iter()
        .take_while(|s| !s.contains("FROM triple AS t0"))
        .collect();
    assert!(planning.len() < log.len(), "{log:?}");
    assert!(
        planning.iter().all(|s| !s.contains("FROM term")),
        "no dictionary read while planning: {log:?}"
    );
}

// query-ir "Dictionary constant is found"
#[test]
fn dictionary_constant_is_found() {
    let (_d, db, _host) = probe_db();
    let q = b().query(b().triple("?s", "v:worksAt", "?o"));
    let ex = explain(&db.now(), &q);
    let id = db.now().encode(&v("worksAt")).unwrap().unwrap();
    assert!(ex.sql.unwrap().contains("t0.p = ?1"));
    assert_eq!(ex.params[0], SqlValue::Integer(id.raw()));
}

// query-ir "Planning does not write"
#[test]
fn planning_does_not_write() {
    let t = people();
    let snapshot = |t: &TestDb| {
        (
            t.db.read_sql("SELECT * FROM term ORDER BY id").unwrap(),
            t.db.read_sql("SELECT * FROM meta ORDER BY key").unwrap(),
        )
    };
    let before = snapshot(&t);
    let q = b().query(Op::join(vec![
        b().triple("?s", "urn:never-seen", "?o"),
        b().triple("?s", "v:name", s("an unknown long string")),
    ]));
    assert!(run(&t.db.now(), &q).is_empty());
    let q = b().query(b().triple("?s", "v:name", "?n").filter(Expr::lt(
        Expr::var("n"),
        Expr::val(s("another unknown string")),
    )));
    run(&t.db.now(), &q);
    assert_eq!(snapshot(&t), before);
}

// query-ir "Unknown IRI constant"
// @lat: [[tests#Query#Unknown Constant Short Circuits]]
#[test]
fn unknown_iri_short_circuits() {
    let (_d, db, host) = probe_db();
    let q = b().query(Op::join(vec![
        b().triple("?s", "urn:never-seen", "?o"),
        b().triple("?s", "v:worksAt", "?c"),
    ]));
    host.log.clear();
    let r = run(&db.now(), &q);
    assert!(r.is_empty() && !r.stats.sql_executed);
    assert_eq!(cols(&r), ["s", "o", "c"]);
    let log = host.log.statements();
    assert!(
        log.iter().all(|s| !s.contains("FROM triple")),
        "no query SQL: {log:?}"
    );
}

// query-ir "Unknown long string constant" / "Literal in predicate position"
#[test]
fn impossible_patterns_are_empty() {
    let t = people();
    let q = b().query(b().triple("?p", "v:name", s("a string longer than seven bytes")));
    assert!(run(&t.db.now(), &q).is_empty());
    let q = b().query(Op::Triple(TriplePattern::new(
        "?s",
        Value::Int(5),
        "?o",
        View::NOW,
    )));
    let r = run(&t.db.now(), &q);
    assert!(r.is_empty() && !r.stats.sql_executed);
    // a literal subject can never match
    let q = b().query(Op::Triple(TriplePattern::new(
        Value::Int(5),
        "?p",
        "?o",
        View::NOW,
    )));
    assert!(run(&t.db.now(), &q).is_empty());
}

// query-ir "Optional side missing from the dictionary"
#[test]
fn optional_side_unknown() {
    let t = people();
    let q = b().query(Op::left_join(
        b().triple("?p", "v:name", "?n"),
        b().triple("?p", "urn:unknown", "?x"),
        None,
    ));
    let r = run(&t.db.now(), &q);
    assert_eq!(cols(&r), ["p", "n", "x"]);
    assert_eq!(
        sorted(&r),
        expect(&[
            &["acme", "Acme Corp", "-"],
            &["alice", "Alice", "-"],
            &["bob", "Bob", "-"]
        ])
    );
}

// query-ir "Union with one empty branch"
#[test]
fn union_with_empty_branch() {
    let t = people();
    let q = b().query(Op::union(vec![
        b().triple("?p", "urn:unknown", "?x"),
        b().triple("?p", "v:age", "?y"),
    ]));
    let r = run(&t.db.now(), &q);
    assert_eq!(cols(&r), ["p", "x", "y"]);
    assert_eq!(
        sorted(&r),
        expect(&[&["alice", "-", "30"], &["bob", "-", "25"]])
    );
}

// query-ir "Count over an empty input" / "Grouped aggregate over an empty input"
#[test]
fn aggregates_over_empty_input() {
    let t = people();
    let empty = || b().triple("?c", "urn:unknown", "?x");
    let q = b().query(empty().aggregate(&[], vec![Agg::count_star("n")]));
    let r = run(&t.db.now(), &q);
    assert_eq!(rows(&r), expect(&[&["0"]]));
    assert!(!r.stats.sql_executed);
    let q = b().query(empty().aggregate(&["c"], vec![Agg::count_star("n")]));
    let r = run(&t.db.now(), &q);
    assert!(r.is_empty());
    assert_eq!(cols(&r), ["c", "n"]);
    // group-less over empty: count 0, collect [], others missing
    let q = b().query(empty().aggregate(
        &[],
        vec![
            Agg::count_star("n"),
            Agg::new("l", AggFunc::Collect, Expr::var("x")),
            Agg::new("m", AggFunc::Max, Expr::var("x")),
        ],
    ));
    assert_eq!(rows(&run(&t.db.now(), &q)), expect(&[&["0", "[]", "-"]]));
}

// query-ir "Parameter as a pattern constant" / "Missing parameter" /
// "Unknown parameter value short-circuits"
#[test]
fn parameters() {
    let t = people();
    let q = b().query(b().triple("?p", "v:email", "$email"));
    let with_param =
        t.db.now()
            .execute_ir(&q, &params([("email", s("a@example.org"))]))
            .unwrap();
    let with_const = run(
        &t.db.now(),
        &b().query(b().triple("?p", "v:email", s("a@example.org"))),
    );
    assert_eq!(rows(&with_param), rows(&with_const));
    assert_eq!(rows(&with_param), expect(&[&["alice"]]));
    match t.db.now().execute_ir(&q, &Params::new()) {
        Err(Error::InvalidQuery { msg }) => assert!(msg.contains("email"), "{msg}"),
        other => panic!("{other:?}"),
    }
    let q = b().query(b().triple("?p", "$pred", "?o"));
    let r =
        t.db.now()
            .execute_ir(&q, &params([("pred", v("neverStored"))]))
            .unwrap();
    assert!(r.is_empty() && !r.stats.sql_executed);
    // skip and limit as parameters
    let q = b().query(Op::OrderLimit(OrderLimit {
        input: Box::new(b().triple("?p", "v:name", "?n")),
        keys: vec![Key::asc(Expr::var("n"))],
        skip: Some(TermOrVar::param("s")),
        limit: Some(TermOrVar::param("l")),
    }));
    let r =
        t.db.now()
            .execute_ir(&q, &params([("s", Value::Int(1)), ("l", Value::Int(1))]))
            .unwrap();
    assert_eq!(rows(&r), expect(&[&["alice", "Alice"]]));
}

// query-ir "Extend rebinds a bound variable" / "Ragged Values" / "Path with no bound endpoint"
#[test]
fn structural_validation() {
    let t = people();
    let q = b().query(
        b().triple("?a", "v:p", "?b")
            .extend("a", Expr::val(Value::Int(1))),
    );
    assert!(matches!(
        t.db.now().execute_ir(&q, &Params::new()),
        Err(Error::InvalidQuery { .. })
    ));
    let c = |i: i64| Some(TermOrVar::Const(Value::Int(i)));
    let q = b().query(Op::Values(Values {
        vars: vec![Var::new("x"), Var::new("y")],
        rows: vec![vec![c(1), c(2), c(3)]],
    }));
    assert!(matches!(
        t.db.now().execute_ir(&q, &Params::new()),
        Err(Error::InvalidQuery { .. })
    ));
    let op = Arc::new(common::mock::MockPath::default());
    let t = TestDb::open(OpenOptions::default().with_native_operator(op));
    let q = b().query(b().path(
        "?a",
        PathExpr::iri(vi("p")).star(),
        "?b",
        PathMode::Reachability,
    ));
    assert!(matches!(
        t.db.now().execute_ir(&q, &Params::new()),
        Err(Error::Unsupported { .. })
    ));
}

use std::sync::Arc;
