//! `path-lowering` (SPARQL): recursive and non-recursive property paths, endpoint
//! binding, temporal scope, layer hops and the unsupported forms.
#![cfg(feature = "sparql")]
mod sparql_common;
use sparql_common::*;
use tiramemsu::*;

const P: &str = "urn:tiramemsu:tm:";

fn chain() -> T {
    let t = T::new();
    t.assert(&[(v("a"), v("knows"), v("b")), (v("b"), v("knows"), v("c"))]);
    t
}

fn set(t: &T, q: &str) -> Vec<String> {
    let mut out: Vec<String> = t
        .col(q, "x")
        .into_iter()
        .map(|c| match c {
            Some(Value::Iri(i)) => i.trim_start_matches("urn:tiramemsu:v:").to_string(),
            other => format!("{other:?}"),
        })
        .collect();
    out.sort();
    out
}

// path-lowering "One-or-more from a bound subject" / "Zero-or-more includes the subject" /
// "Zero-or-one"
#[test]
fn closures_from_a_bound_subject() {
    let t = chain();
    assert_eq!(set(&t, "SELECT ?x WHERE { v:a v:knows+ ?x }"), ["b", "c"]);
    assert_eq!(
        set(&t, "SELECT ?x WHERE { v:a v:knows* ?x }"),
        ["a", "b", "c"]
    );
    assert_eq!(set(&t, "SELECT ?x WHERE { v:a v:knows? ?x }"), ["a", "b"]);
}

// "Zero-or-more from a term not in the graph"
#[test]
fn zero_or_more_from_an_unknown_term() {
    let t = chain();
    assert_eq!(
        set(&t, "SELECT ?x WHERE { v:nobody v:knows* ?x }"),
        ["nobody"]
    );
    assert!(set(&t, "SELECT ?x WHERE { v:nobody v:knows+ ?x }").is_empty());
}

// "No duplicate solutions over a diamond"
#[test]
fn no_duplicates_over_a_diamond() {
    let t = T::new();
    t.assert(&[
        (v("a"), v("p"), v("b")),
        (v("a"), v("p"), v("c")),
        (v("b"), v("p"), v("d")),
        (v("c"), v("p"), v("d")),
    ]);
    assert_eq!(set(&t, "SELECT ?x WHERE { v:a v:p+ ?x }"), ["b", "c", "d"]);
}

// "Object bound, subject variable" / "Both endpoints constant" / "Same variable at both ends"
#[test]
fn bound_object_and_both_ends() {
    let t = chain();
    assert_eq!(set(&t, "SELECT ?x WHERE { ?x v:knows+ v:c }"), ["a", "b"]);
    assert!(t.ask("ASK { v:a v:knows+ v:c }"));
    assert!(!t.ask("ASK { v:c v:knows+ v:a }"));
    t.assert(&[(v("c"), v("knows"), v("a"))]);
    assert_eq!(
        set(&t, "SELECT ?x WHERE { VALUES ?x { v:a } ?x v:knows+ ?x }"),
        ["a"]
    );
    let t2 = T::new();
    t2.assert(&[(v("a"), v("p"), v("b")), (v("b"), v("p"), v("a"))]);
    assert_eq!(
        set(&t2, "SELECT ?x WHERE { VALUES ?x { v:a } ?x v:p+ ?x }"),
        ["a"]
    );
}

// "Complex expression"
#[test]
fn complex_expression() {
    let t = T::new();
    t.assert(&[(v("a"), v("knows"), v("b")), (v("e"), v("knows"), v("b"))]);
    assert_eq!(
        set(&t, "SELECT ?x WHERE { v:a (v:knows/^v:knows)+ ?x }"),
        ["a", "e"]
    );
}

// "Sequence with both ends unbound" / "Alternation with both ends unbound"
#[test]
fn non_recursive_paths_need_no_bound_endpoint() {
    let t = T::new();
    t.assert(&[
        (v("a"), v("knows"), v("b")),
        (v("b"), v("worksAt"), v("acme")),
        (v("c"), v("likes"), v("d")),
    ]);
    let s = t.sel("SELECT ?x ?y WHERE { ?x v:knows/v:worksAt ?y }");
    assert_eq!(s.rows, vec![vec![Some(v("a")), Some(v("acme"))]]);
    let s = t.sel("SELECT ?x ?y WHERE { ?x v:knows|v:likes ?y }");
    assert_eq!(s.rows.len(), 2);
}

// "Inverse path"
#[test]
fn inverse_path() {
    let t = T::new();
    t.assert(&[(v("alice"), v("worksAt"), v("acme"))]);
    assert_eq!(
        set(&t, "SELECT ?x WHERE { v:acme ^v:worksAt ?x }"),
        ["alice"]
    );
    assert_eq!(
        t.sel("SELECT ?p ?c WHERE { ?c ^v:worksAt ?p }").rows.len(),
        1
    );
}

// "Endpoint bound by a preceding pattern" / "Both endpoints free"
#[test]
fn endpoint_binding() {
    let t = T::new();
    t.assert(&[
        (
            v("a"),
            Value::iri(format!("{}type", vocab::RDF)),
            v("Person"),
        ),
        (v("a"), v("knows"), v("b")),
        (v("b"), v("knows"), v("c")),
    ]);
    let s = t.sel("SELECT ?p ?x WHERE { ?p a v:Person . ?p v:knows+ ?x }");
    assert_eq!(s.rows.len(), 2);
    match t.err("SELECT ?x ?y WHERE { ?x v:knows+ ?y }") {
        Error::Unsupported { feature } => assert!(feature.contains("bound endpoint"), "{feature}"),
        other => panic!("{other:?}"),
    }
}

// "Path inside an asOf service group" / "Before and after in one query" /
// "GRAPH is not a time scope for paths" / "Valid-time default applies to paths"
// (a path variant of tests#Query#Per Pattern Time Scopes)
#[test]
fn paths_honour_temporal_scope() {
    let t = T::new();
    t.advance_to(9);
    t.assert(&[(v("a"), v("knows"), v("b"))]);
    t.advance_to(19);
    t.assert(&[(v("b"), v("knows"), v("c"))]);
    let q = format!("SELECT ?x WHERE {{ SERVICE <{P}asOf/15> {{ v:a v:knows+ ?x }} }}");
    assert_eq!(set(&t, &q), ["b"]);
    let q = format!(
        "SELECT ?x WHERE {{ v:a v:knows+ ?x FILTER NOT EXISTS {{ SERVICE <{P}asOf/15> {{ v:a v:knows+ ?x }} }} }}"
    );
    assert_eq!(set(&t, &q), ["c"]);
    let q = format!("SELECT ?x WHERE {{ GRAPH <{P}asOf/15> {{ v:a v:knows+ ?x }} }}");
    let (_, msg) = assert_parse(t.err(&q));
    assert!(msg.contains("SERVICE"), "{msg}");
    // valid time
    let t = T::new();
    let ms = |d: &str| value::parse_datetime(d).unwrap().0;
    t.tx(|tx| {
        tx.assert(
            v("a"),
            v("worksAt"),
            v("acme"),
            Valid::between(ms("2020-01-01T00:00:00Z"), ms("2022-01-01T00:00:00Z")),
        )?;
        tx.assert(v("acme"), v("locatedIn"), v("berlin"), Valid::ALWAYS)?;
        Ok(())
    });
    let q =
        format!("SELECT ?x FROM <{P}validAt/2023-06-01> WHERE {{ v:a v:worksAt/v:locatedIn* ?x }}");
    assert!(set(&t, &q).is_empty());
    let q =
        format!("SELECT ?x FROM <{P}validAt/2021-06-01> WHERE {{ v:a v:worksAt/v:locatedIn* ?x }}");
    assert_eq!(set(&t, &q), ["acme", "berlin"]);
}

// "From a belief to the entities of its supporting fact": virtual hops in a recursive path
#[test]
fn paths_across_layers() {
    let t = T::new();
    t.tx(|tx| {
        let e1 = tx
            .assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?
            .eid();
        tx.assert(v("belief9"), v("supportedBy"), e1, Valid::ALWAYS)?;
        Ok(())
    });
    assert_eq!(
        set(
            &t,
            "SELECT ?x WHERE { v:belief9 v:supportedBy/(sys:subject|sys:object)+ ?x }"
        ),
        ["acme", "alice"]
    );
    // a reifier variable as a path endpoint
    let s = t
        .sel("SELECT ?x WHERE { v:alice v:worksAt v:acme ~ ?r . ?r (sys:subject|sys:object)+ ?x }");
    assert_eq!(s.rows.len(), 2);
}

// "Negated property set"
#[test]
fn negated_property_sets_are_unsupported() {
    let t = chain();
    assert_unsupported(
        t.err("SELECT ?x WHERE { v:a !v:knows ?x }"),
        "negated property sets",
    );
    assert_unsupported(
        t.err("SELECT ?x WHERE { v:a !(v:knows|^v:likes) ?x }"),
        "negated property sets",
    );
}

// "SPARQL has no cap" (path-evaluation "Hop limits")
#[test]
fn sparql_paths_have_no_hop_cap() {
    let t = T::new();
    let edges: Vec<_> = (0..20)
        .map(|i| (v(&format!("n{i}")), v("next"), v(&format!("n{}", i + 1))))
        .collect();
    t.assert(&edges);
    assert_eq!(set(&t, "SELECT ?x WHERE { v:n0 v:next+ ?x }").len(), 20);
}
