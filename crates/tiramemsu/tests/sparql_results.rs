//! `sparql-query`: result terms, JSON results and `CONSTRUCT` results, plus the
//! RDF 1.2 export of `sparql-rdf12-annotations`.
mod sparql_common;
use sparql_common::*;
use tiramemsu::*;

fn graph(t: &T, q: &str) -> Vec<RdfTriple> {
    match t.db.now().sparql(q).unwrap_or_else(|e| panic!("{q}: {e}")) {
        SparqlResult::Graph(g) => g,
        other => panic!("not a graph: {other:?}"),
    }
}

fn iri(s: &str) -> RdfTerm {
    RdfTerm::Iri(s.to_string())
}

// sparql-query "Result terms": Eid rendered as statement IRI
#[test]
fn eid_rendered_as_statement_iri() {
    let t = T::new();
    let rep = t.assert(&[(v("alice"), v("worksAt"), v("acme"))]);
    let e = rep.asserted[0];
    let r =
        t.db.now()
            .sparql("SELECT ?r WHERE { v:alice v:worksAt v:acme ~ ?r }")
            .unwrap();
    let json = r.write_sparql_json().unwrap();
    assert!(
        json.contains(&format!(
            "{{\"type\":\"uri\",\"value\":\"urn:tiramemsu:stmt:{}\"}}",
            e.n()
        )),
        "{json}"
    );
    assert_eq!(r.solutions().unwrap().get(0, "r"), Some(&Value::Stmt(e)));
}

// sparql-query "Result terms": Date-time rendering
#[test]
fn datetime_rendering() {
    let t = T::new();
    t.assert(&[
        (v("a"), v("at"), dt("2026-09-01T12:00:00.250Z")),
        (v("b"), v("at"), dt("2026-09-01T12:00:00.000Z")),
    ]);
    let r =
        t.db.now()
            .sparql("SELECT ?t WHERE { ?s v:at ?t } ORDER BY ?s")
            .unwrap();
    let json = r.write_sparql_json().unwrap();
    assert!(
        json.contains("\"value\":\"2026-09-01T12:00:00.250Z\""),
        "{json}"
    );
    assert!(
        json.contains("\"value\":\"2026-09-01T12:00:00Z\""),
        "{json}"
    );
}

// sparql-query "Result terms": Date-time rendering keeps the offset
#[test]
fn datetime_rendering_keeps_the_offset() {
    let t = T::new();
    t.assert(&[
        (v("a"), v("at"), dt("2026-09-01T14:00:00.000+02:00")),
        (v("b"), v("at"), dt("2026-09-01T12:00:00+00:00")),
        (v("c"), v("at"), dt("2026-09-01T12:00:00")),
    ]);
    let r =
        t.db.now()
            .sparql("SELECT ?t WHERE { ?s v:at ?t } ORDER BY ?s")
            .unwrap();
    let json = r.write_sparql_json().unwrap();
    for want in [
        "2026-09-01T14:00:00+02:00",
        "2026-09-01T12:00:00Z",
        "2026-09-01T12:00:00\"",
    ] {
        assert!(
            json.contains(&format!("\"value\":\"{want}")),
            "{want} in {json}"
        );
    }
}

// sparql-query "SPARQL JSON results": SELECT JSON document
#[test]
fn select_json_document() {
    let t = T::new();
    t.assert(&[(v("bob"), v("name"), s("Bob"))]);
    let r =
        t.db.now()
            .sparql("SELECT ?p ?age WHERE { ?p v:name ?n OPTIONAL { ?p v:age ?age } }")
            .unwrap();
    assert_eq!(
        r.write_sparql_json().unwrap(),
        r#"{"head":{"vars":["p","age"]},"results":{"bindings":[{"p":{"type":"uri","value":"urn:tiramemsu:v:bob"}}]}}"#
    );
}

// sparql-query "SPARQL JSON results": Typed literal JSON
#[test]
fn typed_literal_json() {
    let t = T::new();
    t.assert(&[(v("alice"), v("age"), int(41))]);
    let json =
        t.db.now()
            .sparql("SELECT ?a WHERE { v:alice v:age ?a }")
            .unwrap()
            .write_sparql_json()
            .unwrap();
    assert!(json.contains(
        r#"{"type":"literal","value":"41","datatype":"http://www.w3.org/2001/XMLSchema#integer"}"#
    ));
}

// sparql-query "SPARQL JSON results": ASK JSON document
#[test]
fn ask_json_document() {
    let t = T::new();
    t.assert(&[(v("a"), v("b"), v("c"))]);
    let r = t.db.now().sparql("ASK { v:a v:b v:c }").unwrap();
    assert_eq!(
        r.write_sparql_json().unwrap(),
        r#"{"head":{},"boolean":true}"#
    );
}

// sparql-query "SPARQL JSON results": row order is preserved with ORDER BY
#[test]
fn row_order_is_preserved() {
    let t = T::new();
    t.assert(&[
        (v("a"), v("rank"), int(3)),
        (v("b"), v("rank"), int(1)),
        (v("c"), v("rank"), int(2)),
    ]);
    let json =
        t.db.now()
            .sparql("SELECT ?s WHERE { ?s v:rank ?r } ORDER BY ?r")
            .unwrap()
            .write_sparql_json()
            .unwrap();
    let (b, c, a) = (
        json.find(":v:b").unwrap(),
        json.find(":v:c").unwrap(),
        json.find(":v:a").unwrap(),
    );
    assert!(b < c && c < a, "{json}");
}

// sparql-query "CONSTRUCT results": CONSTRUCT maps predicates
#[test]
fn construct_maps_predicates() {
    let t = T::new();
    t.assert(&[(v("alice"), v("worksAt"), v("acme"))]);
    let g = graph(
        &t,
        "CONSTRUCT { ?c v:employs ?p } WHERE { ?p v:worksAt ?c }",
    );
    assert_eq!(
        g,
        vec![RdfTriple {
            s: iri("urn:tiramemsu:v:acme"),
            p: iri("urn:tiramemsu:v:employs"),
            o: iri("urn:tiramemsu:v:alice"),
        }]
    );
    // CONSTRUCT WHERE
    let g = graph(&t, "CONSTRUCT WHERE { ?p v:worksAt ?c }");
    assert_eq!(g.len(), 1);
}

// sparql-query "CONSTRUCT results": Unbound template variable skipped
#[test]
fn unbound_template_variable_skipped() {
    let t = T::new();
    t.assert(&[(v("alice"), v("name"), s("Alice"))]);
    let g = graph(
        &t,
        "CONSTRUCT { ?p v:hasAge ?a } WHERE { ?p v:name ?n OPTIONAL { ?p v:age ?a } }",
    );
    assert!(g.is_empty());
}

// sparql-query "CONSTRUCT results": Fresh blank node per solution
#[test]
fn fresh_blank_node_per_solution() {
    let t = T::new();
    t.assert(&[
        (v("alice"), v("name"), s("Alice")),
        (v("bob"), v("name"), s("Bob")),
    ]);
    let g = graph(
        &t,
        "CONSTRUCT { ?p v:card [ v:name ?n ] } WHERE { ?p v:name ?n }",
    );
    assert_eq!(g.len(), 4);
    let cards: std::collections::BTreeSet<_> = g
        .iter()
        .filter_map(|t| match &t.o {
            RdfTerm::Blank(b) => Some(b.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(cards.len(), 2);
}

#[test]
fn duplicates_and_invalid_positions() {
    let t = T::new();
    t.assert(&[(v("a"), v("p"), v("x")), (v("b"), v("p"), v("x"))]);
    // both solutions produce the same triple once
    let g = graph(&t, "CONSTRUCT { v:k v:has ?o } WHERE { ?s v:p ?o }");
    assert_eq!(g.len(), 1);
    // a literal subject is skipped
    let g = graph(&t, "CONSTRUCT { \"lit\" v:has ?o } WHERE { ?s v:p ?o }");
    assert!(g.is_empty());
}

// sparql-rdf12-annotations "CONSTRUCT emits RDF 1.2 reification": Export an annotated fact
#[test]
fn export_an_annotated_fact() {
    let t = T::new();
    let mut e1 = None;
    t.tx(|tx| {
        let e = tx
            .assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?
            .eid();
        tx.assert(
            e,
            v("confidence"),
            Value::Decimal("0.8".into()),
            Valid::ALWAYS,
        )?;
        e1 = Some(e);
        Ok(())
    });
    let e1 = e1.unwrap();
    let q = "CONSTRUCT { ?s v:worksAt ?o ~ ?r {| v:confidence ?c |} } \
             WHERE { ?s v:worksAt ?o ~ ?r {| v:confidence ?c |} }";
    let g = graph(&t, q);
    let stmt = iri(&format!("urn:tiramemsu:stmt:{}", e1.n()));
    let base = RdfTriple {
        s: iri("urn:tiramemsu:v:alice"),
        p: iri("urn:tiramemsu:v:worksAt"),
        o: iri("urn:tiramemsu:v:acme"),
    };
    let mut want = vec![
        base.clone(),
        RdfTriple {
            s: stmt.clone(),
            p: iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies"),
            o: RdfTerm::Triple(Box::new(base)),
        },
        RdfTriple {
            s: stmt,
            p: iri("urn:tiramemsu:v:confidence"),
            o: RdfTerm::Literal {
                lex: "0.8".into(),
                datatype: Some("http://www.w3.org/2001/XMLSchema#decimal".into()),
                lang: None,
            },
        },
    ];
    let mut got = g.clone();
    got.sort();
    want.sort();
    assert_eq!(got, want);
    let nt = SparqlResult::Graph(g).write_ntriples().unwrap();
    assert!(
        nt.contains(
            "<<( <urn:tiramemsu:v:alice> <urn:tiramemsu:v:worksAt> <urn:tiramemsu:v:acme> )>>"
        ),
        "{nt}"
    );
}
