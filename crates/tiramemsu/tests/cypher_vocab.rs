//! `vocabulary-mapping`: names, CURIEs, rendering, implicit labels, sys hiding,
//! node identity and vocabulary configuration.
#![cfg(all(feature = "sparql", feature = "cypher"))]
mod cypher_common;
use cypher_common::*;
use tiramemsu::*;

const TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

// "Case is preserved"
#[test]
fn case_is_preserved() {
    let t = T::new();
    t.assert(&[(v("alice"), rdf_type(), v("Person"))]);
    assert_eq!(t.q("MATCH (n:person) RETURN n").rows.len(), 0);
    assert_eq!(short(&t.q("MATCH (n:Person) RETURN n").rows[0][0]), "alice");
}

// "Backticked name with a space"
#[test]
fn backticked_name_with_a_space() {
    let t = T::new();
    t.w("CREATE (n:`Top Customer`)-[:x]->(:Y)");
    assert!(t
        .live()
        .iter()
        .any(|(_, p, o)| p == TYPE && o == "Top%20Customer"));
    assert_eq!(
        t.one("MATCH (n:`Top Customer`) RETURN labels(n)"),
        list(vec![s("Top Customer")])
    );
}

// "User prefix" / "v prefix equals @vocab" / "Full IRI label" / "Invalid IRI" / "Unknown label"
#[test]
fn curies_full_iris_and_unknown_names() {
    let t = T::new();
    t.tx(|tx| tx.set_prefix("schema", "https://schema.org/"));
    t.assert(&[
        (
            v("alice"),
            Value::iri("https://schema.org/name"),
            sv("Alice"),
        ),
        (
            v("acme"),
            rdf_type(),
            Value::iri("https://schema.org/Organization"),
        ),
        (v("alice"), rdf_type(), v("Person")),
    ]);
    assert_eq!(
        short(
            &t.q("MATCH (n) WHERE n.`schema:name` = 'Alice' RETURN n")
                .rows[0][0]
        ),
        "alice"
    );
    assert_eq!(
        t.q("MATCH (n:`v:Person`) RETURN n").rows,
        t.q("MATCH (n:Person) RETURN n").rows
    );
    assert_eq!(
        short(
            &t.q("MATCH (n:`https://schema.org/Organization`) RETURN n")
                .rows[0][0]
        ),
        "acme"
    );
    assert!(matches!(
        t.qerr("MATCH (n:`1bad:<>`) RETURN n"),
        Error::Parse { .. }
    ));
    assert_eq!(t.one("MATCH (n:NeverUsed) RETURN count(n) AS c"), i(0));
}

// "Local name" / "CURIE and full IRI" / "Local name containing a colon is not shortened" / "Round trip of rendered labels"
#[test]
fn rendering_and_round_trip() {
    let t = T::new();
    t.tx(|tx| tx.set_prefix("schema", "https://schema.org/"));
    t.assert(&[
        (v("n1"), rdf_type(), v("Person")),
        (v("n1"), rdf_type(), Value::iri("https://schema.org/Person")),
        (v("n1"), rdf_type(), Value::iri("http://example.org/X")),
        (v("n1"), Value::iri("urn:tiramemsu:v:schema:name"), sv("k")),
    ]);
    let p = params(&[("n", node_ref(v("n1")))]);
    let r = t.qp(
        "MATCH (n) WHERE n = $n RETURN labels(n) AS l, keys(n) AS k",
        &p,
    );
    assert_eq!(
        r.rows[0][0],
        list(vec![
            s("Person"),
            s("http://example.org/X"),
            s("schema:Person")
        ])
    );
    assert_eq!(r.rows[0][1], list(vec![s("v:schema:name")]));
    for l in ["Person", "http://example.org/X", "schema:Person"] {
        let r = t.qp(&format!("MATCH (m:`{l}`) WHERE m = $n RETURN m"), &p);
        assert_eq!(r.rows.len(), 1, "{l}");
    }
    assert_eq!(
        t.qp("MATCH (m) WHERE m = $n RETURN m.`v:schema:name`", &p)
            .rows[0][0],
        s("k")
    );
}

// "Query the predicate schema" / "User class named Statement"
#[test]
fn reserved_implicit_labels() {
    let t = T::new();
    t.assert(&[
        (
            v("email"),
            Value::iri("urn:tiramemsu:sys:unique"),
            Value::Bool(true),
        ),
        (
            v("age"),
            Value::iri("urn:tiramemsu:sys:cardinality"),
            Value::iri("urn:tiramemsu:sys:one"),
        ),
    ]);
    let r = t.q("MATCH (p:Predicate) RETURN p, p.`sys:unique` AS u, p.`sys:cardinality` AS card ORDER BY elementId(p)");
    assert_eq!(r.rows.len(), 2);
    assert_eq!(
        (short(&r.rows[0][0]), r.rows[0][1].clone()),
        ("age".into(), null())
    );
    assert_eq!(r.rows[1][1], b(true));
    let t2 = T::new();
    t2.assert(&[(v("x"), rdf_type(), v("Statement"))]);
    assert_eq!(
        short(&t2.q("MATCH (n:`v:Statement`) RETURN n").rows[0][0]),
        "x"
    );
    assert_eq!(
        t2.q("MATCH (n:Statement) RETURN n").rows.len(),
        1,
        "the statement (x rdf:type Statement) itself"
    );
    assert_ne!(short(&t2.q("MATCH (n:Statement) RETURN n").rows[0][0]), "x");
}

// "Hidden from keys" / "Explicit access to a supersede chain" / "Transaction metadata hidden"
#[test]
fn sys_namespace_is_hidden() {
    let t = T::new();
    t.assert(&[
        (
            v("email"),
            Value::iri("urn:tiramemsu:sys:unique"),
            Value::Bool(true),
        ),
        (
            v("email"),
            Value::iri("http://www.w3.org/2000/01/rdf-schema#label"),
            sv("e-mail"),
        ),
    ]);
    assert_eq!(
        t.one("MATCH (p {`@id`: 'v:email'}) RETURN keys(p) AS k"),
        list(vec![s("rdfs:label")])
    );
    t.assert(&[(v("alice"), v("title"), sv("Dr"))]);
    let old =
        t.db.now()
            .triples(
                Some(t.db.now().encode(&v("alice")).unwrap().unwrap()),
                None,
                None,
            )
            .unwrap()[0]
            .eid;
    t.tx(|tx| {
        tx.supersede(old, Patch::object(sv("Prof")))?;
        Ok(())
    });
    assert_eq!(
        t.q("MATCH (new)-[:`sys:supersedes`]->(old) RETURN elementId(new), elementId(old)")
            .rows
            .len(),
        1
    );
    let t2 = T::new();
    t2.tx(|tx| {
        tx.assert(v("alice"), v("name"), sv("A"), Valid::ALWAYS)?;
        tx.meta(Value::iri("urn:tiramemsu:sys:author"), v("agent7"))?;
        Ok(())
    });
    let ids: Vec<String> = t2
        .q("MATCH (n) RETURN n")
        .rows
        .iter()
        .map(|r| short(&r[0]))
        .collect();
    assert_eq!(ids, vec!["alice"]);
}

// "Match by IRI" / "Skolem round trip for anonymous nodes" / "Match by element id in WHERE" / "Invalid @id"
#[test]
fn node_identity() {
    let t = T::new();
    t.assert(&[(v("alice"), v("name"), sv("Alice"))]);
    assert_eq!(
        t.one("MATCH (n {`@id`: 'v:alice'}) RETURN elementId(n) AS id"),
        s("urn:tiramemsu:v:alice")
    );
    let r = t.w("CREATE (n:Person {name:'Bob'}) RETURN elementId(n) AS id");
    let CypherValue::String(id) = &r.rows[0][0] else {
        panic!()
    };
    assert_eq!(
        t.one(&format!("MATCH (m {{`@id`: '{id}'}}) RETURN m.name")),
        s("Bob")
    );
    let r = t.qp(
        "MATCH (n) WHERE elementId(n) = $id RETURN n",
        &params(&[("id", s(id))]),
    );
    assert_eq!(r.rows.len(), 1);
    assert!(matches!(
        t.qerr("MATCH (n {`@id`: 'not an iri'}) RETURN n"),
        Error::Eval { .. }
    ));
    // id() round trips as an integer
    let CypherValue::Integer(raw) = t.one("MATCH (n {`@id`: 'v:alice'}) RETURN id(n)") else {
        panic!()
    };
    assert_ne!(raw, 0);
    // statements, blank nodes and anonymous nodes round-trip through their skolem IRIs
    let e1 = {
        t.assert(&[(v("alice"), v("knows"), v("bob"))]);
        t.db.now()
            .triples(None, None, None)
            .unwrap()
            .into_iter()
            .find(|x| t.db.now().decode(x.p).unwrap() == v("knows"))
            .unwrap()
            .eid
    };
    let r = t.q(&format!(
        "MATCH (s {{`@id`: 'urn:tiramemsu:stmt:{}'}}) RETURN type(s)",
        e1.n()
    ));
    assert_eq!(r.rows[0][0], s("knows"));
}

// "Change @vocab" / "Historical query uses current vocabulary" / "Reserved prefix name"
#[test]
fn vocabulary_configuration() {
    let t = T::new();
    t.assert(&[(v("a"), rdf_type(), v("Person"))]);
    t.tx(|tx| tx.set_vocab("https://ex.org/"));
    assert_eq!(
        t.one("MATCH (n) RETURN labels(n)"),
        list(vec![s("urn:tiramemsu:v:Person")])
    );
    t.w("CREATE (m:Person)");
    assert!(t
        .live()
        .iter()
        .any(|(_, p, o)| p == TYPE && o == "https://ex.org/Person"));
    // replaced, not duplicated
    let vocab_stmts: Vec<_> = t
        .live()
        .into_iter()
        .filter(|(_, p, _)| p == "urn:tiramemsu:sys:vocab")
        .collect();
    assert_eq!(vocab_stmts.len(), 1);
    // historical query, current prefix table
    let t2 = T::new();
    t2.assert(&[(v("a"), Value::iri("https://schema.org/name"), sv("A"))]); // tx1
    t2.tx(|tx| tx.set_prefix("schema", "https://schema.org/")); // tx2
    let r = t2.q("USE AS OF 1 MATCH (n) WHERE n.`schema:name` IS NOT NULL RETURN n");
    assert_eq!(r.rows.len(), 1);
    // reserved prefix
    let before = t2.last_t();
    let e = t2
        .db
        .transact(TxOptions::default(), |tx| {
            tx.set_prefix("rdf", "https://evil.example/")
        })
        .unwrap_err();
    assert!(matches!(e, Error::ReservedNamespace(_)));
    assert_eq!(t2.last_t(), before);
    // redeclaring replaces
    t2.tx(|tx| tx.set_prefix("schema", "https://other.org/"));
    assert_eq!(
        t2.q("MATCH (n) WHERE n.`schema:name` IS NOT NULL RETURN n")
            .rows
            .len(),
        0
    );
}

// "Labels from SPARQL-written types" / "Cypher sees SPARQL data"
#[test]
fn shared_addressing_with_sparql() {
    let t = T::new();
    t.db.now()
        .sparql("PREFIX v: <urn:tiramemsu:v:> INSERT DATA { v:alice a v:Person, v:Agent . v:alice v:worksAt v:acme }")
        .unwrap();
    assert_eq!(
        t.one("MATCH (n {`@id`: 'v:alice'}) RETURN labels(n) AS l"),
        list(vec![s("Agent"), s("Person")])
    );
    let r = t.q("MATCH (a)-[:worksAt]->(c) RETURN elementId(a), elementId(c)");
    assert_eq!(
        r.rows[0],
        vec![s("urn:tiramemsu:v:alice"), s("urn:tiramemsu:v:acme")]
    );
}
