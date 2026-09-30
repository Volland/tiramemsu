//! Golden IR for reifiers, annotations and triple terms (6.1–6.3).
mod common;
use common::*;

#[test]
fn reifier_forms() {
    insta::assert_snapshot!(
        "tilde_reifier",
        ir("SELECT ?r WHERE { v:alice v:worksAt v:acme ~ ?r }")
    );
    insta::assert_snapshot!(
        "explicit_reifies",
        ir("SELECT ?r WHERE { ?r rdf:reifies <<( v:alice v:worksAt v:acme )>> }")
    );
    insta::assert_snapshot!(
        "reified_triple_subject",
        ir("SELECT ?c WHERE { << v:alice v:worksAt v:acme >> v:confidence ?c }")
    );
    insta::assert_snapshot!(
        "statement_iri_reifier",
        ir("SELECT ?c WHERE { <urn:tiramemsu:stmt:12> rdf:reifies <<( v:alice v:worksAt ?c )>> }")
    );
}

#[test]
fn annotation_forms() {
    insta::assert_snapshot!(
        "annotation_block",
        ir("SELECT ?c ?s WHERE { v:alice v:worksAt v:acme {| v:confidence ?c ; v:source ?s |} }")
    );
    insta::assert_snapshot!(
        "annotation_with_reifier",
        ir("SELECT ?r ?c WHERE { v:alice v:worksAt v:acme ~ ?r {| v:confidence ?c |} }")
    );
    insta::assert_snapshot!(
        "nested_annotation",
        ir("SELECT ?c ?m WHERE { v:alice v:worksAt v:acme {| v:confidence ?c ~ ?r2 {| v:method ?m |} |} }")
    );
}

#[test]
fn triple_terms_in_object_position() {
    insta::assert_snapshot!(
        "triple_term_object",
        ir("ASK { v:belief9 v:supportedBy <<( v:alice v:worksAt v:acme )>> }")
    );
    insta::assert_snapshot!(
        "nested_triple_term",
        ir("SELECT ?who WHERE { ?who v:doubts <<( v:bob v:says <<( v:alice v:worksAt v:acme )>> )>> }")
    );
}

#[test]
fn reifier_that_is_no_statement_is_empty() {
    insta::assert_snapshot!(
        "non_statement_reifier",
        ir("ASK { v:alice v:worksAt v:acme ~ v:someIri }")
    );
}

// the DELETE WHERE shape: a Join of one-triple BGPs is flattened (6.1)
#[test]
fn joined_bgps_are_flattened_before_redundancy_elimination() {
    let text = "SELECT ?r WHERE { v:alice v:worksAt v:acme . { } v:alice v:worksAt v:acme ~ ?r }";
    let out = ir(text);
    assert_eq!(out.matches("(triple").count(), 1, "{out}");
}

#[test]
fn rejections() {
    unsupported(
        "SELECT ?t WHERE { ?r rdf:reifies ?t }",
        "rdf:reifies without triple term",
    );
    unsupported(
        "SELECT ?t WHERE { ?s ?p <<( v:a v:b v:c )>> }",
        "variable predicate with triple term",
    );
}
