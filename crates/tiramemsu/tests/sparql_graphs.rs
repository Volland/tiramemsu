//! `named-graphs` through SPARQL: GRAPH, FROM, FROM NAMED, time scopes and the
//! query-side scenarios of the spec.
mod sparql_common;
use sparql_common::*;
use tiramemsu::*;

const P: &str = "urn:tiramemsu:tm:";
const IN_GRAPH: &str = "urn:tiramemsu:sys:inGraph";

fn g(n: &str) -> Value {
    Value::iri(format!("urn:g:{n}"))
}

/// `(v:a v:p v:b)` in `<g1>` and `<g2>`, `(v:c v:p v:d)` in `<g2>`, `(v:e v:p v:f)` in no graph.
fn store() -> T {
    let t = T::new();
    t.upd("INSERT DATA { GRAPH <urn:g:1> { v:a v:p v:b } GRAPH <urn:g:2> { v:a v:p v:b . v:c v:p v:d } }");
    t.upd("INSERT DATA { v:e v:p v:f }");
    t
}

fn subjects(t: &T, q: &str) -> Vec<Option<Value>> {
    t.col(q, "s")
}

// @lat: [[tests#Named Graphs#GRAPH Selects By Membership]]
#[test]
fn graph_selects_by_membership() {
    let t = store();
    // a constant graph
    assert_eq!(
        subjects(&t, "SELECT ?s WHERE { GRAPH <urn:g:1> { ?s v:p ?o } }"),
        some(&[v("a")])
    );
    // a variable graph: one row per membership
    let s = t.sel("SELECT ?g ?s WHERE { GRAPH ?g { ?s v:p ?o } } ORDER BY ?g ?s");
    assert_eq!(
        s.rows,
        vec![
            vec![Some(g("1")), Some(v("a"))],
            vec![Some(g("2")), Some(v("a"))],
            vec![Some(g("2")), Some(v("c"))],
        ]
    );
    let two = t.col(
        "SELECT ?g WHERE { GRAPH ?g { v:a v:p v:b } } ORDER BY ?g",
        "g",
    );
    assert_eq!(two, some(&[g("1"), g("2")]));
    // a bound graph variable restricts
    assert_eq!(
        subjects(
            &t,
            "SELECT ?s WHERE { VALUES ?g { <urn:g:2> } GRAPH ?g { ?s v:p ?o } } ORDER BY ?s"
        ),
        some(&[v("a"), v("c")])
    );
    // nested GRAPH replaces the outer graph
    assert_eq!(
        subjects(
            &t,
            "SELECT ?s WHERE { GRAPH <urn:g:1> { GRAPH <urn:g:2> { ?s v:p ?o } } } ORDER BY ?s"
        ),
        some(&[v("a"), v("c")])
    );
    // a graph that no statement is in matches nothing
    assert!(t
        .sel("SELECT * WHERE { GRAPH <urn:never:used> { ?s ?p ?o } }")
        .rows
        .is_empty());
    // a statement in no graph has no graph
    assert!(t
        .sel("SELECT ?g WHERE { GRAPH ?g { v:e v:p v:f } }")
        .rows
        .is_empty());
}

// @lat: [[tests#Named Graphs#Default Graph Is The Union]]
#[test]
fn default_graph_is_the_union() {
    let t = store();
    // every statement, and one row for a statement in two graphs
    assert_eq!(
        subjects(&t, "SELECT ?s WHERE { ?s v:p ?o } ORDER BY ?s"),
        some(&[v("a"), v("c"), v("e")])
    );
    // statements in no graph are selectable
    let q = format!(
        "SELECT ?s WHERE {{ ?e <urn:tiramemsu:sys:subject> ?s . ?e <urn:tiramemsu:sys:predicate> v:p \
         FILTER NOT EXISTS {{ ?e <{IN_GRAPH}> ?g }} }}"
    );
    assert_eq!(subjects(&t, &q), some(&[v("e")]));
    // a store that never used graphs is unchanged
    let plain = T::new();
    plain.assert(&[(v("a"), v("p"), v("b"))]);
    assert_eq!(
        subjects(&plain, "SELECT ?s WHERE { ?s v:p ?o }"),
        some(&[v("a")])
    );
}

fn add_then_remove(t: &T) {
    t.advance_to(4);
    t.upd("INSERT DATA { GRAPH <urn:g:1> { v:a v:p v:b } }"); // tx 5
    t.advance_to(8);
    t.upd("DELETE DATA { GRAPH <urn:g:1> { v:a v:p v:b } }"); // tx 9
}

// @lat: [[tests#Named Graphs#Dataset Clauses]]
#[test]
fn dataset_clauses() {
    let t = store();
    // FROM restricts the default graph (and is a union of the listed graphs)
    assert_eq!(
        subjects(&t, "SELECT ?s FROM <urn:g:1> WHERE { ?s v:p ?o }"),
        some(&[v("a")])
    );
    assert_eq!(
        subjects(
            &t,
            "SELECT ?s FROM <urn:g:1> FROM <urn:g:2> WHERE { ?s v:p ?o } ORDER BY ?s"
        ),
        some(&[v("a"), v("c")]),
        "a statement in both graphs appears once"
    );
    // FROM NAMED restricts GRAPH for variable and constant names
    assert_eq!(
        t.col(
            "SELECT ?g FROM NAMED <urn:g:1> WHERE { GRAPH ?g { ?s ?p ?o } }",
            "g"
        ),
        some(&[g("1")])
    );
    assert!(t
        .sel("SELECT ?s FROM NAMED <urn:g:1> WHERE { GRAPH <urn:g:2> { ?s ?p ?o } }")
        .rows
        .is_empty());
    assert_eq!(
        subjects(
            &t,
            "SELECT ?s FROM NAMED <urn:g:1> WHERE { GRAPH <urn:g:1> { ?s ?p ?o } }"
        ),
        some(&[v("a")])
    );
    // a time IRI beside a graph IRI
    let t = T::new();
    add_then_remove(&t);
    let at = |n: &str| {
        subjects(
            &t,
            &format!("SELECT ?s FROM <{P}asOf/{n}> FROM <urn:g:1> WHERE {{ ?s v:p ?o }}"),
        )
    };
    assert_eq!(at("7"), some(&[v("a")]));
    assert!(at("10").is_empty());
}

// @lat: [[tests#Named Graphs#Graphs Combine With Service Scopes]]
#[test]
fn graphs_combine_with_service_scopes() {
    let t = T::new();
    add_then_remove(&t);
    let q = |body: &str| subjects(&t, &format!("SELECT ?s WHERE {{ {body} }}"));
    assert_eq!(
        q(&format!(
            "SERVICE <{P}asOf/7> {{ GRAPH <urn:g:1> {{ ?s v:p ?o }} }}"
        )),
        some(&[v("a")])
    );
    assert!(q(&format!(
        "SERVICE <{P}asOf/10> {{ GRAPH <urn:g:1> {{ ?s v:p ?o }} }}"
    ))
    .is_empty());
    assert_eq!(
        q(&format!(
            "GRAPH <urn:g:1> {{ SERVICE <{P}asOf/7> {{ ?s v:p ?o }} }}"
        )),
        some(&[v("a")])
    );
    assert!(
        q("GRAPH <urn:g:1> { ?s v:p ?o }").is_empty(),
        "removed in tx 9"
    );
    // a time IRI as a GRAPH name still names SERVICE
    let (_, msg) = assert_parse(t.err(&format!(
        "SELECT * WHERE {{ GRAPH <{P}asOf/7> {{ ?s ?p ?o }} }}"
    )));
    assert!(msg.contains("SERVICE"), "{msg}");
}

// @lat: [[tests#Named Graphs#Graph Metadata Is Ordinary Triples]]
#[test]
fn graph_metadata_is_ordinary_triples() {
    let t = T::new();
    t.upd("INSERT DATA { v:session12 v:startedBy v:agent7 ; v:startedAt \"2026-09-30T09:00:00Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime> }");
    let s = t.sel("SELECT ?g ?a WHERE { ?g v:startedBy ?a }");
    assert_eq!(s.rows, vec![vec![Some(v("session12")), Some(v("agent7"))]]);
    // metadata about a graph is not membership
    t.upd("INSERT DATA { GRAPH v:session12 { v:alice v:name \"Alice\" } }");
    assert_eq!(
        subjects(&t, "SELECT ?s WHERE { GRAPH v:session12 { ?s ?p ?o } }"),
        some(&[v("alice")])
    );
    assert_eq!(
        t.col("SELECT ?s WHERE { ?s v:startedBy ?a }", "s"),
        some(&[v("session12")])
    );
}

// @lat: [[tests#Named Graphs#Membership Carries Layers]]
#[test]
fn membership_carries_layers() {
    let t = T::new();
    t.upd("INSERT DATA { GRAPH <urn:g:1> { v:alice v:worksAt v:acme } }");
    t.upd(&format!(
        "INSERT {{ ?m v:addedBy v:agent7 }} WHERE {{ ?e <{IN_GRAPH}> <urn:g:1> ~ ?m }}"
    ));
    let who = t.col(
        &format!("SELECT ?who WHERE {{ ?e <{IN_GRAPH}> <urn:g:1> ~ ?m {{| v:addedBy ?who |}} }}"),
        "who",
    );
    assert_eq!(who, some(&[v("agent7")]));
    // an annotation of a statement inside GRAPH: the statement follows the graph,
    // its annotation triples read the default view
    t.upd("INSERT DATA { v:alice v:worksAt v:acme {| v:confidence 0.9 |} }");
    let c = t.col(
        "SELECT ?c WHERE { GRAPH <urn:g:1> { v:alice v:worksAt v:acme {| v:confidence ?c |} } }",
        "c",
    );
    assert_eq!(c.len(), 1);
    let none = t.sel(
        "SELECT ?c WHERE { GRAPH <urn:g:9> { v:alice v:worksAt v:acme {| v:confidence ?c |} } }",
    );
    assert!(none.rows.is_empty());
}

// @lat: [[tests#Named Graphs#Duplicates Follow Memberships]]
#[test]
fn duplicates_follow_memberships() {
    let t = store();
    // once in the default graph, once per membership under GRAPH ?g
    assert_eq!(
        t.sel("SELECT ?s WHERE { v:a v:p ?o . BIND(v:a AS ?s) }")
            .rows
            .len(),
        1
    );
    assert_eq!(
        t.sel("SELECT ?g WHERE { GRAPH ?g { v:a v:p v:b } }")
            .rows
            .len(),
        2
    );
    // the same with a predicate that holds parallel eids (`pred_multi`)
    t.tx(|tx| {
        let a = tx.create(v("x"), v("q"), v("y"), Valid::ALWAYS)?;
        tx.create(v("x"), v("q"), v("y"), Valid::ALWAYS)?;
        tx.add_to_graph(a, g("1"), AssertOpts::default())?;
        tx.add_to_graph(a, g("2"), AssertOpts::default())?;
        Ok(())
    });
    assert_eq!(
        t.sel("SELECT ?o WHERE { v:x v:q ?o }").rows.len(),
        1,
        "set semantics"
    );
    assert_eq!(
        t.sel("SELECT ?g WHERE { GRAPH ?g { v:x v:q v:y } }")
            .rows
            .len(),
        2
    );
    assert_eq!(
        t.sel("SELECT ?o FROM <urn:g:1> FROM <urn:g:2> WHERE { v:x v:q ?o }")
            .rows
            .len(),
        1
    );
}

// @lat: [[tests#Named Graphs#Membership Is Bitemporal In SPARQL]]
#[test]
fn membership_is_bitemporal_in_sparql() {
    let t = T::new();
    add_then_remove(&t);
    let ask = |body: &str| match t.db.now().sparql(body).unwrap() {
        SparqlResult::Boolean(b) => b,
        other => panic!("{other:?}"),
    };
    let q = |tm: &str| format!("ASK FROM <{P}{tm}> {{ GRAPH <urn:g:1> {{ v:a v:p v:b }} }}");
    assert!(ask(&q("asOf/7")));
    assert!(!ask(&q("asOf/9")));
    // history lists the removed membership
    let h = t.col(
        &format!("SELECT ?g FROM <{P}history> WHERE {{ GRAPH ?g {{ v:a v:p v:b }} }}"),
        "g",
    );
    assert_eq!(h, some(&[g("1")]));
    // valid time of a membership
    let t2 = T::new();
    t2.tx(|tx| {
        let e = tx.assert(v("a"), v("p"), v("b"), Valid::ALWAYS)?.eid();
        let day = |d: &str| value::parse_date(d).unwrap() * 86_400_000;
        tx.add_to_graph(
            e,
            g("1"),
            AssertOpts {
                valid: Valid::between(day("2025-01-01"), day("2025-07-01")),
                ..Default::default()
            },
        )?;
        Ok(())
    });
    let ask2 = |tm: &str| match t2
        .db
        .now()
        .sparql(&format!(
            "ASK FROM <{P}validAt/{tm}> {{ GRAPH <urn:g:1> {{ v:a v:p v:b }} }}"
        ))
        .unwrap()
    {
        SparqlResult::Boolean(b) => b,
        other => panic!("{other:?}"),
    };
    assert!(!ask2("2025-08-01"));
    assert!(ask2("2025-03-01"));
    // a plain delete retracts the statement and its memberships together
    let t3 = store();
    t3.upd("DELETE DATA { v:a v:p v:b }");
    assert!(t3
        .sel("SELECT ?g WHERE { GRAPH ?g { v:a v:p v:b } }")
        .rows
        .is_empty());
    let rows = t3.db.now().triples(None, None, None).unwrap();
    assert!(rows
        .iter()
        .all(|r| t3.db.now().decode(r.p).unwrap() != Value::iri(IN_GRAPH)
            || r.o != t3.db.now().encode(&g("1")).unwrap().unwrap()));
}

// @lat: [[tests#Named Graphs#Property Paths Inside Graphs Are Rejected]]
#[test]
fn property_paths_inside_graphs_are_rejected() {
    let t = store();
    let before = t.last_t();
    assert_unsupported(
        t.err("SELECT ?x WHERE { GRAPH <urn:g:1> { v:a v:knows+ ?x } }"),
        "named graph path",
    );
    assert_unsupported(
        t.err("SELECT ?x FROM <urn:g:1> WHERE { v:a v:knows+ ?x }"),
        "named graph path",
    );
    assert_unsupported(
        t.err("INSERT { v:z v:k ?x } WHERE { GRAPH <urn:g:1> { v:a v:knows+ ?x } }"),
        "named graph path",
    );
    // outside a graph the path engine still works
    assert!(t.sel("SELECT ?x WHERE { v:a v:knows+ ?x }").rows.is_empty());
    assert_eq!(t.last_t(), before);
}

// @lat: [[tests#Named Graphs#Graph Names Are Rejected When Invalid]]
#[test]
fn invalid_graph_names_are_rejected() {
    let t = store();
    let before = t.last_t();
    for q in [
        "SELECT * WHERE { GRAPH <urn:tiramemsu:stmt:1> { ?s ?p ?o } }",
        "SELECT * FROM <urn:tiramemsu:tx:1> WHERE { ?s ?p ?o }",
        "INSERT DATA { GRAPH <urn:tiramemsu:stmt:1> { v:c v:p v:d } }",
        "CREATE GRAPH <urn:tiramemsu:stmt:2>",
    ] {
        assert!(matches!(t.err(q), Error::InvalidGraphName { .. }), "{q}");
    }
    // a graph variable bound to a literal fails at run time and writes nothing
    let e = t.err("INSERT { GRAPH ?o { v:z v:p v:z } } WHERE { VALUES ?o { \"lit\" } }");
    assert!(matches!(e, Error::InvalidGraphName { .. }), "{e:?}");
    assert_eq!(t.last_t(), before);
}
