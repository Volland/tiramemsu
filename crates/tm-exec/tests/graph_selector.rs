//! Spec `named-graphs` (planner half): a graph selector on a triple pattern lowers
//! to a membership join under the pattern's own view.

mod common;

use common::skewed::*;
use common::*;
use tiramemsu::ir::builder::IrBuilder;
use tiramemsu::ir::*;
use tiramemsu::ir::{Op, View};
use tiramemsu::*;

fn b() -> IrBuilder {
    IrBuilder::sparql()
}

fn g(name: &str) -> TermOrVar {
    TermOrVar::iri(format!("urn:tiramemsu:g:{name}"))
}

fn gv(name: &str) -> Value {
    Value::iri(format!("urn:tiramemsu:g:{name}"))
}

/// `?s v:p ?o` restricted by `sel`, projected to `s` (and `g` when it is bound).
fn sel_query(sel: GraphSel, project: &[&str]) -> IrQuery {
    b().query(Op::Triple(b().t("?s", "v:p", "?o").in_graph(sel)).project(project))
}

fn fixture() -> TestDb {
    let t = TestDb::new();
    t.tx(|tx| {
        let e1 = tx.assert(v("a"), v("p"), v("x"), Valid::ALWAYS)?.eid();
        let e2 = tx.assert(v("b"), v("p"), v("y"), Valid::ALWAYS)?.eid();
        tx.assert(v("c"), v("p"), v("z"), Valid::ALWAYS)?; // in no graph
        tx.add_to_graph(e1, gv("1"), AssertOpts::default())?;
        tx.add_to_graph(e1, gv("2"), AssertOpts::default())?;
        tx.add_to_graph(e2, gv("2"), AssertOpts::default())?;
        Ok(())
    });
    t
}

// @lat: [[tests#Named Graphs#Graph Selector Lowers To Membership Join]]
#[test]
fn selector_lowers_to_membership_join() {
    let t = fixture();
    let now = t.db.now();
    // Any: unchanged, everything
    let all = run(&now, &sel_query(GraphSel::Any, &["s"]));
    assert_eq!(sorted(&all), expect(&[&["a"], &["b"], &["c"]]));
    // Set of one: a join on the membership
    let one = run(&now, &sel_query(GraphSel::Set(vec![g("1")]), &["s"]));
    assert_eq!(rows(&one), expect(&[&["a"]]));
    // Set of several: each statement once, though `a` is in both graphs
    let many = run(
        &now,
        &sel_query(GraphSel::Set(vec![g("1"), g("2")]), &["s"]),
    );
    assert_eq!(sorted(&many), expect(&[&["a"], &["b"]]));
    // Var: one solution per membership
    let var = run(&now, &sel_query(GraphSel::Var("g".into()), &["s", "g"]));
    assert_eq!(
        sorted(&var),
        expect(&[
            &["a", "<urn:tiramemsu:g:1>"],
            &["a", "<urn:tiramemsu:g:2>"],
            &["b", "<urn:tiramemsu:g:2>"]
        ])
    );
    // unknown graph: nothing, and no error
    let none = run(&now, &sel_query(GraphSel::Set(vec![g("nope")]), &["s"]));
    assert!(none.rows.is_empty());
    // a parameter names the graph
    let q = sel_query(GraphSel::Set(vec![TermOrVar::param("gr")]), &["s"]);
    let r = now
        .execute_ir(&q, &params([("gr", gv("1"))]))
        .expect("param graph");
    assert_eq!(rows(&r), expect(&[&["a"]]));
    // the internal eid variable is not a result column
    assert_eq!(
        cols(&run(&now, &sel_query(GraphSel::Set(vec![g("1")]), &["s"]))),
        ["s"]
    );
    let star = b().query(Op::Triple(
        b().t("?s", "v:p", "?o")
            .in_graph(GraphSel::Set(vec![g("1")])),
    ));
    assert_eq!(cols(&run(&now, &star)), ["s", "o"]);
}

fn snap(t: &TestDb, name: &str, q: &IrQuery) {
    let ex = explain(&t.db.now(), q);
    let sql = ex.sql.unwrap_or_else(|| "-- short-circuit".to_string());
    let text = format!("{}\n-- params: {}", sql, ex.params.len());
    insta::with_settings!({ snapshot_suffix => name, prepend_module_to_snapshot => false }, {
        insta::assert_snapshot!("graph_selector", text);
    });
}

// @lat: [[tests#Named Graphs#Graph Selector SQL Snapshots]]
#[test]
fn selector_sql_snapshots() {
    let t = fixture();
    snap(
        &t,
        "set_one",
        &sel_query(GraphSel::Set(vec![g("1")]), &["s"]),
    );
    snap(
        &t,
        "set_several",
        &sel_query(GraphSel::Set(vec![g("1"), g("2")]), &["s"]),
    );
    snap(
        &t,
        "var",
        &sel_query(GraphSel::Var("g".into()), &["s", "g"]),
    );
    // the membership is read in the pattern's own view
    let asof = b().at(View::as_of_tx(1));
    let q = b().query(
        Op::Triple(
            asof.t("?s", "v:p", "?o")
                .in_graph(GraphSel::Set(vec![g("1")])),
        )
        .project(&["s"]),
    );
    snap(&t, "set_one_asof", &q);
}

/// The first `t<N>` scan of the plan.
fn first_scan(plan: &[String]) -> String {
    plan.iter()
        .find_map(|l| {
            let mut w = l.split_whitespace();
            match (w.next(), w.next()) {
                (Some("SCAN" | "SEARCH"), Some(a)) if a.starts_with('t') => Some(a.to_string()),
                _ => None,
            }
        })
        .unwrap_or_else(|| panic!("no triple scan in {plan:?}"))
}

// @lat: [[tests#Named Graphs#Small Graph Seeks First]]
#[test]
fn small_graph_seeks_first() {
    let t = skewed();
    // a 1 800-member graph and a 3-member graph over the same statements. The
    // statistics appear on their own: the skewed fixture is loaded in one commit
    // that crosses `optimize_every`, and this second commit does too.
    let persons =
        t.db.now()
            .triples(None, Some(t.id(&v("type"))), Some(t.id(&v("Person"))))
            .unwrap();
    assert_eq!(persons.len(), PERSONS);
    let mut chunk = persons.iter().map(|x| x.eid);
    t.tx(|tx| {
        for (i, e) in chunk.by_ref().enumerate() {
            tx.add_to_graph(e, gv("big"), AssertOpts::default())?;
            if i < 3 {
                tx.add_to_graph(e, gv("small"), AssertOpts::default())?;
            }
        }
        Ok(())
    });
    t.db.optimize().unwrap();
    for (graph, want) in [("small", 3usize), ("big", PERSONS)] {
        let q = b().query(
            Op::Triple(
                b().t("?x", "v:type", "v:Person")
                    .in_graph(GraphSel::Set(vec![g(graph)])),
            )
            .project(&["x"]),
        );
        let ex = explain(&t.db.now(), &q);
        let plan = &ex.sql_region().expect("sql region").query_plan;
        if graph == "small" {
            // the membership (t1 after desugaring) drives the join
            assert_eq!(
                first_scan(plan),
                "t1",
                "{}\n{}",
                ex.sql.as_deref().unwrap(),
                plan.join("\n")
            );
        }
        assert_eq!(run(&t.db.now(), &q).rows.len(), want);
    }
}
