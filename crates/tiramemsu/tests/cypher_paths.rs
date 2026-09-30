//! `path-lowering` (Cypher): variable-length relationships, path and list bindings,
//! relationship isomorphism, shortest paths, endpoint binding, temporal scope and the
//! unsupported forms.
mod cypher_common;
use cypher_common::*;
use tiramemsu::*;

/// A graph of `(from, TYPE, to)` relationships whose nodes carry an `id` property.
fn graph(edges: &[(&str, &str, &str)]) -> T {
    let t = T::new();
    t.tx(|tx| {
        let mut seen = Vec::new();
        for (a, ty, b) in edges {
            for n in [a, b] {
                if !seen.contains(n) {
                    seen.push(*n);
                    tx.assert(v(n), v("id"), sv(n), Valid::ALWAYS)?;
                }
            }
            tx.assert(v(a), v(ty), v(b), Valid::ALWAYS)?;
        }
        Ok(())
    });
    t
}

fn ids(t: &T, q: &str) -> Vec<String> {
    let r = t.q(q);
    let mut out: Vec<String> = r
        .rows
        .iter()
        .map(|row| match &row[0] {
            CypherValue::String(s) => s.clone(),
            other => format!("{other:?}"),
        })
        .collect();
    out.sort();
    out
}

fn knows() -> T {
    graph(&[
        ("a", "KNOWS", "b"),
        ("b", "KNOWS", "c"),
        ("c", "KNOWS", "d"),
    ])
}

// "Default bounds" / "Exact length" / "Range with zero minimum" / "Upper bound only"
#[test]
fn quantifier_forms() {
    let t = knows();
    assert_eq!(
        ids(&t, "MATCH (x {id:'a'})-[:KNOWS*]->(y) RETURN y.id"),
        ["b", "c", "d"]
    );
    assert_eq!(
        ids(&t, "MATCH (x {id:'a'})-[:KNOWS*2]->(y) RETURN y.id"),
        ["c"]
    );
    assert_eq!(
        ids(&t, "MATCH (x {id:'a'})-[:KNOWS*0..1]->(y) RETURN y.id"),
        ["a", "b"]
    );
    assert_eq!(
        ids(&t, "MATCH (x {id:'a'})-[:KNOWS*..2]->(y) RETURN y.id"),
        ["b", "c"]
    );
    assert_eq!(
        ids(&t, "MATCH (x {id:'a'})-[:KNOWS*2..]->(y) RETURN y.id"),
        ["c", "d"]
    );
}

// @lat: [[tests#Query#Unbounded Paths Are Capped]]
// "Unbounded pattern stops at the cap" (and the configured cap)
#[test]
fn unbounded_pattern_stops_at_the_cap() {
    let names: Vec<String> = (0..=20).map(|i| format!("n{i}")).collect();
    let edges: Vec<(&str, &str, &str)> = names
        .windows(2)
        .map(|w| (w[0].as_str(), "NEXT", w[1].as_str()))
        .collect();
    let t = graph(&edges);
    assert_eq!(
        t.one("MATCH (s {id:'n0'})-[:NEXT*]->(e) RETURN count(*)"),
        i(15)
    );
    assert_eq!(
        t.one("MATCH (s {id:'n0'})-[:NEXT*1..18]->(e) RETURN count(*)"),
        i(18),
        "an explicit bound above the cap is honoured"
    );
    // a configured cap
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(
        dir.path().join("c.db"),
        OpenOptions {
            path_max_hops: 5,
            ..OpenOptions::default()
        },
    )
    .unwrap();
    db.transact(TxOptions::default(), |tx| {
        for w in names.windows(2) {
            tx.assert(v(&w[0]), v("id"), sv(&w[0]), Valid::ALWAYS)?;
            tx.assert(v(&w[0]), v("NEXT"), v(&w[1]), Valid::ALWAYS)?;
        }
        Ok(())
    })
    .unwrap();
    let r = db
        .now()
        .cypher(
            "MATCH (s {id:'n0'})-[:NEXT*]->(e) RETURN count(*)",
            &no_params(),
        )
        .unwrap();
    assert_eq!(r.rows[0][0], i(5));
}

// "Type alternation" / "Any type" / "Incoming direction" / "Undirected" / "Bag semantics over parallel relationships"
#[test]
fn types_and_directions() {
    let t = graph(&[("a", "KNOWS", "b"), ("b", "LIKES", "c")]);
    assert_eq!(
        ids(&t, "MATCH (x {id:'a'})-[:KNOWS|LIKES*]->(y) RETURN y.id"),
        ["b", "c"]
    );
    t.assert(&[(v("b"), v("name"), sv("Bob"))]);
    let r = t.q("MATCH (x {id:'a'})-[*]->(y) RETURN y");
    assert_eq!(r.rows.len(), 2);
    let mut got: Vec<String> = r.rows.iter().map(|row| short(&row[0])).collect();
    got.sort();
    assert_eq!(got, ["b", "c"], "the literal \"Bob\" is never a node");
    let k = knows();
    assert_eq!(
        ids(&k, "MATCH (y {id:'c'})<-[:KNOWS*]-(x) RETURN x.id"),
        ["a", "b"]
    );
    let u = graph(&[("a", "KNOWS", "b"), ("c", "KNOWS", "b")]);
    assert_eq!(
        ids(&u, "MATCH (x {id:'a'})-[:KNOWS*2]-(y) RETURN y.id"),
        ["c"]
    );
    let p = T::new();
    p.tx(|tx| {
        tx.assert(v("a"), v("id"), sv("a"), Valid::ALWAYS)?;
        tx.create(v("a"), v("CALLED"), v("b"), Valid::ALWAYS)?;
        tx.create(v("a"), v("CALLED"), v("b"), Valid::ALWAYS)?;
        Ok(())
    });
    assert_eq!(
        p.one("MATCH (x {id:'a'})-[:CALLED*1..1]->(y) RETURN count(*)"),
        i(2)
    );
}

// "Path value functions" / "Path order with a bound right-hand node" / "Relationship list variable"
#[test]
fn path_and_list_bindings() {
    let t = graph(&[("a", "KNOWS", "b"), ("b", "KNOWS", "c")]);
    let r = t.q("MATCH p = (x {id:'a'})-[:KNOWS*2]->(y) \
         RETURN length(p), [n IN nodes(p) | n.id], [r IN relationships(p) | id(r)]");
    assert_eq!(r.rows.len(), 1);
    assert_eq!(r.rows[0][0], i(2));
    assert_eq!(r.rows[0][1], list(vec![s("a"), s("b"), s("c")]));
    let CypherValue::List(rels) = &r.rows[0][2] else {
        panic!()
    };
    assert_eq!(rels.len(), 2);
    let e1 =
        t.db.now()
            .triples(
                Some(t.db.now().encode(&v("a")).unwrap().unwrap()),
                Some(t.db.now().encode(&v("KNOWS")).unwrap().unwrap()),
                None,
            )
            .unwrap()[0]
            .eid
            .oid()
            .raw();
    assert_eq!(
        rels[0],
        i(e1),
        "the same relationship values a single-hop MATCH binds"
    );
    // the right-hand node is bound: paths read left to right
    let r = t.q("MATCH p = (x)-[:KNOWS*]->(y {id:'c'}) RETURN [n IN nodes(p) | n.id] AS ns");
    let mut got: Vec<CypherValue> = r.rows.iter().map(|row| row[0].clone()).collect();
    got.sort_by_key(|v| format!("{v:?}"));
    assert_eq!(
        got,
        vec![
            list(vec![s("a"), s("b"), s("c")]),
            list(vec![s("b"), s("c")]),
        ]
    );
    let r = t.q("MATCH (x {id:'a'})-[rs:KNOWS*2]->(y) RETURN size(rs), [r IN rs | id(r)]");
    assert_eq!(r.rows[0][0], i(2));
    assert_eq!(r.rows[0][1], rels_ids(&t));
}

fn rels_ids(t: &T) -> CypherValue {
    let view = t.db.now();
    let knows = view.encode(&v("KNOWS")).unwrap().unwrap();
    let mut es: Vec<_> = view.triples(None, Some(knows), None).unwrap();
    es.sort_by_key(|x| x.eid.oid().raw());
    list(es.iter().map(|x| i(x.eid.oid().raw())).collect())
}

// "Virtual hop in a Cypher path"
// (the layer-hop path spec `tests#Query#Paths Cross Layers` is covered by the engine test `paths_cross_layers`)
#[test]
fn virtual_hop_in_a_cypher_path() {
    let t = T::new();
    t.tx(|tx| {
        let e1 = tx
            .assert(v("alice"), v("WORKS_AT"), v("acme"), Valid::ALWAYS)?
            .eid();
        tx.assert(v("belief9"), rdf_type(), v("Belief"), Valid::ALWAYS)?;
        tx.assert(v("belief9"), v("SUPPORTED_BY"), e1, Valid::ALWAYS)?;
        Ok(())
    });
    let r = t.q("MATCH p = (b:Belief)-[:SUPPORTED_BY|`sys:subject`*2]->(x) \
         RETURN [r IN relationships(p) | type(r)] AS types, x");
    assert_eq!(r.rows.len(), 1, "{:?}", r.rows);
    assert_eq!(
        r.rows[0][0],
        list(vec![s("SUPPORTED_BY"), s("sys:subject")])
    );
    assert_eq!(short(&r.rows[0][1]), "alice");
}

// "Variable-length pattern avoids a fixed relationship" / "Two variable-length patterns share no relationship"
#[test]
fn relationship_isomorphism_with_paths() {
    let t = graph(&[("a", "KNOWS", "b"), ("b", "KNOWS", "c")]);
    let r = t.q("MATCH (x {id:'a'})-[r:KNOWS]->(y), (y)<-[:KNOWS*1..2]-(z) RETURN z.id");
    assert!(r.rows.is_empty(), "{:?}", r.rows);
    assert_eq!(
        t.one("MATCH (x {id:'a'})-[:KNOWS*]->(m), (m)<-[:KNOWS*]-(w) RETURN count(*)"),
        i(0)
    );
}

// "shortestPath between two bound nodes" / "allShortestPaths returns every minimal path" /
// "shortestPath is deterministic" / "shortestPath with a larger minimum" / "No path"
#[test]
fn shortest_paths() {
    let t = graph(&[("a", "R", "b"), ("b", "R", "c"), ("a", "R", "c")]);
    assert_eq!(
        t.one(
            "MATCH (x {id:'a'}), (y {id:'c'}), p = shortestPath((x)-[:R*]->(y)) RETURN length(p)"
        ),
        i(1)
    );
    let d = graph(&[
        ("a", "R", "b"),
        ("a", "R", "c"),
        ("b", "R", "d"),
        ("c", "R", "d"),
    ]);
    let r = d.q(
        "MATCH (x {id:'a'}), (y {id:'d'}), p = allShortestPaths((x)-[:R*]-(y)) \
         RETURN [n IN nodes(p) | n.id]",
    );
    let mut got: Vec<CypherValue> = r.rows.iter().map(|row| row[0].clone()).collect();
    got.sort_by_key(|v| format!("{v:?}"));
    assert_eq!(
        got,
        vec![
            list(vec![s("a"), s("b"), s("d")]),
            list(vec![s("a"), s("c"), s("d")])
        ]
    );
    let q = "MATCH (x {id:'a'}), (y {id:'d'}), p = shortestPath((x)-[:R*]->(y)) \
             RETURN [n IN nodes(p) | n.id]";
    assert_eq!(d.q(q).rows, d.q(q).rows);
    match d.qerr("MATCH p = shortestPath((x {id:'a'})-[:R*2..5]->(y)) RETURN p") {
        Error::Unsupported { feature } => assert!(feature.contains("minimum"), "{feature}"),
        other => panic!("{other:?}"),
    }
    assert!(t
        .q("MATCH (x {id:'a'}), (y {id:'z'}), p = shortestPath((x)-[:R*]->(y)) RETURN p")
        .rows
        .is_empty());
    let o = t.q(
        "MATCH (x {id:'a'}) OPTIONAL MATCH (y {id:'z'}), p = shortestPath((x)-[:R*]->(y)) RETURN p",
    );
    assert_eq!(o.rows, vec![vec![null()]]);
}

// "Endpoint from a label scan" / "No bound endpoint"
#[test]
fn endpoint_binding() {
    let t = graph(&[("a", "KNOWS", "b"), ("b", "KNOWS", "c")]);
    t.assert(&[(v("a"), rdf_type(), v("Person"))]);
    let r = t.q("MATCH (x:Person)-[:KNOWS*1..2]->(y) RETURN x, y");
    assert_eq!(r.rows.len(), 2);
    match t.qerr("MATCH (x)-[:KNOWS*]->(y) RETURN x, y") {
        Error::Unsupported { feature } => assert!(feature.contains("bound endpoint"), "{feature}"),
        other => panic!("{other:?}"),
    }
}

// "Trail as of an earlier transaction" / "Shortest path in a time-scoped subquery"
#[test]
fn temporal_scope() {
    let t = T::new();
    t.advance_to(9);
    t.assert(&[(v("a"), v("id"), sv("a")), (v("a"), v("KNOWS"), v("b"))]);
    t.advance_to(19);
    t.assert(&[(v("b"), v("KNOWS"), v("c"))]);
    assert_eq!(
        ids(
            &t,
            "USE AS OF 15 MATCH (x {id:'a'})-[:KNOWS*]->(y) RETURN y.id"
        )
        .len(),
        1
    );
    let r = t.q("USE AS OF 15 MATCH (x {id:'a'})-[:KNOWS*]->(y) RETURN y");
    assert_eq!(short(&r.rows[0][0]), "b");
    assert_eq!(
        ids(&t, "MATCH (x {id:'a'})-[:KNOWS*]->(y) RETURN y.id").len(),
        2
    );
    // a time-scoped subquery beside the current view
    let s = graph(&[("a", "R", "b"), ("b", "R", "c"), ("c", "R", "d")]);
    s.advance_to(49);
    s.assert(&[(v("a"), v("R"), v("d"))]);
    let r = s.q(
        "CALL { USE AS OF 49 MATCH (x {id:'a'}), (y {id:'d'}), p = shortestPath((x)-[:R*]->(y)) \
         RETURN length(p) AS before } \
         MATCH (x {id:'a'}), (y {id:'d'}), p = shortestPath((x)-[:R*]->(y)) \
         RETURN before, length(p) AS now",
    );
    assert_eq!(r.rows, vec![vec![i(3), i(1)]]);
}

// "Property map on a variable-length relationship" / "Quantified path pattern" /
// "Repeatable elements with a variable-length pattern"
#[test]
fn unsupported_forms() {
    let t = knows();
    for (q, what) in [
        (
            "MATCH (x {id:'a'})-[:KNOWS*1..3 {since: 2020}]->(y) RETURN y",
            "property maps on variable-length relationships",
        ),
        (
            "MATCH (x {id:'a'}) ((m)-[:KNOWS]->(n)){1,3} (y) RETURN y",
            "quantified path patterns",
        ),
        (
            "MATCH REPEATABLE ELEMENTS (x {id:'a'})-[:KNOWS*1..3]->(y) RETURN y",
            "walk semantics",
        ),
    ] {
        match t.qerr(q) {
            Error::Unsupported { feature } => assert!(feature.contains(what), "{q}: {feature}"),
            other => panic!("{q}: {other:?}"),
        }
    }
}
