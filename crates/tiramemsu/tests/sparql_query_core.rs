//! `sparql-query`: views, query forms, patterns, set semantics, OPTIONAL, UNION,
//! MINUS/EXISTS, BIND and VALUES.
#![cfg(feature = "sparql")]
mod sparql_common;
use sparql_common::*;
use tiramemsu::*;

fn alice_acme_retracted() -> T {
    let t = T::new();
    t.assert(&[(v("alice"), v("worksAt"), v("acme"))]); // tx 1
    t.retract(&v("alice"), &v("worksAt"), &v("acme")); // tx 2
    t
}

// sparql-query "SPARQL queries run against a view": Current view sees live statements only
#[test]
fn current_view_sees_live_statements_only() {
    let t = alice_acme_retracted();
    let s = t.sel("SELECT ?c WHERE { v:alice v:worksAt ?c }");
    assert_eq!(s.vars, vec!["c".to_string()]);
    assert!(s.rows.is_empty());
}

// sparql-query "SPARQL queries run against a view": Historical view sees the past
#[test]
fn historical_view_sees_the_past() {
    let t = alice_acme_retracted();
    let s = sel_on(
        &t.db.as_of(TimeRef::Tx(1)),
        "SELECT ?c WHERE { v:alice v:worksAt ?c }",
    );
    assert_eq!(s.rows, vec![vec![Some(v("acme"))]]);
}

// sparql-query "SPARQL queries run against a view": Speculative view sees uncommitted state
#[test]
fn speculative_view_sees_uncommitted_state() {
    let t = T::new();
    let q = "ASK { v:bob v:worksAt v:initech }";
    let inside =
        t.db.with(
            |tx| {
                tx.assert(v("bob"), v("worksAt"), v("initech"), Valid::ALWAYS)?;
                Ok(())
            },
            |view| view.sparql(q),
        )
        .unwrap();
    assert_eq!(inside, SparqlResult::Boolean(true));
    assert!(!t.ask(q));
}

// sparql-query "Supported query forms": SELECT with projected expression
#[test]
fn select_with_projected_expression() {
    let t = T::new();
    t.assert(&[(v("alice"), v("age"), int(41))]);
    let s = t.sel("SELECT ?p ((?a + 1) AS ?next) WHERE { ?p v:age ?a }");
    assert_eq!(s.vars, vec!["p".to_string(), "next".to_string()]);
    assert_eq!(s.rows, vec![vec![Some(v("alice")), Some(int(42))]]);
}

// sparql-query "Supported query forms": SELECT star hides internal variables
#[test]
fn select_star_hides_internal_variables() {
    let t = T::new();
    t.assert(&[(v("alice"), v("worksAt"), v("acme"))]);
    let s = t.sel("SELECT * WHERE { ?s v:worksAt [] }");
    assert_eq!(s.vars, vec!["s".to_string()]);
    assert_eq!(s.rows, vec![vec![Some(v("alice"))]]);
}

// sparql-query "Supported query forms": ASK true and false
#[test]
fn ask_true_and_false() {
    let t = T::new();
    t.assert(&[(v("alice"), v("worksAt"), v("acme"))]);
    let q = "ASK { v:alice v:worksAt v:acme }";
    assert!(t.ask(q));
    t.retract(&v("alice"), &v("worksAt"), &v("acme"));
    assert!(!t.ask(q));
}

// sparql-query "Basic graph patterns and joins": Two-pattern join
#[test]
fn two_pattern_join() {
    let t = T::new();
    t.assert(&[
        (v("alice"), v("worksAt"), v("acme")),
        (v("acme"), v("locatedIn"), v("berlin")),
        (v("bob"), v("worksAt"), v("initech")),
    ]);
    let s = t.sel("SELECT ?p ?city WHERE { ?p v:worksAt ?c . ?c v:locatedIn ?city }");
    assert_eq!(s.rows, vec![vec![Some(v("alice")), Some(v("berlin"))]]);
}

// sparql-query "Basic graph patterns and joins": Unknown constant yields empty result
#[test]
fn unknown_constant_yields_empty_result() {
    let t = T::new();
    t.assert(&[(v("alice"), v("worksAt"), v("acme"))]);
    assert!(t
        .sel("SELECT ?o WHERE { v:neverSeen v:worksAt ?o }")
        .rows
        .is_empty());
}

// sparql-query "Basic graph patterns and joins": Variable in predicate position
#[test]
fn variable_in_predicate_position() {
    let t = T::new();
    t.assert(&[
        (v("alice"), v("worksAt"), v("acme")),
        (v("alice"), v("age"), int(41)),
    ]);
    let got = t.col("SELECT ?p WHERE { v:alice ?p ?o } ORDER BY ?p", "p");
    assert_eq!(got, some(&[v("age"), v("worksAt")]));
}

/// alice works at acme as two episodes.
fn two_episodes() -> T {
    let t = T::new();
    let ms = |d: &str| value::parse_datetime(d).unwrap().0;
    t.tx(|tx| {
        tx.assert(
            v("alice"),
            v("worksAt"),
            v("acme"),
            Valid::between(ms("2020-01-01T00:00:00Z"), ms("2022-01-01T00:00:00Z")),
        )?;
        tx.assert(
            v("alice"),
            v("worksAt"),
            v("acme"),
            Valid::from(ms("2024-01-01T00:00:00Z")),
        )?;
        Ok(())
    });
    t
}

// sparql-query "Set-of-triples semantics over eids": Two episodes show as one triple
#[test]
fn two_episodes_show_as_one_triple() {
    let t = two_episodes();
    assert_eq!(
        t.col("SELECT ?c WHERE { v:alice v:worksAt ?c }", "c"),
        some(&[v("acme")])
    );
    // One row per episode when the eid is bound
    assert_eq!(
        t.sel("SELECT ?r WHERE { v:alice v:worksAt ?c ~ ?r }")
            .rows
            .len(),
        2
    );
}

// sparql-query "Set-of-triples semantics over eids": Parallel edges created by Cypher show as one triple
#[test]
fn parallel_edges_show_as_one_triple() {
    let t = T::new();
    t.tx(|tx| {
        tx.create(v("alice"), v("called"), v("bob"), Valid::ALWAYS)?;
        tx.create(v("alice"), v("called"), v("bob"), Valid::ALWAYS)?;
        Ok(())
    });
    let s = t.sel("SELECT (COUNT(*) AS ?n) WHERE { v:alice v:called v:bob }");
    assert_eq!(s.rows, vec![vec![Some(int(1))]]);
}

// sparql-query "Set-of-triples semantics over eids": Binding the eid exposes every occurrence
#[test]
fn binding_the_eid_exposes_every_occurrence() {
    let t = T::new();
    t.tx(|tx| {
        tx.create(v("alice"), v("called"), v("bob"), Valid::ALWAYS)?;
        tx.create(v("alice"), v("called"), v("bob"), Valid::ALWAYS)?;
        Ok(())
    });
    assert_eq!(
        t.sel("SELECT ?r WHERE { v:alice v:called v:bob ~ ?r }")
            .rows
            .len(),
        2
    );
}

// sparql-query "Set-of-triples semantics over eids": Projection keeps bag semantics
#[test]
fn projection_keeps_bag_semantics() {
    let t = T::new();
    t.assert(&[
        (v("alice"), v("knows"), v("bob")),
        (v("alice"), v("knows"), v("carol")),
    ]);
    assert_eq!(t.sel("SELECT ?s WHERE { ?s v:knows ?o }").rows.len(), 2);
    assert_eq!(
        t.sel("SELECT DISTINCT ?s WHERE { ?s v:knows ?o }")
            .rows
            .len(),
        1
    );
}

// sparql-query "Homomorphic matching": Same statement matched by two patterns
#[test]
fn same_statement_matched_by_two_patterns() {
    let t = T::new();
    t.assert(&[(v("alice"), v("knows"), v("bob"))]);
    let s = t.sel("SELECT ?a ?c WHERE { ?a v:knows ?b . ?c v:knows ?d }");
    assert_eq!(s.rows, vec![vec![Some(v("alice")), Some(v("alice"))]]);
}

// sparql-query "OPTIONAL": Missing optional value is unbound
#[test]
fn missing_optional_value_is_unbound() {
    let t = T::new();
    t.assert(&[
        (v("alice"), v("age"), int(41)),
        (v("bob"), v("name"), s("Bob")),
    ]);
    let r = t.sel("SELECT ?p ?age WHERE { ?p v:name ?n OPTIONAL { ?p v:age ?age } }");
    assert_eq!(r.rows, vec![vec![Some(v("bob")), None]]);
}

// sparql-query "OPTIONAL": Filter inside OPTIONAL is a join condition
#[test]
fn filter_inside_optional_is_a_join_condition() {
    let t = T::new();
    t.assert(&[
        (v("alice"), v("name"), s("Alice")),
        (v("alice"), v("age"), int(41)),
    ]);
    let r =
        t.sel("SELECT ?p ?age WHERE { ?p v:name ?n OPTIONAL { ?p v:age ?age FILTER(?age > 50) } }");
    assert_eq!(r.rows, vec![vec![Some(v("alice")), None]]);
}

// sparql-query "UNION": Union of two predicates
#[test]
fn union_of_two_predicates() {
    let t = T::new();
    t.assert(&[
        (v("alice"), v("email"), s("a@x.org")),
        (v("bob"), v("phone"), s("123")),
    ]);
    let mut r = t
        .sel("SELECT ?p ?e ?t WHERE { { ?p v:email ?e } UNION { ?p v:phone ?t } }")
        .rows;
    r.sort_by_key(|row| format!("{:?}", row[0]));
    assert_eq!(
        r,
        vec![
            vec![Some(v("alice")), Some(s("a@x.org")), None],
            vec![Some(v("bob")), None, Some(s("123"))],
        ]
    );
}

fn people() -> T {
    let t = T::new();
    t.assert(&[
        (
            v("alice"),
            Value::iri(vocab::RDF.to_string() + "type"),
            v("Person"),
        ),
        (
            v("bob"),
            Value::iri(vocab::RDF.to_string() + "type"),
            v("Person"),
        ),
        (v("bob"), v("banned"), Value::Bool(true)),
    ]);
    t
}

// sparql-query "MINUS and EXISTS": MINUS removes matching subjects
#[test]
fn minus_removes_matching_subjects() {
    let t = people();
    let got = t.col(
        "SELECT ?p WHERE { ?p a v:Person MINUS { ?p v:banned true } }",
        "p",
    );
    assert_eq!(got, some(&[v("alice")]));
}

// sparql-query "MINUS and EXISTS": MINUS without shared variables removes nothing
#[test]
fn minus_without_shared_variables_removes_nothing() {
    let t = people();
    let got = t.col(
        "SELECT ?p WHERE { ?p a v:Person MINUS { ?x v:banned true } } ORDER BY ?p",
        "p",
    );
    assert_eq!(got, some(&[v("alice"), v("bob")]));
}

// sparql-query "MINUS and EXISTS": NOT EXISTS is correlated
#[test]
fn not_exists_is_correlated() {
    let t = people();
    let got = t.col(
        "SELECT ?p WHERE { ?p a v:Person FILTER NOT EXISTS { ?p v:banned true } }",
        "p",
    );
    assert_eq!(got, some(&[v("alice")]));
}

// sparql-query "BIND and VALUES": BIND computes a value
#[test]
fn bind_computes_a_value() {
    let t = T::new();
    t.assert(&[(v("alice"), v("age"), int(41))]);
    assert_eq!(
        t.col(
            "SELECT ?y WHERE { v:alice v:age ?a BIND(2026 - ?a AS ?y) }",
            "y"
        ),
        some(&[int(1985)])
    );
}

// sparql-query "BIND and VALUES": BIND error leaves the variable unbound
#[test]
fn bind_error_leaves_the_variable_unbound() {
    let t = T::new();
    t.assert(&[(v("bob"), v("age"), s("unknown"))]);
    let r = t.sel("SELECT ?p ?y WHERE { ?p v:age ?a BIND(?a * 2 AS ?y) }");
    assert_eq!(r.rows, vec![vec![Some(v("bob")), None]]);
}

// sparql-query "BIND and VALUES": BIND over an in-scope variable is rejected
#[test]
fn bind_over_an_in_scope_variable_is_rejected() {
    let t = T::new();
    assert_parse(t.err("SELECT ?a WHERE { v:alice v:age ?a BIND(1 AS ?a) }"));
}

fn ages() -> T {
    let t = T::new();
    t.assert(&[
        (v("alice"), v("age"), int(41)),
        (v("bob"), v("age"), int(30)),
    ]);
    t
}

// sparql-query "BIND and VALUES": VALUES restricts solutions
#[test]
fn values_restricts_solutions() {
    let t = ages();
    let r = t.sel("SELECT ?p ?a WHERE { VALUES ?p { v:bob v:zoe } ?p v:age ?a }");
    assert_eq!(r.rows, vec![vec![Some(v("bob")), Some(int(30))]]);
}

// sparql-query "BIND and VALUES": UNDEF in VALUES
#[test]
fn undef_in_values() {
    let t = ages();
    let r = t.sel("SELECT ?p ?a WHERE { ?p v:age ?a } VALUES (?p ?a) { (UNDEF 30) }");
    assert_eq!(r.rows, vec![vec![Some(v("bob")), Some(int(30))]]);
}
