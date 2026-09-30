//! Time-aware evaluation: every hop reads through the view of the path
//! (`path-evaluation`: time-aware evaluation).

mod common;

use common::paths::*;
use common::*;
use tiramemsu::*;

fn at(g: &G, t: u64) -> View<'_> {
    g.t.db.as_of(TimeRef::Tx(t))
}

/// `(a knows b)` in tx 10 and `(b knows c)` in tx 20.
fn two_step() -> G {
    let g = G::new();
    g.t.advance_to(9);
    g.edge("a", "knows", "b");
    g.t.advance_to(19);
    g.edge("b", "knows", "c");
    g
}

// "Path valid now but not as of an earlier transaction"
#[test]
fn now_versus_as_of() {
    let g = two_step();
    assert_eq!(g.now("a", "knows+", REACH), eh(&[("b", 1), ("c", 2)]));
    assert_eq!(
        g.ends(at(&g, 15), "a", "knows+", REACH, u32::MAX),
        eh(&[("b", 1)])
    );
    // path-table-function "As of a transaction"
    let sql = format!(
        "SELECT count(*) FROM tm_path({}, 'knows+', 'REACH', NULL, 'asOf/15')",
        g.id("a").raw()
    );
    assert_eq!(g.t.db.read_sql(&sql).unwrap()[0][0].as_i64(), Some(1));
}

// "Path valid as of an earlier transaction but not now" / "History traverses retracted statements"
#[test]
fn retracted_paths() {
    let g = G::new();
    g.t.advance_to(9);
    let e = g.edges(&[("a", "knows", "b"), ("b", "knows", "c")]);
    g.t.advance_to(29);
    g.t.tx(|tx| tx.retract(e[1]).map(|_| ()));
    assert_eq!(
        g.ends(at(&g, 20), "a", "knows+", REACH, u32::MAX),
        eh(&[("b", 1), ("c", 2)])
    );
    assert_eq!(g.now("a", "knows+", REACH), eh(&[("b", 1)]));
    assert_eq!(
        g.ends(g.t.db.history(), "a", "knows+", REACH, u32::MAX),
        eh(&[("b", 1), ("c", 2)])
    );
}

// "Valid time filters hops" / api "API uses the view's valid time"
#[test]
fn valid_time_filters_hops() {
    let g2 = G::new();
    g2.t.tx(|tx| {
        tx.assert(
            v("a"),
            v("worksAt"),
            v("acme"),
            Valid::between(1_577_836_800_000, 1_640_995_200_000),
        )?;
        tx.assert(v("acme"), v("locatedIn"), v("berlin"), Valid::ALWAYS)?;
        Ok(())
    });
    let view = |ms| g2.t.db.now().valid_at(ms);
    let inside = g2.ends(view(1_622_505_600_000), "a", "worksAt/locatedIn", REACH, 9);
    assert_eq!(inside, eh(&[("berlin", 2)]));
    assert!(g2
        .ends(view(1_685_577_600_000), "a", "worksAt/locatedIn", REACH, 9)
        .is_empty());
    // the API path with a valid-time view and TRAIL mode
    let rows = view(1_622_505_600_000)
        .path(g2.id("a"), "worksAt/locatedIn", TRAIL, 5)
        .unwrap();
    assert_eq!(rows.len(), 1);
    // history plus valid time through SQL
    let sql = format!(
        "SELECT count(*) FROM tm_path({}, 'worksAt', 'REACH', NULL, 'history;validAt/2021-06-01')",
        g2.id("a").raw()
    );
    assert_eq!(g2.t.db.read_sql(&sql).unwrap()[0][0].as_i64(), Some(1));
}

// "Superseded edge under asOf"
#[test]
fn superseded_edge_under_as_of() {
    let g = G::new();
    g.t.advance_to(9);
    let e1 = g.edge("a", "worksAt", "acme");
    g.t.advance_to(39);
    let mut e10 = None;
    g.t.tx(|tx| {
        e10 = Some(tx.supersede(e1, Patch::object(v("globex")))?);
        Ok(())
    });
    let via = |view: View<'_>| -> (Eid, String) {
        let rows = view.path(g.id("a"), "worksAt", TRAIL, 3).unwrap();
        assert_eq!(rows.len(), 1);
        let p = rows[0].path.as_ref().unwrap();
        (Eid::from_oid(p.hops[0].eid).unwrap(), g.name(rows[0].end))
    };
    assert_eq!(via(at(&g, 39)), (e1, "acme".to_string()));
    assert_eq!(via(g.t.db.now()), (e10.unwrap(), "globex".to_string()));
}

// "Historical path results are stable" (extends tests#Time Travel#Historical Reads Are Stable)
#[test]
fn historical_path_results_are_stable() {
    let g = G::new();
    let e = g.edges(&[("a", "p", "b"), ("b", "p", "c"), ("c", "p", "d")]);
    let t = g.t.last_t();
    let snapshot = |g: &G| {
        let v = at(g, t);
        [REACH, TRAIL, ANY, ALL].map(|m| v.path(g.id("a"), "p+", m, 9).unwrap())
    };
    let before = snapshot(&g);
    g.t.tx(|tx| tx.retract(e[1]).map(|_| ()));
    g.t.tx(|tx| tx.supersede(e[0], Patch::object(v("z"))).map(|_| ()));
    g.t.tx(|tx| tx.assert(v("d"), v("p"), v("e"), Valid::ALWAYS).map(|_| ()));
    assert_eq!(snapshot(&g), before);
}

// path-table-function "Callable inside speculation" / api "API inside speculation"
#[test]
fn paths_see_speculative_statements() {
    let g = G::new();
    g.edge("alice", "knows", "bob");
    let alice = g.id("alice");
    let seen =
        g.t.db
            .with(
                |tx| {
                    tx.assert(v("bob"), v("knows"), v("carol"), Valid::ALWAYS)
                        .map(|_| ())
                },
                |view| {
                    let carol = view.encode(&v("carol"))?.expect("speculative term");
                    let api = view.path(alice, "knows+", REACH, 9)?;
                    // the planner routes an IR path region through `tm_path` on the writer
                    let b = tiramemsu::ir::builder::IrBuilder::sparql();
                    let q = b.query(b.path(
                        tiramemsu::ir::TermOrVar::Id(alice),
                        tiramemsu::ir::PathExpr::iri(vi("knows")).plus(),
                        "?x",
                        REACH,
                    ));
                    let via_sql = view.execute_ir(&q, &Params::new())?;
                    Ok((api.iter().any(|r| r.end == carol), via_sql.len()))
                },
            )
            .unwrap();
    assert_eq!(seen, (true, 2));
    // after the speculation the statement is gone
    assert_eq!(g.now("alice", "knows+", REACH), eh(&[("bob", 1)]));
}
