//! Path expression operators, the relationship-view wildcard, and the search modes
//! (`path-evaluation`: operators, modes, reachability, trails, shortest paths).

mod common;

use common::paths::*;
use common::*;
use tiramemsu::*;

fn knows_chain() -> G {
    let g = G::new();
    chain(&g, "knows", &["a", "b", "c", "d"]);
    g
}

// path-evaluation "Single forward predicate"
#[test]
fn single_forward_predicate() {
    let g = G::new();
    g.edges(&[("a", "knows", "b"), ("b", "knows", "c")]);
    assert_eq!(g.now("a", "knows", REACH), eh(&[("b", 1)]));
}

// "Sequence"
#[test]
fn sequence() {
    let g = G::new();
    g.edges(&[
        ("a", "knows", "b"),
        ("b", "worksAt", "acme"),
        ("a", "worksAt", "globex"),
    ]);
    assert_eq!(g.now("a", "knows/worksAt", REACH), eh(&[("acme", 2)]));
}

// "Alternation of predicates"
#[test]
fn alternation() {
    let g = G::new();
    g.edges(&[
        ("a", "knows", "b"),
        ("a", "likes", "c"),
        ("a", "hates", "d"),
    ]);
    let r = g.now("a", "knows|likes", REACH);
    assert_eq!(sorted_vec(&r), eh(&[("b", 1), ("c", 1)]));
}

// "Inverse step"
#[test]
fn inverse_step() {
    let g = G::new();
    g.edge("b", "knows", "a");
    assert_eq!(g.now("a", "^knows", REACH), eh(&[("b", 1)]));
}

// "One or more" / "Zero or more includes the start" / "Zero or one"
#[test]
fn closures_on_a_chain() {
    let g = knows_chain();
    assert_eq!(
        g.now("a", "knows+", REACH),
        eh(&[("b", 1), ("c", 2), ("d", 3)])
    );
    assert_eq!(
        g.now("a", "knows*", REACH),
        eh(&[("a", 0), ("b", 1), ("c", 2), ("d", 3)])
    );
    assert_eq!(g.now("a", "knows?", REACH), eh(&[("a", 0), ("b", 1)]));
}

// "Bounded repetition" / "Bounded repetition with open maximum"
#[test]
fn bounded_repetition() {
    let g = knows_chain();
    assert_eq!(g.now("a", "knows{2,3}", REACH), eh(&[("c", 2), ("d", 3)]));
    let r = g.ends(g.t.db.now(), "a", "knows{2,}", TRAIL, 15);
    assert_eq!(r, eh(&[("c", 2), ("d", 3)]));
}

// "Precedence of inverse and sequence"
#[test]
fn precedence_of_inverse_and_sequence() {
    let g = G::new();
    g.edges(&[("b", "knows", "a"), ("b", "worksAt", "acme")]);
    assert_eq!(g.now("a", "^knows/worksAt", REACH), eh(&[("acme", 2)]));
}

// "Grouped repetition of a sequence"
#[test]
fn grouped_repetition() {
    let g = G::new();
    g.edges(&[
        ("a", "p", "b"),
        ("b", "q", "c"),
        ("c", "p", "d"),
        ("d", "q", "e"),
    ]);
    assert_eq!(g.now("a", "(p/q)+", REACH), eh(&[("c", 2), ("e", 4)]));
}

// "Unknown predicate matches nothing"
#[test]
fn unknown_predicate_matches_nothing() {
    let g = G::new();
    g.edge("a", "p", "b");
    assert_eq!(g.now("a", "neverUsed*", REACH), eh(&[("a", 0)]));
}

// "Wildcard skips properties and labels" / "Wildcard honours isEdge"
// path-table-function "Wildcard atom"
#[test]
fn wildcard_step() {
    let g = G::new();
    g.t.tx(|tx| {
        tx.assert(v("a"), v("knows"), v("b"), Valid::ALWAYS)?;
        tx.assert(v("a"), v("name"), s("Ann"), Valid::ALWAYS)?;
        tx.assert(
            v("a"),
            Value::iri(format!("{}type", vocab::RDF)),
            v("Person"),
            Valid::ALWAYS,
        )?;
        tx.set_vocab("urn:tiramemsu:v:")?;
        Ok(())
    });
    assert_eq!(g.now("a", "sys:anyRelationship", REACH), eh(&[("b", 1)]));
    // `sys:` statements are not relationships
    let view = g.t.db.now();
    let db_node = view.encode(&Value::iri(vocab::SYS_DB)).unwrap().unwrap();
    assert!(view
        .path(db_node, "sys:anyRelationship", REACH, 9)
        .unwrap()
        .is_empty());
    // an isEdge predicate makes its literal object a relationship end
    g.t.tx(|tx| {
        tx.assert(v("a"), v("site"), s("http://x"), Valid::ALWAYS)?;
        tx.assert(
            v("site"),
            Value::iri(vocab::SYS_IS_EDGE),
            Value::Bool(true),
            Valid::ALWAYS,
        )
        .map(|_| ())
    });
    let r = g.now("a", "sys:anyRelationship", REACH);
    assert!(r.iter().any(|(n, _)| n.contains("http://x")), "{r:?}");
}

// "Unsupported mode is rejected"
#[test]
fn unsupported_mode_is_rejected() {
    let g = G::new();
    g.edge("a", "p", "b");
    let e =
        g.t.db
            .read_sql(&format!(
                "SELECT * FROM tm_path({}, 'p', 'SIMPLE')",
                g.id("a").raw()
            ))
            .unwrap_err();
    assert!(e.to_string().contains("SIMPLE"), "{e}");
}

// "Set semantics over parallel edges" / "over diamond" / "Hops is shortest witness length"
#[test]
fn reach_set_semantics() {
    let g = G::new();
    g.edges(&[("a", "knows", "b")]);
    g.t.tx(|tx| {
        tx.create(v("a"), v("knows"), v("b"), Valid::ALWAYS)
            .map(|_| ())
    });
    assert_eq!(g.now("a", "knows+", REACH), eh(&[("b", 1)]));
    let g = G::new();
    g.edges(&[
        ("a", "p", "b"),
        ("a", "p", "c"),
        ("b", "p", "d"),
        ("c", "p", "d"),
    ]);
    assert_eq!(g.now("a", "p+", REACH), eh(&[("b", 1), ("c", 1), ("d", 2)]));
    let g = G::new();
    g.edges(&[("a", "p", "b"), ("b", "p", "c"), ("a", "p", "c")]);
    assert_eq!(g.now("a", "p+", REACH), eh(&[("b", 1), ("c", 1)]));
}

// "Zero-length match of an isolated start" / "of a literal start"
#[test]
fn zero_length_matches() {
    let g = G::new();
    let eids = g.edges(&[("nobody", "p", "x")]);
    g.t.tx(|tx| tx.retract(eids[0]).map(|_| ()));
    assert_eq!(g.now("nobody", "knows*", REACH), eh(&[("nobody", 0)]));
    let view = g.t.db.now();
    let lit = view.encode(&s("x")).unwrap().unwrap();
    let rows = view.path(lit, "knows*", REACH, u32::MAX).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!((rows[0].end, rows[0].hops), (lit, 0));
    assert!(rows[0].path.is_none());
}

// "Parallel edges give distinct trails" / "A relationship is not reused"
#[test]
fn trail_relationship_identity() {
    let g = G::new();
    let e = g.edges(&[("a", "knows", "b")]);
    g.t.tx(|tx| {
        tx.create(v("a"), v("knows"), v("b"), Valid::ALWAYS)
            .map(|_| ())
    });
    let view = g.t.db.now();
    let rows = view.path(g.id("a"), "knows{1}", TRAIL, 5).unwrap();
    assert_eq!(rows.len(), 2, "two relationships give two trails");
    let g2 = G::new();
    g2.edge("a", "knows", "b");
    let r = g2.paths("a", "(knows|^knows){1,3}", TRAIL, 5);
    assert_eq!(r, vec![vec!["a".to_string(), "b".to_string()]]);
    let _ = e;
}

// "Nodes may repeat on a trail" / "Zero-length trail"
#[test]
fn trail_nodes_repeat() {
    let g = G::new();
    let e = g.edges(&[("a", "p", "b"), ("b", "p", "a"), ("a", "p", "c")]);
    let view = g.t.db.now();
    let rows = view.path(g.id("a"), "p+", TRAIL, 15).unwrap();
    assert!(rows.iter().any(|r| {
        let p = r.path.as_ref().unwrap();
        p.nodes.iter().map(|n| g.name(*n)).collect::<Vec<_>>() == ["a", "b", "a", "c"]
    }));
    for r in &rows {
        let mut ids: Vec<_> = r
            .path
            .as_ref()
            .unwrap()
            .hops
            .iter()
            .map(|h| h.eid)
            .collect();
        let n = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), n);
    }
    let _ = e;
    let z = view.path(g.id("a"), "p*", TRAIL, 15).unwrap();
    assert_eq!(z[0].hops, 0);
    assert_eq!(z[0].path.as_ref().unwrap().nodes, vec![g.id("a")]);
    assert!(z[0].path.as_ref().unwrap().hops.is_empty());
}

// "Any shortest picks the minimal length" / "Shortest on a cycle" (any)
#[test]
fn any_shortest() {
    let g = G::new();
    g.edges(&[("a", "p", "b"), ("b", "p", "c"), ("a", "p", "c")]);
    let rows = g.paths("a", "p+", ANY, u32::MAX);
    assert!(rows.contains(&vec!["a".to_string(), "c".to_string()]));
    assert_eq!(
        g.ends(g.t.db.now(), "a", "p+", ANY, u32::MAX),
        eh(&[("b", 1), ("c", 1)])
    );
}

// "Any shortest is deterministic among ties"
#[test]
fn any_shortest_is_deterministic_among_ties() {
    let g = G::new();
    g.t.tx(|tx| {
        tx.create(v("a"), v("p"), v("b"), Valid::ALWAYS)?;
        tx.create(v("a"), v("p"), v("b"), Valid::ALWAYS)?;
        Ok(())
    });
    let view = g.t.db.now();
    let first = view.path(g.id("a"), "p", ANY, 5).unwrap();
    assert_eq!(first.len(), 1);
    let eids: Vec<i64> = view
        .triples(Some(g.id("a")), Some(g.id("p")), None)
        .unwrap()
        .iter()
        .map(|t| t.eid.oid().raw())
        .collect();
    let via = first[0].path.as_ref().unwrap().hops[0].eid.raw();
    assert_eq!(via, *eids.iter().min().unwrap(), "smallest eid wins");
    assert_eq!(view.path(g.id("a"), "p", ANY, 5).unwrap(), first);
}

// "All shortest returns every minimal path" / "does not duplicate ambiguous matches"
#[test]
fn all_shortest() {
    let g = G::new();
    g.edges(&[
        ("a", "p", "b"),
        ("a", "p", "c"),
        ("b", "p", "d"),
        ("c", "p", "d"),
        ("a", "q", "x"),
        ("x", "q", "y"),
        ("y", "q", "d"),
    ]);
    let rows = g.paths("a", "(p|q)+", ALL, u32::MAX);
    let to_d: Vec<_> = rows.iter().filter(|p| p.last().unwrap() == "d").collect();
    assert_eq!(to_d.len(), 2);
    assert!(to_d.iter().all(|p| p.len() == 3));
    let g = G::new();
    g.edge("a", "p", "b");
    assert_eq!(g.paths("a", "p|p", ALL, u32::MAX).len(), 1);
}

// "Shortest with both endpoints fixed" / "Only the end is bound" (engine level)
#[test]
fn end_restricted_search() {
    let g = G::new();
    chain(&g, "p", &["a", "b", "c", "d"]);
    let sql = format!(
        "SELECT \"end\", hops FROM tm_path({}, 'p+', 'ANY_SHORTEST') WHERE \"end\" = {}",
        g.id("a").raw(),
        g.id("d").raw()
    );
    let rows = g.t.db.read_sql(&sql).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0][0].as_i64(), Some(g.id("d").raw()));
    assert_eq!(rows[0][1].as_i64(), Some(3));
}
