//! Spec `cyclic-join-execution` (OpenSpec change `add-lftj-operator`): the native
//! cyclic-join operator returns exactly the rows and multiplicities of the SQL
//! route, is routed conservatively with an explain reason, and honours budgets.
//!
//! Every differential test reads one database file through two handles: the
//! writer's default handle (LFTJ off, SQL route) and a second handle opened with
//! LFTJ on and no estimate threshold (native route).

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::*;
use proptest::prelude::*;
use tiramemsu::ir::builder::IrBuilder;
use tiramemsu::ir::{GraphSel, GraphSet, Op, Semantics, TermOrVar, TriplePattern, View};
use tiramemsu::*;

fn b() -> IrBuilder {
    IrBuilder::sparql()
}

fn bag() -> Semantics {
    Semantics::sparql().with_graph_set(GraphSet::BagOfEids)
}

fn lftj(min_rows_estimate: u64) -> OpenOptions {
    OpenOptions {
        planner: PlannerOptions {
            lftj: LftjConfig {
                enabled: true,
                min_rows_estimate,
            },
        },
        ..OpenOptions::default()
    }
}

/// A test database plus a second handle on the same file with LFTJ on.
struct Pair {
    t: TestDb,
    native: Db,
}

fn pair() -> Pair {
    let t = TestDb::new();
    let native = Db::open(&t.path, lftj(0)).expect("open native handle");
    Pair { t, native }
}

fn kinds(ex: &Explain) -> Vec<(RegionKind, RouteNote)> {
    ex.regions.iter().map(|r| (r.kind, r.note)).collect()
}

fn routed_native(ex: &Explain) -> bool {
    ex.regions
        .iter()
        .any(|r| r.kind == RegionKind::NativeLftj && r.note == RouteNote::LftjNative)
}

impl Pair {
    /// Runs `q` on both routes, checks the routing of each and that both return
    /// the same rows with the same multiplicities; returns the row count.
    fn same(&self, q: &IrQuery) -> usize {
        let ex = explain(&self.native.now(), q);
        // a constant missing from the dictionary empties the query before routing
        if !ex.short_circuit {
            assert!(routed_native(&ex), "not native: {:?}", kinds(&ex));
            assert!(ex.sql.as_deref().unwrap_or("").contains("tm_lftj("));
        }
        assert!(!routed_native(&explain(&self.t.db.now(), q)));
        let sql = sorted(&run(&self.t.db.now(), q));
        let native = sorted(&run(&self.native.now(), q));
        assert_eq!(native, sql, "native and SQL differ for\n{q}");
        sql.len()
    }
}

fn n(i: usize) -> Value {
    v(&format!("n{i}"))
}

/// A cyclic graph over `nodes` nodes on `v:knows` with parallel statements, self
/// loops and two-cycles, so set, bag and isomorphism semantics all differ.
fn knows_graph(t: &TestDb, nodes: usize) {
    t.tx(|tx| {
        for i in 0..nodes {
            for k in [1usize, 2, 5] {
                tx.create(n(i), v("knows"), n((i * 3 + k) % nodes), Valid::ALWAYS)?;
            }
        }
        // parallel statements on a few edges
        for i in (0..nodes).step_by(2) {
            for k in [1usize, 2, 5] {
                tx.create(n(i), v("knows"), n((i * 3 + k) % nodes), Valid::ALWAYS)?;
            }
        }
        tx.create(n(0), v("knows"), n(0), Valid::ALWAYS)?;
        tx.create(n(1), v("knows"), n(2), Valid::ALWAYS)?;
        tx.create(n(2), v("knows"), n(1), Valid::ALWAYS)?;
        Ok(())
    });
}

fn tri(bb: &IrBuilder, p: &str) -> Vec<TriplePattern> {
    vec![
        bb.t("?a", p, "?b"),
        bb.t("?b", p, "?c"),
        bb.t("?c", p, "?a"),
    ]
}

fn join(ps: Vec<TriplePattern>) -> Op {
    Op::join(ps.into_iter().map(Op::Triple).collect())
}

// cyclic-join-execution "Parallel statements": set and bag modes each match their
// SQL baseline
// @lat: [[tests#Cyclic Joins#Parallel Statements Match SQL]]
#[test]
fn parallel_statements_set_and_bag_match_sql() {
    let p = pair();
    knows_graph(&p.t, 12);
    let set = p.same(&b().query(join(tri(&b(), "v:knows"))));
    let bag_rows = p.same(&IrQuery::new(join(tri(&b(), "v:knows")), bag()));
    assert!(set > 0);
    assert!(bag_rows > set, "parallel statements multiply bag rows");
    // a bound eid gives one row per statement under set semantics too
    let mut ps = tri(&b(), "v:knows");
    ps[0] = ps[0].clone().with_eid("?r");
    let with_eid = p.same(&b().query(join(ps)));
    assert!(with_eid > set && with_eid <= bag_rows);
    // composed with SQL above it: a filter, a projection with DISTINCT, a count
    let filtered = b().query(b().filter(
        join(tri(&b(), "v:knows")),
        tiramemsu::ir::Expr::not(tiramemsu::ir::Expr::SameTerm(
            Box::new(tiramemsu::ir::Expr::var("a")),
            Box::new(tiramemsu::ir::Expr::var("b")),
        )),
    ));
    p.same(&filtered);
    p.same(&b().query(join(tri(&b(), "v:knows")).project_distinct(&["a"])));
    let count = b().query(
        join(tri(&b(), "v:knows")).aggregate(&[], vec![tiramemsu::ir::Agg::count_star("n")]),
    );
    p.same(&count);
}

// Cypher semantics: bag of eids plus relationship isomorphism inside the region
// @lat: [[tests#Cyclic Joins#Cypher Bag Of Eids Matches SQL]]
#[test]
fn cypher_bag_and_isomorphism_match_sql() {
    let p = pair();
    knows_graph(&p.t, 9);
    let c = IrBuilder::cypher();
    let iso: Vec<TriplePattern> = tri(&c, "v:knows")
        .into_iter()
        .map(|t| t.in_group(0))
        .collect();
    let with_iso = p.same(&c.query(join(iso.clone())));
    let homo = p.same(&c.query(join(tri(&c, "v:knows"))));
    assert!(
        with_iso < homo,
        "isomorphism removes repeated relationships"
    );
    // relationship variables bound, plus a fourth pattern of the same group outside
    // the cycle's variables (still one pure cyclic join)
    let mut named = iso.clone();
    for (i, t) in named.iter_mut().enumerate() {
        t.eid = Some(tiramemsu::ir::Var::new(format!("r{i}")));
    }
    named.push(c.t("?a", "v:knows", "?d").in_group(0));
    p.same(&c.query(join(named)));
    // Cypher text: the triangle query returns the same bag on both handles
    let q = "MATCH (a)-[:knows]->(b)-[:knows]->(c)-[:knows]->(a) RETURN a, b, c";
    let none = CypherParams::default();
    let mut sql: Vec<String> =
        p.t.db
            .now()
            .cypher(q, &none)
            .unwrap()
            .rows
            .iter()
            .map(|r| format!("{r:?}"))
            .collect();
    let mut native: Vec<String> = p
        .native
        .now()
        .cypher(q, &none)
        .unwrap()
        .rows
        .iter()
        .map(|r| format!("{r:?}"))
        .collect();
    sql.sort();
    native.sort();
    assert_eq!(native, sql);
}

// cyclic-join-execution "Mixed temporal views": each pattern applies its own view
// predicates
// @lat: [[tests#Cyclic Joins#Mixed Temporal Views Match SQL]]
#[test]
fn mixed_temporal_views_match_sql() {
    let p = pair();
    let mut first = Vec::new();
    p.t.tx(|tx| {
        for i in 0..8 {
            first.push(tx.create(n(i), v("knows"), n((i + 1) % 8), Valid::ALWAYS)?);
            tx.create(
                n(i),
                v("knows"),
                n((i + 3) % 8),
                Valid::between(1_000, 5_000),
            )?;
        }
        Ok(())
    });
    let t1 = p.t.last_t();
    p.t.tx(|tx| {
        for e in first.iter().step_by(2) {
            tx.retract(*e)?;
        }
        for i in 0..8 {
            tx.create(n(i), v("knows"), n((i + 6) % 8), Valid::from(3_000))?;
            tx.create(n((i + 2) % 8), v("knows"), n(i), Valid::ALWAYS)?;
        }
        Ok(())
    });
    let views = [
        View::NOW,
        View::as_of_tx(t1),
        View::history(),
        View::NOW.valid_at(2_000),
        View::as_of_tx(t1).valid_at(4_000),
        View::history().valid_at(6_000),
    ];
    let mut nonempty = 0;
    for (i, a) in views.iter().enumerate() {
        for bv in &views[i..] {
            for c in [View::NOW, View::history()] {
                let mut ps = tri(&b(), "v:knows");
                ps[0].view = *a;
                ps[1].view = *bv;
                ps[2].view = c;
                if p.same(&b().query(join(ps.clone()))) > 0 {
                    nonempty += 1;
                }
                p.same(&IrQuery::new(join(ps), bag()));
            }
        }
    }
    assert!(nonempty > 3, "the fixture exercises non-empty mixes");
}

// each access path carries its graph-membership constraint in its own view
// @lat: [[tests#Cyclic Joins#Graph Membership Matches SQL]]
#[test]
fn graph_membership_matches_sql() {
    let p = pair();
    let g = |s: &str| Value::iri(format!("urn:tiramemsu:g:{s}"));
    p.t.tx(|tx| {
        for i in 0..9 {
            for k in [1usize, 2, 4] {
                let e = tx.create(n(i), v("knows"), n((i * 2 + k) % 9), Valid::ALWAYS)?;
                tx.add_to_graph(
                    e,
                    g(if (i + k) % 2 == 0 { "even" } else { "odd" }),
                    AssertOpts::default(),
                )?;
                if k == 1 {
                    tx.add_to_graph(e, g("odd"), AssertOpts::default())?;
                }
            }
        }
        Ok(())
    });
    let sel = |gs: GraphSel| -> Vec<TriplePattern> {
        tri(&b(), "v:knows")
            .into_iter()
            .map(|t| t.in_graph(gs.clone()))
            .collect()
    };
    let one = GraphSel::Set(vec![TermOrVar::iri("urn:tiramemsu:g:odd")]);
    p.same(&b().query(join(sel(one))));
    p.same(&b().query(join(sel(GraphSel::Var("g".into())))));
    p.same(&IrQuery::new(join(sel(GraphSel::Var("g".into()))), bag()));
}

// SPARQL text: explain names the native route, provenance bindings agree
// @lat: [[tests#Cyclic Joins#Provenance Matches SQL]]
#[test]
fn sparql_text_and_provenance_match_sql() {
    let p = pair();
    knows_graph(&p.t, 10);
    let q = "SELECT ?a ?b ?c WHERE { ?a v:knows ?b . ?b v:knows ?c . ?c v:knows ?a }";
    let ex = p.native.now().explain_sparql(q).unwrap();
    assert!(routed_native(&ex), "{:?}", kinds(&ex));
    let native_plan = ex
        .regions
        .iter()
        .find(|r| r.kind == RegionKind::NativeLftj)
        .unwrap();
    assert_eq!(native_plan.aliases.len(), 1);
    assert!(
        !native_plan.query_plan.is_empty(),
        "EXPLAIN QUERY PLAN rows for the call"
    );
    let base = p.t.db.now().explain_sparql(q).unwrap();
    assert!(base
        .regions
        .iter()
        .any(|r| r.kind == RegionKind::Sql && r.note == RouteNote::CyclicLftjDisabled));
    let opts = SparqlOptions {
        provenance: true,
        ..SparqlOptions::default()
    };
    let sql = p.t.db.now().sparql_with(q, &opts).unwrap();
    let native = p.native.now().sparql_with(q, &opts).unwrap();
    let rows = |r: &SparqlResult| {
        let s = r.solutions().unwrap();
        let mut out: Vec<String> = (0..s.rows.len())
            .map(|i| format!("{:?} {:?}", s.rows[i], s.provenance(i)))
            .collect();
        out.sort();
        out
    };
    assert!(!rows(&sql).is_empty());
    assert_eq!(rows(&native), rows(&sql));
}

// cyclic-join-execution "No installed operator": enabled, but no operator is
// registered, so the region remains SQL and explain identifies the fallback
// @lat: [[tests#Cyclic Joins#No Installed Operator Falls Back]]
#[test]
fn enabled_without_operator_stays_sql() {
    let t = TestDb::new();
    knows_graph(&t, 6);
    let mut opts = PlannerOptions::default();
    opts.lftj.enabled = true;
    opts.lftj.min_rows_estimate = 0;
    let engine = tm_exec::QueryEngine::new(opts, tm_exec::OperatorRegistry::new(), 64);
    let conn = rusqlite::Connection::open(&t.path).unwrap();
    tm_rusqlite::register::register_defaults(&conn).unwrap();
    let caps = Capabilities {
        functions: true,
        vtab: true,
        ..Capabilities::default()
    };
    let mut exec = tm_rusqlite::RusqliteExec::from_connection(conn, caps);
    engine.install(&mut exec).unwrap();
    let q = b().query(join(tri(&b(), "v:knows")));
    let prepared = engine.prepare(&q, &Params::new()).unwrap();
    let ex = engine.explain(&mut exec, &prepared).unwrap();
    assert!(kinds(&ex).contains(&(RegionKind::Sql, RouteNote::LftjUnavailable)));
    assert!(!routed_native(&ex));
    assert!(!ex.sql.unwrap().contains("tm_lftj"));
    let r = engine
        .execute(&mut exec, tm_exec::CacheMode::Scoped, &prepared)
        .unwrap();
    assert_eq!(sorted(&r), sorted(&run(&t.db.now(), &q)));
}

// cyclic-join-execution "Unsupported region": optional algebra and non-triple
// inputs keep SQL with the reason
// @lat: [[tests#Cyclic Joins#Unsupported Region Stays SQL]]
#[test]
fn unsupported_regions_stay_sql() {
    let p = pair();
    knows_graph(&p.t, 8);
    let both = |q: &IrQuery| {
        let ex = explain(&p.native.now(), q);
        assert!(!routed_native(&ex), "{:?}", kinds(&ex));
        assert_eq!(
            sorted(&run(&p.native.now(), q)),
            sorted(&run(&p.t.db.now(), q))
        );
        ex
    };
    // a cycle closed only through OPTIONAL
    let [ab, bc, ca]: [TriplePattern; 3] = tri(&b(), "v:knows").try_into().unwrap();
    let opt = b().query(Op::left_join(
        Op::Triple(ab.clone()),
        join(vec![bc.clone(), ca.clone()]),
        None,
    ));
    let ex = both(&opt);
    assert!(
        kinds(&ex).contains(&(RegionKind::Sql, RouteNote::LftjUnsupportedShape)),
        "{:?}",
        kinds(&ex)
    );
    // SPARQL text with the same shape
    let text = "SELECT * WHERE { ?a v:knows ?b OPTIONAL { ?b v:knows ?c . ?c v:knows ?a } }";
    let ex = p.native.now().explain_sparql(text).unwrap();
    assert!(!routed_native(&ex));
    assert!(ex
        .regions
        .iter()
        .any(|r| r.note == RouteNote::LftjUnsupportedShape));
    // a cycle through a virtual predicate (a non-triple input)
    let virt = b().query(Op::join(vec![
        Op::Triple(ab.clone().with_eid("?e")),
        Op::Triple(bc.clone()),
        Op::Triple(ca.clone()),
        b().triple("?e", tiramemsu::ir::vocab::SYS_SUBJECT, "?a"),
    ]));
    let ex = both(&virt);
    assert!(kinds(&ex).contains(&(RegionKind::Sql, RouteNote::LftjUnsupportedShape)));
    // an acyclic BGP carries no LFTJ note at all
    let chain = b().query(join(vec![ab, bc]));
    assert!(both(&chain)
        .regions
        .iter()
        .all(|r| r.note == RouteNote::None));
}

// a pure cycle inside OPTIONAL is its own region: native, composed by SQL
// @lat: [[tests#Cyclic Joins#Optional Side Composes]]
#[test]
fn native_region_composes_inside_optional() {
    let p = pair();
    knows_graph(&p.t, 10);
    p.t.tx(|tx| {
        for i in 0..14 {
            tx.assert(n(i), v("likes"), v("tea"), Valid::ALWAYS)?;
        }
        Ok(())
    });
    let q = b().query(Op::left_join(
        b().triple("?a", "v:likes", "v:tea"),
        join(tri(&b(), "v:knows")),
        None,
    ));
    let rows = p.same(&q);
    assert!(rows > 14);
    let ex = explain(&p.native.now(), &q);
    // the call sits in an unflattened derived table, so it is materialised once
    assert!(ex.sql.unwrap().contains("LIMIT -1"));
}

// routing needs the documented estimate policy to agree
// @lat: [[tests#Cyclic Joins#Estimate Policy Gates Routing]]
#[test]
fn estimate_policy_gates_routing() {
    let t = TestDb::new();
    t.tx(|tx| {
        tx.assert(v("x"), v("knows"), v("y"), Valid::ALWAYS)
            .map(|_| ())
    });
    let before = t.last_t();
    knows_graph(&t, 8);
    let knows = t.id(&v("knows")).raw();
    let stored =
        t.db.read_sql(&format!("SELECT count(*) FROM triple WHERE p = {knows}"))
            .unwrap()[0][0]
            .as_i64()
            .unwrap() as u64;
    let q = b().query(join(tri(&b(), "v:knows")));
    let high = Db::open(&t.path, lftj(stored + 1)).unwrap();
    let ex = explain(&high.now(), &q);
    assert!(kinds(&ex).contains(&(RegionKind::Sql, RouteNote::LftjBelowEstimate)));
    let at = Db::open(&t.path, lftj(stored)).unwrap();
    assert!(routed_native(&explain(&at.now(), &q)));
    // a pattern's own view counts: as of before the data, nothing reaches the bar
    let mut ps = tri(&b(), "v:knows");
    for p in &mut ps {
        p.view = View::as_of_tx(before);
    }
    let ex = explain(&at.now(), &b().query(join(ps)));
    assert!(kinds(&ex).contains(&(RegionKind::Sql, RouteNote::LftjBelowEstimate)));
    assert_eq!(sorted(&run(&high.now(), &q)), sorted(&run(&at.now(), &q)));
}

// opt-in: the default options never route natively, nor install the operator
// @lat: [[tests#Cyclic Joins#Disabled By Default]]
#[test]
fn disabled_by_default() {
    assert!(!OpenOptions::default().planner.lftj.enabled);
    let t = TestDb::new();
    knows_graph(&t, 6);
    let q = b().query(join(tri(&b(), "v:knows")));
    let ex = explain(&t.db.now(), &q);
    assert!(kinds(&ex).contains(&(RegionKind::Sql, RouteNote::CyclicLftjDisabled)));
    assert!(!ex.sql.unwrap().contains("tm_lftj"));
    let e =
        t.db.read_sql("SELECT * FROM tm_lftj('lftj1;1;0|?0 #1 #2 ?0 now - b -')");
    assert!(e.is_err(), "tm_lftj is not installed by default");
}

// cyclic-join-execution "Cancellation": resources are released and no partial
// successful result is returned
// @lat: [[tests#Cyclic Joins#Cancellation Returns No Rows]]
#[test]
fn cancelled_native_join_returns_nothing() {
    let p = pair();
    knows_graph(&p.t, 30);
    let q = b().query(join(tri(&b(), "v:knows")));
    let token = CancelToken::new();
    let hook_token = token.clone();
    // cancel after planning, as the statement (and so the operator) starts
    p.native
        .set_query_hook(Some(Arc::new(move || hook_token.cancel())));
    let budget = QueryBudget {
        cancel: Some(token),
        ..QueryBudget::default()
    };
    let r = p
        .native
        .now()
        .with_budget(&budget)
        .execute_ir(&q, &Params::new());
    assert!(matches!(r, Err(Error::Cancelled)), "{r:?}");
    p.native.set_query_hook(None);
    // a deadline that has passed fails the same way, typed
    let late = QueryBudget {
        timeout: Some(Duration::ZERO),
        ..QueryBudget::default()
    };
    let r = p
        .native
        .now()
        .with_budget(&late)
        .execute_ir(&q, &Params::new());
    assert!(matches!(r, Err(Error::DeadlineExceeded { .. })), "{r:?}");
    // the readers were returned: every pooled connection still answers in full
    let full = sorted(&run(&p.t.db.now(), &q));
    for _ in 0..p.native.reader_count() + 1 {
        assert_eq!(sorted(&run(&p.native.now(), &q)), full);
    }
}

// row and byte budgets apply to native rows, with no partial result
// @lat: [[tests#Cyclic Joins#Result Limits Apply]]
#[test]
fn result_limits_apply_to_native_rows() {
    let p = pair();
    knows_graph(&p.t, 12);
    let q = b().query(join(tri(&b(), "v:knows")));
    let total = p.same(&q) as u64;
    assert!(total > 2);
    let rows = QueryBudget {
        max_rows: Some(2),
        ..QueryBudget::default()
    };
    let r = p
        .native
        .now()
        .with_budget(&rows)
        .execute_ir(&q, &Params::new());
    assert!(
        matches!(
            r,
            Err(Error::ResultLimitExceeded {
                limit: ResultLimit::Rows(2)
            })
        ),
        "{r:?}"
    );
    let bytes = QueryBudget {
        max_bytes: Some(64),
        ..QueryBudget::default()
    };
    let r = p
        .native
        .now()
        .with_budget(&bytes)
        .execute_ir(&q, &Params::new());
    assert!(
        matches!(
            r,
            Err(Error::ResultLimitExceeded {
                limit: ResultLimit::Bytes(64)
            })
        ),
        "{r:?}"
    );
    let enough = QueryBudget {
        max_rows: Some(total),
        ..QueryBudget::default()
    };
    let r = p
        .native
        .now()
        .with_budget(&enough)
        .execute_ir(&q, &Params::new())
        .unwrap();
    assert_eq!(r.len() as u64, total);
}

#[derive(Clone, Debug)]
enum Step {
    Create(u8, u8, u8),
    Retract(u8),
}

fn steps() -> impl Strategy<Value = Vec<Vec<Step>>> {
    let step = prop_oneof![
        4 => (0u8..7, 0u8..2, 0u8..7).prop_map(|(a, p, c)| Step::Create(a, p, c)),
        1 => (0u8..40).prop_map(Step::Retract),
    ];
    prop::collection::vec(prop::collection::vec(step, 1..14), 1..4)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 24, ..ProptestConfig::default() })]

    // random histories with parallel statements, retractions and two predicates:
    // every view mix, in set, bag and isomorphism modes, matches SQL
    // @lat: [[tests#Cyclic Joins#Random Histories Match SQL]]
    #[test]
    fn random_histories_match_sql(txs in steps(), mix in prop::collection::vec(0usize..4, 3)) {
        let p = pair();
        let mut eids = Vec::new();
        let mut ts = Vec::new();
        for steps in &txs {
            p.t.tx(|tx| {
                for s in steps {
                    match s {
                        Step::Create(a, pr, c) => eids.push(tx.create(
                            n(*a as usize),
                            v(if *pr == 0 { "k" } else { "m" }),
                            n(*c as usize),
                            Valid::ALWAYS,
                        )?),
                        Step::Retract(i) => {
                            if let Some(e) = eids.get(*i as usize) {
                                tx.retract(*e)?;
                            }
                        }
                    }
                }
                Ok(())
            });
            ts.push(p.t.last_t());
        }
        let view = |k: usize| match k {
            0 => View::NOW,
            1 => View::history(),
            2 => View::as_of_tx(ts[0]),
            _ => View::as_of_tx(ts[ts.len() / 2]),
        };
        let preds = ["v:k", "v:k", "v:m"];
        let mut ps: Vec<TriplePattern> = vec![
            b().t("?a", preds[0], "?b"),
            b().t("?b", preds[1], "?c"),
            b().t("?c", preds[2], "?a"),
        ];
        for (t, k) in ps.iter_mut().zip(&mix) {
            t.view = view(*k);
        }
        p.same(&b().query(join(ps.clone())));
        p.same(&IrQuery::new(join(ps.clone()), bag()));
        let c = IrBuilder::cypher();
        let iso: Vec<TriplePattern> = ps.into_iter().map(|t| t.in_group(0)).collect();
        p.same(&c.query(join(iso)));
    }
}
