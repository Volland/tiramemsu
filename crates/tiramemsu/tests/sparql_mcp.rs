//! The MCP-facing surface (task 11.2): `View::sparql` plus `write_sparql_json` on
//! the current and as-of views produce valid SPARQL Query Results JSON.
mod sparql_common;
use sparesults::{QueryResultsFormat, QueryResultsParser, ReaderQueryResultsParserOutput};
use sparql_common::*;
use tiramemsu::*;

fn parse(doc: &str) -> ReaderQueryResultsParserOutput<&[u8]> {
    QueryResultsParser::from_format(QueryResultsFormat::Json)
        .for_reader(doc.as_bytes())
        .unwrap_or_else(|e| panic!("invalid SPARQL JSON {doc}: {e}"))
}

#[test]
fn sparql_json_is_valid_on_the_current_and_as_of_views() {
    let t = T::new();
    t.assert(&[
        (v("alice"), v("worksAt"), v("acme")),
        (v("alice"), v("age"), int(41)),
    ]); // tx 1
    t.retract(&v("alice"), &v("worksAt"), &v("acme")); // tx 2
    let q = "SELECT ?p ?o WHERE { v:alice ?p ?o } ORDER BY ?p";
    let now = t.db.now().sparql(q).unwrap().write_sparql_json().unwrap();
    let past =
        t.db.as_of(TimeRef::Tx(1))
            .sparql(q)
            .unwrap()
            .write_sparql_json()
            .unwrap();
    let count = |doc: &str| match parse(doc) {
        ReaderQueryResultsParserOutput::Solutions(s) => s.count(),
        _ => panic!("not solutions"),
    };
    assert_eq!((count(&now), count(&past)), (1, 2));
    // ASK documents are valid too, on both views
    for (view, want) in [(t.db.now(), false), (t.db.as_of(TimeRef::Tx(1)), true)] {
        let doc = view
            .sparql("ASK { v:alice v:worksAt v:acme }")
            .unwrap()
            .write_sparql_json()
            .unwrap();
        assert!(matches!(parse(&doc), ReaderQueryResultsParserOutput::Boolean(b) if b == want));
    }
    // a graph result has N-Triples, not JSON
    let g =
        t.db.now()
            .sparql("CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }")
            .unwrap();
    assert!(g.write_sparql_json().is_err());
    assert!(g.write_ntriples().unwrap().ends_with(" .\n"));
}
