//! `cypher-read`: read clauses, classification, patterns, expressions, values.
#![cfg(feature = "cypher")]
mod cypher_common;
use cypher_common::*;
use tiramemsu::*;

fn people() -> T {
    let t = T::new();
    t.assert(&[
        (v("alice"), rdf_type(), v("Person")),
        (v("acme"), rdf_type(), v("Company")),
        (v("alice"), v("worksAt"), v("acme")),
        (v("alice"), v("name"), sv("Alice")),
    ]);
    t
}

// cypher-read "Read query on the current view"
#[test]
fn read_query_on_the_current_view() {
    let t = T::new();
    t.assert(&[
        (v("alice"), rdf_type(), v("Person")),
        (v("alice"), v("name"), sv("Alice")),
    ]);
    let r = t.q("MATCH (p:Person) RETURN p.name AS name");
    assert_eq!(r.columns, vec!["name"]);
    assert_eq!(r.rows, vec![vec![s("Alice")]]);
}

// cypher-read "Write clause on a read-only view is rejected"
#[test]
fn write_clause_on_a_read_only_view_is_rejected() {
    let t = T::new();
    let before = t.last_t();
    let e = t.qerr("CREATE (n:Person {name: 'Bob'})");
    assert!(
        matches!(e, Error::Unsupported { ref feature } if feature.contains("CREATE")),
        "{e:?}"
    );
    assert_eq!(t.last_t(), before);
    assert!(t.live().is_empty());
}

// cypher-read "View time selection is the default"
#[test]
fn view_time_selection_is_the_default() {
    let t = T::new();
    t.assert(&[(v("alice"), v("name"), sv("Alice"))]); // tx 1
    t.tx(|tx| {
        let (a, b, c) = (
            tx.encode(v("alice"))?,
            tx.encode(v("name"))?,
            tx.encode(sv("Alice"))?,
        );
        tx.retract_matching(Some(a), Some(b), Some(c))?;
        Ok(())
    }); // tx 2
    let q = "MATCH (n) WHERE n.name = 'Alice' RETURN n";
    let past = t.db.as_of(TimeRef::Tx(1)).cypher(q, &no_params()).unwrap();
    assert_eq!(past.rows.len(), 1);
    assert_eq!(t.q(q).rows.len(), 0);
}

// cypher-read "Literal object is a property" / "Node object is a relationship"
#[test]
fn literal_object_is_a_property_and_node_object_a_relationship() {
    let t = T::new();
    t.assert(&[
        (v("alice"), v("age"), Value::Int(42)),
        (v("alice"), v("knows"), v("bob")),
    ]);
    assert_eq!(t.one("MATCH (a)-[r:age]->(x) RETURN count(r) AS c"), i(0));
    assert_eq!(
        t.one("MATCH (a)-[r]->(x) WHERE a.age = 42 RETURN count(r) AS c"),
        i(1)
    );
    let r = t.q("MATCH (a)-[r:knows]->(b) RETURN b");
    assert_eq!(r.rows.len(), 1);
    assert_eq!(short(&r.rows[0][0]), "bob");
    assert_eq!(
        t.q("MATCH (a) WHERE a.knows IS NOT NULL RETURN a")
            .rows
            .len(),
        0
    );
    assert_eq!(t.one("MATCH (a {`@id`: 'v:alice'}) RETURN a.age"), i(42));
}

// cypher-read "sys:isEdge true forces the relationship view on a literal"
#[test]
fn is_edge_true_forces_relationship_on_literal() {
    let t = T::new();
    t.assert(&[
        (
            Value::iri("urn:tiramemsu:v:tag"),
            Value::iri("urn:tiramemsu:sys:isEdge"),
            Value::Bool(true),
        ),
        (v("alice"), v("tag"), sv("urgent")),
    ]);
    let r = t.q("MATCH (a)-[r:tag]->(t) RETURN t, a.tag");
    assert_eq!(r.rows, vec![vec![s("urgent"), null()]]);
}

// cypher-read "sys:isEdge false forces the property view on a node object"
#[test]
fn is_edge_false_forces_property_on_node_object() {
    let t = T::new();
    t.assert(&[
        (
            Value::iri("urn:tiramemsu:v:homepage"),
            Value::iri("urn:tiramemsu:sys:isEdge"),
            Value::Bool(false),
        ),
        (
            v("alice"),
            v("homepage"),
            Value::iri("https://alice.example/"),
        ),
    ]);
    assert_eq!(
        t.one("MATCH (a {`@id`: 'v:alice'}) RETURN a.homepage"),
        s("https://alice.example/")
    );
    assert_eq!(t.q("MATCH ()-[r:homepage]->() RETURN r").rows.len(), 0);
}

// cypher-read "rdf:type is never a relationship"
#[test]
fn rdf_type_is_never_a_relationship() {
    let t = T::new();
    t.assert(&[(v("alice"), rdf_type(), v("Person"))]);
    assert_eq!(t.q("MATCH ()-[r]->() RETURN r").rows.len(), 0);
}

// cypher-read "Label matches rdf:type" / "Multiple labels are conjunctive" / "Label disjunction"
#[test]
fn labels() {
    let t = people();
    let r = t.q("MATCH (n:Person) RETURN n");
    assert_eq!(r.rows.len(), 1);
    assert_eq!(short(&r.rows[0][0]), "alice");
    assert_eq!(t.one("MATCH (n:Person|Company) RETURN count(n) AS c"), i(2));
    t.assert(&[
        (v("alice"), rdf_type(), v("Employee")),
        (v("bob"), rdf_type(), v("Person")),
    ]);
    let r = t.q("MATCH (n:Person:Employee) RETURN n");
    assert_eq!(r.rows.len(), 1);
    assert_eq!(short(&r.rows[0][0]), "alice");
}

// cypher-read "Duplicate label statements do not duplicate rows"
#[test]
fn duplicate_label_statements_do_not_duplicate_rows() {
    let t = T::new();
    t.tx(|tx| {
        tx.assert(v("alice"), rdf_type(), v("Person"), Valid::between(0, 1000))?;
        tx.assert(v("alice"), rdf_type(), v("Person"), Valid::from(2000))?;
        Ok(())
    });
    assert_eq!(t.q("MATCH (n:Person) RETURN n").rows.len(), 1);
}

// cypher-read "Inline property map filters" / "Numeric property map compares by value"
#[test]
fn inline_property_maps() {
    let t = T::new();
    t.assert(&[
        (v("a"), rdf_type(), v("Person")),
        (v("a"), v("name"), sv("Alice")),
        (v("b"), rdf_type(), v("Person")),
        (v("b"), v("name"), sv("Bob")),
        (v("x"), v("score"), Value::Int(30)),
    ]);
    let r = t.q("MATCH (n:Person {name: 'Alice'}) RETURN n");
    assert_eq!(r.rows.len(), 1);
    assert_eq!(short(&r.rows[0][0]), "a");
    let r = t.q("MATCH (n {score: 30.0}) RETURN n");
    assert_eq!(r.rows.len(), 1);
    assert_eq!(short(&r.rows[0][0]), "x");
}

// cypher-read "Unlabelled node scan excludes statements, transactions and class IRIs"
#[test]
fn node_scan_excludes_statements_transactions_and_classes() {
    let t = T::new();
    let rep = t.tx(|tx| {
        tx.assert(v("alice"), rdf_type(), v("Person"), Valid::ALWAYS)?;
        let e1 = tx
            .assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?
            .eid();
        tx.assert(e1, v("confidence"), Value::Double(0.8), Valid::ALWAYS)?;
        tx.meta(Value::iri("urn:tiramemsu:sys:author"), v("agent7"))?;
        Ok(())
    });
    assert!(!rep.asserted.is_empty());
    let r = t.q("MATCH (n) RETURN n ORDER BY elementId(n)");
    let ids: Vec<String> = r.rows.iter().map(|x| short(&x[0])).collect();
    assert_eq!(ids, vec!["acme", "alice"]);
}

// cypher-read relationship patterns
#[test]
fn relationship_patterns() {
    let t = T::new();
    t.assert(&[(v("alice"), v("worksAt"), v("acme"))]);
    let r = t.q("MATCH (c)<-[:worksAt]-(p) RETURN p, c");
    assert_eq!(short(&r.rows[0][0]), "alice");
    assert_eq!(short(&r.rows[0][1]), "acme");
    let t2 = T::new();
    t2.assert(&[(v("alice"), v("knows"), v("bob"))]);
    let r = t2.q("MATCH (a)-[:knows]-(b) RETURN a, b");
    let mut pairs: Vec<(String, String)> = r
        .rows
        .iter()
        .map(|x| (short(&x[0]), short(&x[1])))
        .collect();
    pairs.sort();
    assert_eq!(
        pairs,
        vec![
            ("alice".into(), "bob".into()),
            ("bob".into(), "alice".into())
        ]
    );
}

// cypher-read "Parallel edges are distinct rows" / "Type disjunction"
#[test]
fn parallel_edges_and_type_disjunction() {
    let t = T::new();
    t.tx(|tx| {
        tx.assert(v("alice"), rdf_type(), v("Person"), Valid::ALWAYS)?;
        tx.create(v("alice"), v("called"), v("bob"), Valid::ALWAYS)?;
        tx.create(v("alice"), v("called"), v("bob"), Valid::ALWAYS)?;
        tx.assert(v("alice"), v("knows"), v("bob"), Valid::ALWAYS)?;
        tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?;
        Ok(())
    });
    assert_eq!(
        t.one("MATCH (:Person)-[r:called]->(b) RETURN count(r) AS c"),
        i(2)
    );
    assert_eq!(
        t.one("MATCH (v)-[r:knows|worksAt]->(x) RETURN count(*) AS c"),
        i(2)
    );
}

// cypher-read "Untyped pattern hides sys predicates"
#[test]
fn untyped_pattern_hides_sys_predicates() {
    let t = T::new();
    t.assert(&[(v("alice"), v("title"), sv("Dr"))]);
    let e = t.tx(|tx| {
        let old = tx.encode(v("alice"))?;
        let _ = old;
        Ok(())
    });
    let _ = e;
    let old = t.db.now().triples(None, None, None).unwrap()[0].eid;
    t.tx(|tx| {
        tx.supersede(old, Patch::object(sv("Prof")))?;
        Ok(())
    });
    let r = t.q("MATCH ()-[r]->() RETURN type(r)");
    assert!(
        r.rows.iter().all(|x| x[0] != s("sys:supersedes")),
        "{:?}",
        r.rows
    );
    assert_eq!(
        t.q("MATCH (new)-[:`sys:supersedes`]->(old) RETURN elementId(new), elementId(old)")
            .rows
            .len(),
        1
    );
}

// cypher-read "Relationship property map"
#[test]
fn relationship_property_map() {
    let t = T::new();
    t.tx(|tx| {
        let e1 = tx
            .assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?
            .eid();
        let e2 = tx
            .assert(v("bob"), v("worksAt"), v("acme"), Valid::ALWAYS)?
            .eid();
        tx.assert(e1, v("role"), sv("cto"), Valid::ALWAYS)?;
        tx.assert(e2, v("role"), sv("dev"), Valid::ALWAYS)?;
        Ok(())
    });
    let r = t.q("MATCH (p)-[r:worksAt {role: 'cto'}]->(c) RETURN p");
    assert_eq!(r.rows.len(), 1);
    assert_eq!(short(&r.rows[0][0]), "alice");
}
