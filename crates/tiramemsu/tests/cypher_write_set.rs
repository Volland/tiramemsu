//! `cypher-write`: SET, REMOVE, DELETE, DETACH DELETE, schema rules, time rules.
mod cypher_common;
use cypher_common::*;
use tiramemsu::*;

const TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

fn alice() -> T {
    let t = T::new();
    t.assert(&[(v("alice"), v("name"), sv("Alice"))]);
    t
}

fn eid_of_stmt(t: &T, s: &str, p: &str) -> Eid {
    t.db.now()
        .triples(None, None, None)
        .unwrap()
        .into_iter()
        .find(|tr| {
            let d = |id| t.db.now().decode(id).unwrap();
            d(tr.s) == v(s) && d(tr.p) == v(p)
        })
        .unwrap()
        .eid
}

// "First value is asserted"
#[test]
fn set_first_value_is_asserted() {
    let t = alice();
    let r = t.w("MATCH (n {name:'Alice'}) SET n.age = 42");
    assert_eq!(r.report.unwrap().asserted.len(), 1);
    assert!(t
        .live()
        .contains(&("alice".into(), "age".into(), "42".into())));
}

// "Same value is a no-op"
#[test]
fn set_same_value_is_a_noop() {
    let t = alice();
    t.assert(&[(v("alice"), v("age"), Value::Int(42))]);
    let e5 = eid_of_stmt(&t, "alice", "age");
    let rep = t
        .w("MATCH (n {name:'Alice'}) SET n.age = 42")
        .report
        .unwrap();
    assert!(rep.retracted.is_empty());
    assert!(rep.existing.contains(&e5));
    assert!(rep.asserted.is_empty());
}

// "Changing a value supersedes and keeps annotations"
#[test]
fn set_change_supersedes_and_keeps_annotations() {
    let t = alice();
    t.assert(&[(v("alice"), v("title"), sv("Dr"))]);
    let e5 = eid_of_stmt(&t, "alice", "title");
    t.assert(&[(Value::Stmt(e5), v("source"), sv("cv"))]);
    let rep = t
        .w("MATCH (n {name:'Alice'}) SET n.title = 'Prof'")
        .report
        .unwrap();
    assert!(rep
        .retracted
        .iter()
        .any(|(e, k)| *e == e5 && *k == RetKind::Supersede));
    assert!(!rep.superseded.is_empty());
    let live = t.live();
    assert!(live
        .iter()
        .any(|(s, p, o)| s == "alice" && p == "title" && o == "Prof"));
    assert!(live
        .iter()
        .any(|(s, p, o)| p == "source" && o == "cv" && s.starts_with("urn:tiramemsu:stmt:")));
    assert!(live
        .iter()
        .any(|(_, p, _)| p == "urn:tiramemsu:sys:supersedes"));
}

// "Cardinality-one predicate replaces without replay"
#[test]
fn set_cardinality_one_replaces_without_replay() {
    let t = alice();
    t.assert(&[(
        v("age"),
        Value::iri("urn:tiramemsu:sys:cardinality"),
        Value::iri("urn:tiramemsu:sys:one"),
    )]);
    t.assert(&[(v("alice"), v("age"), Value::Int(41))]);
    let e = eid_of_stmt(&t, "alice", "age");
    t.assert(&[(Value::Stmt(e), v("source"), sv("form"))]);
    let rep = t
        .w("MATCH (n {name:'Alice'}) SET n.age = 42")
        .report
        .unwrap();
    assert!(rep
        .retracted
        .iter()
        .any(|(x, k)| *x == e && *k == RetKind::Cardinality));
    assert!(t.live().iter().all(|(_, p, _)| p != "source"));
    assert!(t
        .live()
        .contains(&("alice".into(), "age".into(), "42".into())));
}

// "Multi-valued property is replaced" / "Setting null removes"
#[test]
fn set_multi_valued_and_null() {
    let t = alice();
    t.assert(&[(v("alice"), v("nick"), sv("al"))]);
    t.assert(&[(v("alice"), v("nick"), sv("ally"))]);
    let rep = t
        .w("MATCH (n {name:'Alice'}) SET n.nick = 'ali'")
        .report
        .unwrap();
    assert_eq!(rep.retracted.len(), 2);
    assert!(rep.retracted.iter().all(|(_, k)| *k == RetKind::Explicit));
    let nicks: Vec<_> = t
        .live()
        .into_iter()
        .filter(|(_, p, _)| p == "nick")
        .collect();
    assert_eq!(nicks.len(), 1);
    t.w("MATCH (n {name:'Alice'}) SET n.nick = null");
    assert!(t.live().iter().all(|(_, p, _)| p != "nick"));
}

// "SET on a relationship"
#[test]
fn set_on_a_relationship() {
    let t = T::new();
    t.assert(&[
        (v("alice"), rdf_type(), v("Person")),
        (v("alice"), v("name"), sv("Alice")),
        (v("alice"), v("worksAt"), v("acme")),
    ]);
    let e1 = eid_of_stmt(&t, "alice", "worksAt");
    t.w("MATCH (:Person {name:'Alice'})-[r:worksAt]->() SET r.confidence = 0.9");
    let d =
        t.db.now()
            .triples(
                Some(t.db.now().encode(&Value::Stmt(e1)).unwrap().unwrap()),
                None,
                None,
            )
            .unwrap();
    assert_eq!(d.len(), 1);
}

// "SET += merges properties" / "SET = replaces all properties" / "Labels set and removed"
#[test]
fn set_maps_and_labels() {
    let t = alice();
    t.assert(&[
        (v("alice"), v("age"), Value::Int(41)),
        (v("alice"), v("worksAt"), v("acme")),
        (v("alice"), rdf_type(), v("Person")),
    ]);
    t.w("MATCH (n {name:'Alice'}) SET n += {age: 42, city: 'Lviv'}");
    let l = t.live();
    assert!(l.contains(&("alice".into(), "age".into(), "42".into())));
    assert!(l.contains(&("alice".into(), "city".into(), "Lviv".into())));
    assert!(l.contains(&("alice".into(), "name".into(), "Alice".into())));
    t.w("MATCH (n {name:'Alice'}) SET n = {name: 'Alicia'}");
    let l = t.live();
    assert!(l.contains(&("alice".into(), "name".into(), "Alicia".into())));
    assert!(l
        .iter()
        .all(|(s, p, _)| !(s == "alice" && (p == "age" || p == "city"))));
    assert!(l.iter().any(|(_, p, _)| p == "worksAt"));
    assert!(l.iter().any(|(_, p, o)| p == TYPE && o == "Person"));
    t.w("MATCH (n {name:'Alicia'}) SET n:Admin REMOVE n:Person");
    assert_eq!(
        t.one("MATCH (n {name:'Alicia'}) RETURN labels(n)"),
        list(vec![s("Admin")])
    );
    assert!(matches!(
        t.werr("MATCH (a {name:'Alicia'}), (b) SET a = b"),
        Error::Unsupported { .. }
    ));
}

// "REMOVE cascades annotations"
#[test]
fn remove_cascades_annotations() {
    let t = alice();
    t.assert(&[(v("alice"), v("age"), Value::Int(42))]);
    let e5 = eid_of_stmt(&t, "alice", "age");
    t.assert(&[(Value::Stmt(e5), v("source"), sv("form"))]);
    let rep = t.w("MATCH (n {name:'Alice'}) REMOVE n.age").report.unwrap();
    assert!(rep
        .retracted
        .iter()
        .any(|(e, k)| *e == e5 && *k == RetKind::Explicit));
    assert!(rep.retracted.iter().any(|(_, k)| *k == RetKind::Cascade));
}

// "Delete a relationship" / "Delete null"
#[test]
fn delete_relationship_and_null() {
    let t = T::new();
    t.assert(&[
        (v("alice"), rdf_type(), v("Person")),
        (v("alice"), v("name"), sv("Alice")),
        (v("alice"), v("worksAt"), v("acme")),
    ]);
    let e1 = eid_of_stmt(&t, "alice", "worksAt");
    t.assert(&[(Value::Stmt(e1), v("confidence"), Value::Double(0.8))]);
    let rep = t
        .w("MATCH (:Person {name:'Alice'})-[r:worksAt]->() DELETE r")
        .report
        .unwrap();
    assert!(rep
        .retracted
        .iter()
        .any(|(e, k)| *e == e1 && *k == RetKind::Explicit));
    assert!(rep.retracted.iter().any(|(_, k)| *k == RetKind::Cascade));
    let past =
        t.db.as_of(TimeRef::Tx(2))
            .cypher("MATCH ()-[r:worksAt]->() RETURN r.confidence", &no_params())
            .unwrap();
    assert_eq!(past.rows, vec![vec![f(0.8)]]);
    let rep = t.w("OPTIONAL MATCH (n:Nobody) DELETE n").report.unwrap();
    assert!(rep.retracted.is_empty());
}

// "Node with relationships cannot be deleted" / "Deleting the relationships in the same query succeeds" / "Isolated node is deleted"
#[test]
fn delete_nodes() {
    let t = T::new();
    t.assert(&[
        (v("alice"), v("name"), sv("Alice")),
        (v("alice"), v("worksAt"), v("acme")),
        (v("zed"), v("name"), sv("Zed")),
        (v("zed"), rdf_type(), v("Person")),
    ]);
    let e = t.werr("MATCH (n {name:'Alice'}) DELETE n");
    match e {
        Error::DeleteConnectedNode { relationships, .. } => assert_eq!(relationships.len(), 1),
        other => panic!("{other:?}"),
    }
    assert!(t
        .live()
        .contains(&("alice".into(), "name".into(), "Alice".into())));
    t.w("MATCH (n {name:'Alice'})-[r]-() DELETE r, n");
    assert!(t.live().iter().all(|(s, _, _)| s != "alice"));
    t.w("MATCH (n {name:'Zed'}) DELETE n");
    assert_eq!(t.q("MATCH (n {name:'Zed'}) RETURN n").rows.len(), 0);
    assert!(t.live().iter().all(|(s, _, _)| s != "zed"));
}

// "Detach delete removes everything that mentions the node" / "History still shows the node"
#[test]
fn detach_delete() {
    let t = T::new();
    t.assert(&[
        (v("alice"), rdf_type(), v("Person")),
        (v("alice"), v("name"), sv("Alice")),
        (v("alice"), v("worksAt"), v("acme")),
        (v("bob"), v("knows"), v("alice")),
        (v("acme"), v("name"), sv("Acme")),
        (v("bob"), v("name"), sv("Bob")),
    ]);
    let e1 = eid_of_stmt(&t, "alice", "worksAt");
    t.assert(&[(Value::Stmt(e1), v("confidence"), Value::Double(0.8))]);
    let before = t.last_t();
    let rep = t
        .w("MATCH (n {name:'Alice'}) DETACH DELETE n")
        .report
        .unwrap();
    assert_eq!(rep.retracted.len(), 5);
    let l = t.live();
    assert!(l.contains(&("acme".into(), "name".into(), "Acme".into())));
    assert!(l.contains(&("bob".into(), "name".into(), "Bob".into())));
    assert_eq!(l.len(), 2);
    let r =
        t.db.as_of(TimeRef::Tx(before))
            .cypher("MATCH (n {name:'Alice'})-[r]->(c) RETURN c", &no_params())
            .unwrap();
    assert_eq!(r.rows.len(), 1);
}

// "valueType mismatch" / "Writing a reserved predicate" / "Cascade limit"
#[test]
fn schema_and_namespace_rules() {
    let t = alice();
    t.assert(&[(
        v("age"),
        Value::iri("urn:tiramemsu:sys:valueType"),
        Value::iri("http://www.w3.org/2001/XMLSchema#integer"),
    )]);
    assert!(matches!(
        t.werr("CREATE (n {age: 'old'})"),
        Error::ValueTypeMismatch { .. }
    ));
    assert!(matches!(
        t.werr("MATCH (n {name:'Alice'}) SET n.`sys:reason` = 'x'"),
        Error::ReservedNamespace(_)
    ));
    // cascade limit
    let t = T::new();
    t.assert(&[(v("a"), v("name"), sv("A")), (v("a"), v("knows"), v("b"))]);
    let e1 = eid_of_stmt(&t, "a", "knows");
    t.tx(|tx| {
        for k in 0..5 {
            tx.assert(
                Value::Stmt(e1),
                v(&format!("k{k}")),
                Value::Int(k),
                Valid::ALWAYS,
            )?;
        }
        Ok(())
    });
    let e =
        t.db.cypher_write(
            TxOptions {
                max_cascade: 2,
                ..TxOptions::default()
            },
            "MATCH (n {name:'A'}) DETACH DELETE n",
            &no_params(),
        )
        .unwrap_err();
    assert!(matches!(e, Error::CascadeLimitExceeded { .. }), "{e:?}");
    assert_eq!(t.live().len(), 7);
}

// "Query-level AS OF with a write rejected" / "Restore a past value from a historical scope"
#[test]
fn writes_and_time_clauses() {
    let t = T::new();
    t.assert(&[(v("alice"), v("title"), sv("Dr"))]); // tx1
    t.w("MATCH (n {`@id`:'v:alice'}) SET n.title = 'Prof'"); // tx2
    assert!(matches!(
        t.werr("USE AS OF 1 MATCH (n) SET n.x = 1"),
        Error::Unsupported { .. }
    ));
    t.w("CALL { USE AS OF 1 MATCH (a {`@id`: 'v:alice'}) RETURN a.title AS old } MATCH (n {`@id`: 'v:alice'}) SET n.title = old");
    assert_eq!(
        t.one("MATCH (n {`@id`: 'v:alice'}) RETURN n.title"),
        s("Dr")
    );
}

// "FOREACH in a write query"
#[test]
fn foreach_is_unsupported() {
    let t = T::new();
    let before = t.last_t();
    match t.werr("MATCH (n) FOREACH (x IN [1,2] | CREATE (:T {v: x}))") {
        Error::Unsupported { feature } => assert!(feature.contains("FOREACH")),
        other => panic!("{other:?}"),
    }
    assert_eq!(t.last_t(), before);
}
