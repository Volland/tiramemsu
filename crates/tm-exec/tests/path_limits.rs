//! Hop limits, termination on cycles, the search-state guard and result order
//! (`path-evaluation`: hop limits, termination, memory guard, ordering).

mod common;

use common::paths::*;
use common::*;
use tiramemsu::*;

fn next_chain(g: &G) {
    let names: Vec<String> = (0..=20).map(|i| format!("n{i}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    chain(g, "next", &refs);
}

// "max_hops bounds reachability" / "Explicit bound above the cap" / "SPARQL has no cap" (engine)
#[test]
fn max_hops_is_a_hard_bound() {
    let g = G::new();
    next_chain(&g);
    let v = g.t.db.now();
    assert_eq!(
        g.ends(v, "n0", "next+", REACH, 3),
        eh(&[("n1", 1), ("n2", 2), ("n3", 3)])
    );
    assert_eq!(g.ends(v, "n0", "next+", TRAIL, 18).len(), 18);
    assert_eq!(g.ends(v, "n0", "next+", REACH, u32::MAX).len(), 20);
    assert_eq!(g.ends(v, "n0", "next+", ANY, 4).len(), 4);
}

// "Reachability on a cycle" / "Self-loop" / "Trail on a cycle" / "Shortest on a cycle"
#[test]
fn cycles_terminate() {
    let g = G::new();
    g.edges(&[("a", "p", "b"), ("b", "p", "c"), ("c", "p", "a")]);
    assert_eq!(g.now("a", "p*", REACH), eh(&[("a", 0), ("b", 1), ("c", 2)]));
    let v = g.t.db.now();
    let trails = g.ends(v, "a", "p+", TRAIL, 15);
    assert_eq!(trails, eh(&[("b", 1), ("c", 2), ("a", 3)]));
    let all = g.ends(v, "a", "p+", ALL, u32::MAX);
    assert_eq!(all, eh(&[("b", 1), ("c", 2), ("a", 3)]));
    let any = g.ends(v, "a", "p+", ANY, u32::MAX);
    assert_eq!(any, all);
    let s = G::new();
    s.edge("a", "p", "a");
    assert_eq!(s.now("a", "p+", REACH), eh(&[("a", 1)]));
    assert_eq!(s.now("a", "p+", ALL), eh(&[("a", 1)]));
}

// task 5.1: the layer driver's zero-length emission in every mode
#[test]
fn zero_length_row_in_every_mode() {
    let g = G::new();
    g.edge("a", "p", "b");
    for m in [REACH, TRAIL, ANY, ALL] {
        assert_eq!(g.now("a", "p*", m), eh(&[("a", 0), ("b", 1)]), "{m}");
        assert_eq!(g.now("a", "p+", m), eh(&[("b", 1)]), "{m}");
    }
}

// "Guard trips on explosive trails": complete graph of 20 nodes, limit 1000
#[test]
fn guard_trips_on_explosive_trails() {
    let g = G::with(OpenOptions {
        path_max_states: 1_000,
        ..OpenOptions::default()
    });
    g.t.tx(|tx| {
        for i in 0..20 {
            for j in 0..20 {
                if i != j {
                    tx.assert(
                        v(&format!("k{i}")),
                        v("p"),
                        v(&format!("k{j}")),
                        Valid::ALWAYS,
                    )?;
                }
            }
        }
        Ok(())
    });
    let view = g.t.db.now();
    match view.path(g.id("k0"), "p+", TRAIL, 10) {
        Err(Error::PathLimitExceeded { limit }) => assert_eq!(limit, 1_000),
        other => panic!("{other:?}"),
    }
    // through SQL the typed error still surfaces
    let sql = format!(
        "SELECT count(*) FROM tm_path({}, 'p+', 'TRAIL', 10)",
        g.id("k0").raw()
    );
    match g.t.db.read_sql(&sql) {
        Err(Error::PathLimitExceeded { limit }) => assert_eq!(limit, 1_000),
        other => panic!("{other:?}"),
    }
    // the guard leaves the database unchanged and later small queries work
    assert_eq!(view.path(g.id("k0"), "p", REACH, 1).unwrap().len(), 19);
}

// "Reachability order"
#[test]
fn reach_order() {
    let g = G::new();
    g.edges(&[("a", "p", "c"), ("a", "p", "b"), ("b", "p", "d")]);
    let view = g.t.db.now();
    let rows = view.path(g.id("a"), "p+", REACH, 9).unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].hops, 1);
    assert_eq!(rows[1].hops, 1);
    assert!(rows[0].end.raw() < rows[1].end.raw());
    assert_eq!((g.name(rows[2].end), rows[2].hops), ("d".to_string(), 2));
}

// "Trail order" / "Repeatable order"
#[test]
fn trail_order_and_repeatability() {
    let g = G::new();
    let e = g.edges(&[("a", "p", "c"), ("a", "p", "b")]);
    let view = g.t.db.now();
    let rows = view.path(g.id("a"), "p", TRAIL, 3).unwrap();
    let eids: Vec<i64> = rows
        .iter()
        .map(|r| r.path.as_ref().unwrap().hops[0].eid.raw())
        .collect();
    assert_eq!(
        eids,
        vec![e[0].oid().raw(), e[1].oid().raw()],
        "smaller eid first"
    );
    assert_eq!(view.path(g.id("a"), "p", TRAIL, 3).unwrap(), rows);
    // non-decreasing hops in every mode
    let g = G::new();
    g.edges(&[
        ("a", "p", "b"),
        ("b", "p", "c"),
        ("a", "p", "c"),
        ("c", "p", "d"),
    ]);
    for m in [REACH, TRAIL, ANY, ALL] {
        let hops: Vec<u32> = g.now("a", "p+", m).iter().map(|x| x.1).collect();
        let mut s = hops.clone();
        s.sort();
        assert_eq!(hops, s, "{m}");
    }
}

// task 1.3: option defaults
#[test]
fn option_defaults() {
    let o = OpenOptions::default();
    assert_eq!((o.path_max_hops, o.path_max_states), (15, 1_000_000));
    let p = tm_exec::PathOptions::default();
    assert_eq!((p.max_hops, p.max_states), (15, 1_000_000));
    assert_eq!(G::new().t.db.path_max_hops(), 15);
}
