//! Golden IR for the pattern lowering (tasks 4.1–4.4).
mod common;
use common::*;

// sparql-query "Supported query forms": SELECT lowers with the SPARQL flags
#[test]
fn select_carries_the_sparql_flags() {
    insta::assert_snapshot!(ir("SELECT ?c WHERE { v:alice v:worksAt ?c }"));
}

#[test]
fn ask_and_construct_forms() {
    insta::assert_snapshot!("ask", ir("ASK { v:alice v:worksAt v:acme }"));
    insta::assert_snapshot!(
        "construct",
        ir("CONSTRUCT { ?c v:employs ?p } WHERE { ?p v:worksAt ?c }")
    );
}

// sparql-query "Supported query forms": DESCRIBE is unsupported
#[test]
fn describe_is_unsupported() {
    unsupported("DESCRIBE v:alice", "DESCRIBE");
}

#[test]
fn join_and_left_join_with_condition() {
    insta::assert_snapshot!(
        "join",
        ir("SELECT ?p ?city WHERE { ?p v:worksAt ?c . ?c v:locatedIn ?city }")
    );
    insta::assert_snapshot!(
        "left_join_cond",
        ir("SELECT ?p ?age WHERE { ?p v:name ?n OPTIONAL { ?p v:age ?age FILTER(?age > 50) } }")
    );
}

#[test]
fn filter_union_extend_values() {
    insta::assert_snapshot!(
        "filter",
        ir("SELECT ?p WHERE { ?p v:age ?a FILTER(?a > 30) }")
    );
    insta::assert_snapshot!(
        "union_flat",
        ir("SELECT * WHERE { { ?p v:email ?e } UNION { ?p v:phone ?t } UNION { ?p v:fax ?f } }")
    );
    insta::assert_snapshot!(
        "extend",
        ir("SELECT ?y WHERE { v:alice v:age ?a BIND(2026 - ?a AS ?y) }")
    );
    insta::assert_snapshot!(
        "values_undef",
        ir("SELECT ?p ?a WHERE { ?p v:age ?a } VALUES (?p ?a) { (UNDEF 30) (v:bob 31) }")
    );
}

// sparql-query "BIND and VALUES": BIND over an in-scope variable is rejected
#[test]
fn bind_over_in_scope_variable_is_a_parse_error() {
    match err("SELECT ?a WHERE { v:alice v:age ?a BIND(1 AS ?a) }") {
        tm_core::Error::Parse { dialect, .. } => assert_eq!(dialect, tm_core::Dialect::Sparql),
        other => panic!("{other:?}"),
    }
}

// task 4.3
#[test]
fn service_and_graph_lowering() {
    insta::assert_snapshot!(
        "service_scope",
        ir("SELECT ?a ?b WHERE { SERVICE <urn:tiramemsu:tm:asOf/150> { v:alice v:worksAt ?a } v:alice v:worksAt ?b }")
    );
    insta::assert_snapshot!(
        "service_silent_nested",
        ir("SELECT ?c WHERE { SERVICE SILENT <urn:tiramemsu:tm:asOf/150> { SERVICE <urn:tiramemsu:tm:validAt/2021-06-01> { v:alice v:worksAt ?c } } }")
    );
    unsupported(
        "SELECT * WHERE { SERVICE <http://dbpedia.org/sparql> { ?s ?p ?o } }",
        "SERVICE",
    );
    unsupported("SELECT * WHERE { SERVICE ?e { ?s ?p ?o } }", "SERVICE");
    unsupported(
        "SELECT * WHERE { GRAPH <http://ex/g> { ?s ?p ?o } }",
        "named graph",
    );
    unsupported(
        "SELECT * WHERE { SERVICE <urn:tiramemsu:tm:history> { GRAPH <http://ex/g> { ?s ?p ?o } } }",
        "named graph",
    );
    unsupported("SELECT * WHERE { GRAPH ?g { ?s ?p ?o } }", "GRAPH variable");
    match err("SELECT * WHERE { GRAPH <urn:tiramemsu:tm:asOf/150> { ?s ?p ?o } }") {
        tm_core::Error::Parse { msg, .. } => assert!(msg.contains("SERVICE")),
        other => panic!("{other:?}"),
    }
}

// task 4.4
#[test]
fn minus_and_exists() {
    insta::assert_snapshot!(
        "minus_shared",
        ir("SELECT ?p WHERE { ?p a v:Person MINUS { ?p v:banned true } }")
    );
    // no shared variable: MINUS removes nothing
    insta::assert_snapshot!(
        "minus_disjoint",
        ir("SELECT ?p WHERE { ?p a v:Person MINUS { ?x v:banned true } }")
    );
    insta::assert_snapshot!(
        "not_exists",
        ir("SELECT ?p WHERE { ?p a v:Person FILTER NOT EXISTS { ?p v:banned true } }")
    );
    insta::assert_snapshot!(
        "exists",
        ir("SELECT ?p WHERE { ?p a v:Person FILTER EXISTS { ?p v:worksAt ?c } }")
    );
}
