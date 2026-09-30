//! `cypher-dual-view`: statements as relationships and `:Statement` nodes.
mod cypher_common;
use cypher_common::*;
use tiramemsu::*;

/// e1 = (alice WORKS_AT acme), belief9 typed Belief supported by e1, (e1 confidence 0.8).
fn fixture() -> (T, Eid) {
    let t = T::new();
    let mut e1 = None;
    t.tx(|tx| {
        tx.assert(v("alice"), rdf_type(), v("Person"), Valid::ALWAYS)?;
        let e = tx
            .assert(v("alice"), v("WORKS_AT"), v("acme"), Valid::ALWAYS)?
            .eid();
        tx.assert(v("belief9"), rdf_type(), v("Belief"), Valid::ALWAYS)?;
        tx.assert(v("belief9"), v("SUPPORTED_BY"), e, Valid::ALWAYS)?;
        tx.assert(e, v("confidence"), Value::Double(0.8), Valid::ALWAYS)?;
        e1 = Some(e);
        Ok(())
    });
    (t, e1.unwrap())
}

// dual-view "Belief supporting a relationship in the same MATCH"
#[test]
fn belief_supporting_a_relationship_in_the_same_match() {
    let (t, _) = fixture();
    let r = t.q(
        "MATCH (a)-[r:WORKS_AT]->(c), (b:Belief)-[:SUPPORTED_BY]->(r) RETURN a, c, r.confidence, b",
    );
    assert_eq!(r.rows.len(), 1);
    let row = &r.rows[0];
    assert_eq!(short(&row[0]), "alice");
    assert_eq!(short(&row[1]), "acme");
    assert_eq!(row[2], f(0.8));
    assert_eq!(short(&row[3]), "belief9");
}

// "Relationship variable reused in a later clause" / "Statement as subject of a relationship" / "Node-position use is not a traversal"
#[test]
fn relationship_variable_in_later_clauses() {
    let (t, e1) = fixture();
    let r = t.q("MATCH (a)-[r:WORKS_AT]->(c) WITH r MATCH (b)-[:SUPPORTED_BY]->(r) RETURN b");
    assert_eq!(short(&r.rows[0][0]), "belief9");
    t.assert(&[(Value::Stmt(e1), v("assertedBy"), v("agent7"))]);
    let r = t.q("MATCH ()-[r:WORKS_AT]->() MATCH (r)-[:assertedBy]->(who) RETURN who");
    assert_eq!(short(&r.rows[0][0]), "agent7");
    assert_eq!(
        t.one("MATCH (a)-[r:WORKS_AT]->(c), (x)-[s]->(r) RETURN count(*) AS n"),
        i(2 - 1)
    );
}

// "Unbound node variable binds to a statement"
#[test]
fn unbound_node_variable_binds_to_a_statement() {
    let (t, e1) = fixture();
    let r =
        t.q("MATCH (b:Belief)-[:SUPPORTED_BY]->(x) RETURN x, labels(x) AS l, x.confidence AS conf");
    let CypherValue::Node(n) = &r.rows[0][0] else {
        panic!("{:?}", r.rows[0][0])
    };
    assert_eq!(n.element_id, format!("urn:tiramemsu:stmt:{}", e1.n()));
    let CypherValue::List(l) = &r.rows[0][1] else {
        panic!()
    };
    assert!(l.contains(&s("Statement")));
    assert_eq!(r.rows[0][2], f(0.8));
}

// "Enumerate statements" / "Typed statement" / "Statement nodes are not plain nodes"
#[test]
fn statement_label() {
    let t = T::new();
    t.assert(&[
        (v("alice"), v("name"), sv("Alice")),
        (v("alice"), v("knows"), v("bob")),
    ]);
    assert_eq!(t.one("MATCH (s:Statement) RETURN count(s) AS n"), i(2));
    let e1 =
        t.db.now()
            .triples(None, None, None)
            .unwrap()
            .into_iter()
            .find(|x| t.db.now().decode(x.p).unwrap() == v("knows"))
            .unwrap()
            .eid;
    t.assert(&[
        (Value::Stmt(e1), rdf_type(), v("Claim")),
        (Value::Stmt(e1), v("confidence"), Value::Double(0.8)),
    ]);
    let r = t.q("MATCH (s:Statement:Claim) RETURN labels(s) AS l");
    assert_eq!(r.rows[0][0], list(vec![s("Statement"), s("Claim")]));
    assert_eq!(t.one("MATCH (n) RETURN count(n) AS c"), i(2));
}

// "Functions on the relationship form" / "Functions on the node form" / "Property statement end is a literal"
#[test]
fn start_end_type_on_either_form() {
    let (t, _) = fixture();
    let r = t.q("MATCH (a)-[r:WORKS_AT]->(c) RETURN startNode(r) = a AS s, endNode(r) = c AS e, type(r) AS t");
    assert_eq!(r.rows[0], vec![b(true), b(true), s("WORKS_AT")]);
    let r = t.q(
        "MATCH (:Belief)-[:SUPPORTED_BY]->(x) RETURN elementId(startNode(x)) AS s, type(x) AS t",
    );
    assert_eq!(r.rows[0], vec![s("urn:tiramemsu:v:alice"), s("WORKS_AT")]);
    let t2 = T::new();
    t2.assert(&[(v("alice"), v("name"), sv("Alice"))]);
    assert_eq!(
        t2.one("MATCH (s:Statement) WHERE type(s) = 'name' RETURN endNode(s) AS v"),
        s("Alice")
    );
}

// "Two-level layer"
#[test]
fn two_level_layer() {
    let (t, _) = fixture();
    let e7 =
        t.db.now()
            .triples(None, None, None)
            .unwrap()
            .into_iter()
            .find(|x| t.db.now().decode(x.p).unwrap() == v("SUPPORTED_BY"))
            .unwrap()
            .eid;
    t.assert(&[(Value::Stmt(e7), v("method"), sv("llm-extraction"))]);
    assert_eq!(
        t.one("MATCH ()-[r:WORKS_AT]->(), ()-[s:SUPPORTED_BY]->(r) RETURN s.method AS m"),
        s("llm-extraction")
    );
}

// "Same eid, two forms"
#[test]
fn same_eid_two_forms() {
    let (t, _) = fixture();
    let r = t.q(
        "MATCH ()-[r:WORKS_AT]->() MATCH (:Belief)-[:SUPPORTED_BY]->(x) \
         RETURN r, x, r = x AS same, elementId(r) = elementId(x) AS sameId",
    );
    assert!(matches!(r.rows[0][0], CypherValue::Relationship(_)));
    let CypherValue::Node(n) = &r.rows[0][1] else {
        panic!()
    };
    assert!(n.labels.contains(&"Statement".to_string()));
    assert_eq!(r.rows[0][2], b(true));
    assert_eq!(r.rows[0][3], b(true));
}

// "Unlabelled relationship count unaffected by layers" / "Node variable in relationship position rejected"
#[test]
fn standard_queries_keep_their_meaning() {
    let t = T::new();
    let mut e1 = None;
    t.tx(|tx| {
        e1 = Some(
            tx.assert(v("alice"), v("knows"), v("bob"), Valid::ALWAYS)?
                .eid(),
        );
        tx.assert(v("carol"), v("SUPPORTED_BY"), e1.unwrap(), Valid::ALWAYS)?;
        Ok(())
    });
    assert_eq!(
        t.one("MATCH (a)-[r]->(b) WHERE NOT b:Statement RETURN count(r) AS c"),
        i(1)
    );
    assert_eq!(t.one("MATCH (a)-[r]->(b) RETURN count(r)"), i(2));
    assert!(matches!(
        t.qerr("MATCH (:Belief)-[:SUPPORTED_BY]->(x) MATCH ()-[x]->() RETURN x"),
        Error::Parse { .. }
    ));
}

// "Attach a belief to a relationship" / "Annotate a relationship" / "Delete a statement through its node form"
#[test]
fn writing_layers_through_the_dual_view() {
    let (t, e1) = fixture();
    t.w("MATCH (a {`@id`: 'v:alice'})-[r:WORKS_AT]->() CREATE (b:Belief {text: 'from CV'})-[:SUPPORTED_BY]->(r) RETURN b");
    let n =
        t.db.now()
            .triples(
                None,
                None,
                Some(t.db.now().encode(&Value::Stmt(e1)).unwrap().unwrap()),
            )
            .unwrap();
    assert_eq!(n.len(), 2);
    t.w("MATCH ()-[r:WORKS_AT]->() SET r.confidence = 0.95");
    assert_eq!(
        t.one("MATCH ()-[r:WORKS_AT]->() RETURN r.confidence"),
        f(0.95)
    );
    let rep = t
        .w("MATCH (:Belief)-[:SUPPORTED_BY]->(x) DELETE x")
        .report
        .unwrap();
    assert!(rep.retracted.iter().any(|(e, _)| *e == e1));
    assert!(rep.retracted.len() >= 3, "{:?}", rep.retracted);
    assert_eq!(t.q("MATCH ()-[r:WORKS_AT]->() RETURN r").rows.len(), 0);
}
