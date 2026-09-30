//! Golden IR for aggregates, solution modifiers, subqueries and paths (4.5–4.7).
mod common;
use common::*;

// task 4.5
#[test]
fn aggregates_and_having() {
    insta::assert_snapshot!(
        "count_group_having",
        ir("SELECT ?c (COUNT(?p) AS ?n) WHERE { ?p v:worksAt ?c } GROUP BY ?c HAVING (COUNT(?p) > 1) ORDER BY ?c")
    );
    insta::assert_snapshot!(
        "implicit_group",
        ir("SELECT (COUNT(*) AS ?n) (SUM(?a) AS ?s) (AVG(?a) AS ?m) WHERE { ?p v:age ?a }")
    );
    insta::assert_snapshot!(
        "distinct_and_concat",
        ir("SELECT (COUNT(DISTINCT ?c) AS ?n) (MIN(?a) AS ?lo) (MAX(?a) AS ?hi) (SAMPLE(?a) AS ?x) (GROUP_CONCAT(?c; SEPARATOR=\"|\") AS ?all) WHERE { ?p v:worksAt ?c . ?p v:age ?a }")
    );
    insta::assert_snapshot!(
        "group_concat_default_separator",
        ir("SELECT (GROUP_CONCAT(?c) AS ?all) WHERE { ?p v:worksAt ?c }")
    );
}

// sparql-query "Aggregates and grouping": custom aggregates fail
#[test]
fn custom_aggregate_is_unsupported() {
    let q = "PREFIX ex: <http://example.org/> SELECT (ex:agg(?a) AS ?x) WHERE { ?p v:age ?a }";
    // spargebra only parses custom aggregates it was told about, so a plain call
    // is a function, rejected by the function whitelist
    match err(q) {
        tm_core::Error::Unsupported { feature } => assert_eq!(feature, "http://example.org/agg"),
        other => panic!("{other:?}"),
    }
    assert_eq!(tm_sparql::error::CUSTOM_AGGREGATE, "custom aggregate");
}

// task 4.6
#[test]
fn solution_modifiers() {
    insta::assert_snapshot!(
        "order_limit_offset",
        ir("SELECT ?n WHERE { ?p v:name ?n } ORDER BY DESC(?n) LIMIT 2 OFFSET 1")
    );
    insta::assert_snapshot!(
        "order_by_hidden_variable",
        ir("SELECT ?p WHERE { ?p v:age ?a } ORDER BY ?a")
    );
    insta::assert_snapshot!(
        "distinct_order_limit",
        ir("SELECT DISTINCT ?s WHERE { ?s v:knows ?o } ORDER BY ?s LIMIT 5")
    );
    insta::assert_snapshot!("reduced", ir("SELECT REDUCED ?s WHERE { ?s v:knows ?o }"));
    insta::assert_snapshot!(
        "limit_only",
        ir("SELECT ?s WHERE { ?s v:knows ?o } LIMIT 3")
    );
}

// sparql-query "Solution modifiers": DISTINCT with a non-projected sort key
#[test]
fn distinct_with_non_projected_key_is_unsupported() {
    unsupported(
        "SELECT DISTINCT ?s WHERE { ?s v:knows ?o } ORDER BY ?o",
        "ORDER BY non-projected variable with DISTINCT",
    );
}

#[test]
fn subqueries() {
    insta::assert_snapshot!(
        "top_n_subquery",
        ir("SELECT ?p ?n WHERE { { SELECT ?p WHERE { ?p v:age ?a } ORDER BY DESC(?a) LIMIT 2 } OPTIONAL { ?p v:name ?n } }")
    );
    insta::assert_snapshot!(
        "inner_variables_hidden",
        ir("SELECT ?a WHERE { { SELECT ?p WHERE { ?p v:age ?a } } }")
    );
}

// task 4.7
#[test]
fn interim_property_paths() {
    insta::assert_snapshot!(
        "inverse_path",
        ir("SELECT ?p WHERE { v:acme ^v:worksAt ?p }")
    );
    for q in [
        "SELECT ?x WHERE { v:alice v:knows+ ?x }",
        "SELECT ?x WHERE { v:alice v:knows* ?x }",
        "SELECT ?x WHERE { v:alice v:knows? ?x }",
        "SELECT ?city WHERE { v:alice v:worksAt/v:locatedIn ?city }",
        "SELECT ?x WHERE { v:alice v:a|v:b ?x }",
        "SELECT ?x WHERE { v:alice !v:a ?x }",
        "SELECT ?x WHERE { v:alice ^(v:a/v:b) ?x }",
    ] {
        unsupported(q, "property path");
    }
}
