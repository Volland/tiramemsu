//! `cypher-write`: entry points, CREATE, MERGE.
#![cfg(feature = "cypher")]
mod cypher_common;
use cypher_common::*;
use tiramemsu::*;

fn live_has(t: &T, s: &str, p: &str, o: &str) -> bool {
    t.live()
        .contains(&(s.to_string(), p.to_string(), o.to_string()))
}

// "Create and return in one transaction"
#[test]
fn create_and_return_in_one_transaction() {
    let t = T::new();
    let r = t.w("CREATE (n:Person {name: 'Bob'}) RETURN n.name AS name");
    assert_eq!(r.rows, vec![vec![s("Bob")]]);
    let rep = r.report.unwrap();
    assert_eq!(rep.asserted.len(), 2);
    assert_eq!(rep.t.0, 1);
}

// "Later clauses see earlier writes"
#[test]
fn later_clauses_see_earlier_writes() {
    let t = T::new();
    let r = t.w("CREATE (n:Temp {k: 1}) WITH n MATCH (m:Temp) RETURN count(m) AS c");
    assert_eq!(r.rows, vec![vec![i(1)]]);
}

// "Failure leaves no trace"
#[test]
fn failure_leaves_no_trace() {
    let t = T::new();
    t.assert(&[
        (
            v("email"),
            Value::iri("urn:tiramemsu:sys:unique"),
            Value::Bool(true),
        ),
        (v("alice"), v("email"), sv("a@x")),
    ]);
    let before = t.last_t();
    let e = t.werr("CREATE (n:Person {name: 'Eve'}) CREATE (m {email: 'a@x'})");
    assert!(matches!(e, Error::UniqueViolation { .. }), "{e:?}");
    assert_eq!(t.last_t(), before);
    assert_eq!(t.q("MATCH (n {name: 'Eve'}) RETURN n").rows.len(), 0);
}

// "Transaction handle composes with API operations"
#[test]
fn transaction_handle_composes_with_api_operations() {
    let t = T::new();
    let rep =
        t.db.transact(TxOptions::default(), |tx| {
            tx.assert(v("alice"), v("age"), Value::Int(42), Valid::ALWAYS)?;
            tx.cypher("MATCH (n) WHERE n.age = 42 SET n:Adult", &no_params())?;
            Ok(())
        })
        .unwrap();
    assert_eq!(rep.t.0, 1);
    assert!(live_has(
        &t,
        "alice",
        "http://www.w3.org/1999/02/22-rdf-syntax-ns#type",
        "Adult"
    ));
}

// "Anonymous node with label and properties"
#[test]
fn anonymous_node_with_label_and_properties() {
    let t = T::new();
    let r = t.w("CREATE (n:Person {name: 'Bob', age: 30}) RETURN elementId(n) AS id");
    let CypherValue::String(id) = &r.rows[0][0] else {
        panic!()
    };
    assert!(id.starts_with("urn:tiramemsu:node:"), "{id}");
    assert_eq!(t.live().len(), 3);
    assert_eq!(t.one("MATCH (n:Person) RETURN n.age"), i(30));
}

// "Node with an explicit IRI"
#[test]
fn node_with_an_explicit_iri() {
    let t = T::new();
    t.w("CREATE (n:Person {`@id`: 'v:carol', name: 'Carol'})");
    assert!(live_has(
        &t,
        "carol",
        "http://www.w3.org/1999/02/22-rdf-syntax-ns#type",
        "Person"
    ));
    assert!(live_has(&t, "carol", "name", "Carol"));
    assert!(t.live().iter().all(|(_, p, _)| !p.contains("@id")));
}

// "Empty node leaves no statement"
#[test]
fn empty_node_leaves_no_statement() {
    let t = T::new();
    let r = t.w("CREATE (n) RETURN n");
    assert_eq!(r.rows.len(), 1);
    assert!(matches!(r.rows[0][0], CypherValue::Node(_)));
    assert!(r.report.unwrap().asserted.is_empty());
    assert_eq!(t.one("MATCH (n) RETURN count(n) AS c"), i(0));
}

// "Parallel edges" / "Relationship properties are layer statements" / "Path creation"
#[test]
fn create_relationships() {
    let t = T::new();
    t.w("CREATE (:P {name: 'Alice'}), (:P {name: 'Bob'})");
    let q = "MATCH (a {name:'Alice'}), (b {name:'Bob'}) CREATE (a)-[:CALLED]->(b)";
    t.w(q);
    t.w(q);
    assert_eq!(
        t.one("MATCH ({name:'Alice'})-[r:CALLED]->() RETURN count(r)"),
        i(2)
    );
    let r = t.w("MATCH (a {name:'Alice'}), (b {name:'Bob'}) CREATE (a)-[r:worksAt {confidence: 0.8}]->(b) RETURN r");
    let CypherValue::Relationship(rel) = &r.rows[0][0] else {
        panic!()
    };
    assert_eq!(rel.properties["confidence"], f(0.8));
    let t2 = T::new();
    t2.w("CREATE (a:Person {name:'X'})-[:knows]->(b:Person {name:'Y'})-[:knows]->(a)");
    assert_eq!(t2.one("MATCH ()-[r:knows]->() RETURN count(r)"), i(2));
    assert_eq!(t2.one("MATCH (n:Person) RETURN count(n)"), i(2));
}

// "Valid time on creation"
#[test]
fn valid_time_on_creation() {
    let t = T::new();
    t.w("CREATE (:P {name: 'Alice'}), (:P {name: 'Acme'})");
    let r = t.w(
        "MATCH (a {name:'Alice'}), (c {name:'Acme'}) CREATE (a)-[r:worksAt {validFrom: date('2025-01-01')}]->(c) RETURN r.validFrom AS f",
    );
    assert_eq!(r.to_json()["rows"][0][0], "2025-01-01T00:00:00.000Z");
    assert!(t.live().iter().all(|(_, p, _)| p != "validFrom"));
    let e =
        t.werr("MATCH (a {name:'Alice'}), (c {name:'Acme'}) CREATE (a)-[r:x {validFrom: 5}]->(c)");
    assert!(matches!(e, Error::Eval { .. }), "{e:?}");
}

// "List property becomes several statements" / "Map value rejected"
#[test]
fn list_and_map_values() {
    let t = T::new();
    let r = t.w("CREATE (n:Doc {tags: ['a', 'b']}) RETURN n.tags AS t");
    assert_eq!(r.rows[0][0], list(vec![s("a"), s("b")]));
    assert_eq!(t.live().iter().filter(|(_, p, _)| p == "tags").count(), 2);
    let before = t.last_t();
    let e = t.werr("CREATE (n {meta: {x: 1}})");
    assert!(matches!(e, Error::Eval { .. }));
    assert_eq!(t.last_t(), before);
    assert!(matches!(
        t.werr("CREATE (n {l: [1, null]})"),
        Error::Eval { .. }
    ));
}

// "DateTime keeps its offset" / "LocalDateTime is a date-time without timezone" / "Named zone stored as its offset"
#[test]
fn datetime_write_encoding() {
    let t = T::new();
    let r = t.w("CREATE (n {at: datetime('2025-03-01T10:00:00+02:00')}) RETURN n.at AS at");
    assert_eq!(r.to_json()["rows"][0][0], "2025-03-01T10:00:00.000+02:00");
    assert!(t
        .live()
        .iter()
        .any(|(_, p, o)| p == "at" && o == "2025-03-01T10:00:00.000+02:00"));
    let r = t.w("CREATE (n {at: localdatetime('2025-03-01T10:00:00')}) RETURN n.at AS at");
    assert_eq!(r.to_json()["rows"][0][0], "2025-03-01T10:00:00.000");
    let r = t.w("CREATE (n {at: datetime('2025-07-01T09:00:00[Europe/Kyiv]')}) RETURN n.at AS at");
    assert_eq!(r.to_json()["rows"][0][0], "2025-07-01T09:00:00.000+03:00");
}

// "Same instant with another offset is a different value"
#[test]
fn same_instant_other_offset_is_a_supersede() {
    let t = T::new();
    t.assert(&[(
        v("m"),
        v("at"),
        Value::literal("2026-03-01T12:00:00+02:00", Some(vocab::XSD_DATETIME), None),
    )]);
    let r = t.w("MATCH (n {`@id`: 'v:m'}) SET n.at = datetime('2026-03-01T10:00:00Z')");
    let rep = r.report.unwrap();
    assert_eq!(rep.superseded.len(), 1);
    assert_eq!(
        t.one("MATCH (n {`@id`: 'v:m'}) RETURN n.at")
            .to_json()
            .to_string(),
        "\"2026-03-01T10:00:00.000Z\""
    );
}

// "Upsert creates when absent" / "Upsert returns existing subject" / "Existing subject without the label gains it"
#[test]
fn merge_with_unique_key() {
    let t = T::new();
    t.assert(&[(
        v("email"),
        Value::iri("urn:tiramemsu:sys:unique"),
        Value::Bool(true),
    )]);
    let r = t.w("MERGE (n:Person {email: 'a@x'}) ON CREATE SET n.created = true RETURN n");
    let id1 = eid_of(&r.rows[0][0]);
    assert!(t
        .live()
        .iter()
        .any(|(_, p, o)| p == "created" && o == "true"));
    let r = t.w("MERGE (n:Person {email: 'a@x'}) ON MATCH SET n.seen = true RETURN n");
    assert_eq!(eid_of(&r.rows[0][0]), id1);
    assert!(t.live().iter().any(|(_, p, o)| p == "seen" && o == "true"));
    let rep = r.report.unwrap();
    assert!(!rep.existing.is_empty());
    assert_eq!(t.one("MATCH (n:Person) RETURN count(n)"), i(1));
    t.assert(&[(v("alice"), v("email"), sv("al@x"))]);
    let r = t.w("MERGE (n:Person {email: 'al@x'}) RETURN n");
    assert_eq!(short(&r.rows[0][0]), "alice");
    assert!(live_has(
        &t,
        "alice",
        "http://www.w3.org/1999/02/22-rdf-syntax-ns#type",
        "Person"
    ));
}

// "Merge node without unique key" / "Merge relationship between bound nodes" / "Merge binds every existing match"
// / "Null in MERGE rejected"
#[test]
fn merge_by_pattern() {
    let t = T::new();
    t.w("MERGE (n:City {name: 'Lviv'})");
    t.w("MERGE (n:City {name: 'Lviv'})");
    assert_eq!(t.one("MATCH (n:City {name: 'Lviv'}) RETURN count(n)"), i(1));
    t.w("CREATE (:P {name:'Alice'}), (:P {name:'Bob'})");
    let q = "MATCH (a {name:'Alice'}), (b {name:'Bob'}) MERGE (a)-[r:knows]->(b) RETURN r";
    let r1 = t.w(q);
    let r2 = t.w(q);
    assert_eq!(eid_of(&r1.rows[0][0]), eid_of(&r2.rows[0][0]));
    assert_eq!(t.one("MATCH ()-[r:knows]->() RETURN count(r)"), i(1));
    t.w("CREATE (:Tag {name: 'x'}), (:Tag {name: 'x'})");
    let r = t.w("MERGE (t:Tag {name: 'x'}) RETURN count(t) AS c");
    assert_eq!(r.rows, vec![vec![i(2)]]);
    let e =
        t.db.cypher_write(
            TxOptions::default(),
            "MERGE (n:City {name: $nm})",
            &params(&[("nm", null())]),
        )
        .unwrap_err();
    assert!(matches!(e, Error::Eval { .. }), "{e:?}");
}

// "Concurrent merges do not duplicate"
#[test]
fn concurrent_merges_do_not_duplicate() {
    let t = T::new();
    std::thread::scope(|sc| {
        for _ in 0..2 {
            sc.spawn(|| {
                t.w("MERGE (n:City {name: 'Kyiv'})");
            });
        }
    });
    assert_eq!(t.one("MATCH (n:City {name: 'Kyiv'}) RETURN count(n)"), i(1));
}
