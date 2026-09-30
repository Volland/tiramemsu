//! Golden IR for expressions and the function whitelist (5.1–5.4).
mod common;
use common::*;

#[test]
fn operators_in_bound_if_coalesce_same_term() {
    insta::assert_snapshot!(
        "operators",
        ir("SELECT ?p WHERE { ?p v:age ?a FILTER((?a > 1 && ?a < 9) || !(?a = 5) || ?a IN (1, 2) || ?a NOT IN (3)) }")
    );
    insta::assert_snapshot!(
        "bound_if_coalesce",
        ir("SELECT (IF(BOUND(?b), 1, 2) AS ?x) (COALESCE(?b, ?a, 0) AS ?y) (sameTerm(?a, ?a) AS ?z) WHERE { ?p v:age ?a OPTIONAL { ?p v:b ?b } }")
    );
    insta::assert_snapshot!(
        "arithmetic",
        ir("SELECT ((-?a + 2 * ?a / 3 - +1) AS ?r) WHERE { ?p v:age ?a }")
    );
}

// task 5.2 / 5.3: whitelisted functions lower, the rest are named
#[test]
fn whitelisted_functions_lower() {
    insta::assert_snapshot!(
        "functions",
        ir("SELECT (SUBSTR(?n, 2) AS ?a) (CONCAT(?n, \"x\") AS ?b) (xsd:integer(?n) AS ?c) (YEAR(?t) AS ?d) (TZ(?t) AS ?e) (isBlank(?p) AS ?f) WHERE { ?p v:n ?n . ?p v:t ?t }")
    );
    // NOW() is a constant fixed at the query start
    insta::assert_snapshot!("now", ir("SELECT (NOW() AS ?n) WHERE { }"));
}

// sparql-query "Built-in functions": every rejected function is named
#[test]
fn rejected_functions_are_named() {
    let table: &[(&str, &str)] = &[
        ("RAND()", "RAND"),
        ("BNODE()", "BNODE"),
        ("UUID()", "UUID"),
        ("STRUUID()", "STRUUID"),
        ("MD5(?n)", "MD5"),
        ("SHA1(?n)", "SHA1"),
        ("SHA256(?n)", "SHA256"),
        ("SHA384(?n)", "SHA384"),
        ("SHA512(?n)", "SHA512"),
        ("TRIPLE(?p, ?p, ?p)", "TRIPLE"),
        ("SUBJECT(?p)", "SUBJECT"),
        ("PREDICATE(?p)", "PREDICATE"),
        ("OBJECT(?p)", "OBJECT"),
        ("isTRIPLE(?p)", "isTRIPLE"),
        ("LANGDIR(?n)", "LANGDIR"),
        ("hasLANG(?n)", "hasLANG"),
        ("hasLANGDIR(?n)", "hasLANGDIR"),
        ("STRLANGDIR(?n, \"en\", \"ltr\")", "STRLANGDIR"),
        ("<http://example.org/fn>(?n)", "http://example.org/fn"),
        (
            "xsd:hexBinary(?n)",
            "http://www.w3.org/2001/XMLSchema#hexBinary",
        ),
    ];
    for (call, feature) in table {
        unsupported(
            &format!("SELECT ({call} AS ?x) WHERE {{ ?p v:n ?n }}"),
            feature,
        );
    }
}
