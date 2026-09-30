//! `sparql-update`: DELETE/INSERT WHERE, time-scoped WHERE and store constraints
//! (tasks 8.4 and 8.6).
mod sparql_common;
use sparql_common::*;
use tiramemsu::*;

// sparql-update "DELETE/INSERT WHERE": Replace a value
#[test]
fn replace_a_value() {
    let t = T::new();
    t.assert(&[(v("alice"), v("worksAt"), v("acme"))]);
    let r = t.upd(
        "DELETE { v:alice v:worksAt ?c } INSERT { v:alice v:worksAt v:initech } WHERE { v:alice v:worksAt ?c }",
    );
    assert!(!t.has(&v("alice"), &v("worksAt"), &v("acme")));
    assert!(t.has(&v("alice"), &v("worksAt"), &v("initech")));
    assert_eq!(r.retracted.len(), 1);
    assert_eq!(r.asserted.len(), 1);
    let hist = t.db.history().triples(None, None, None).unwrap();
    assert!(hist
        .iter()
        .all(|x| x.t_add == r.t || x.t_ret == Some(r.t) || x.t_add.0 == 1));
    let old = hist.iter().find(|x| x.t_ret.is_some()).unwrap();
    assert_eq!(old.t_ret, Some(r.t));
}

// sparql-update "DELETE/INSERT WHERE": WHERE is evaluated before changes
#[test]
fn where_is_evaluated_before_changes() {
    let t = T::new();
    t.assert(&[(v("a"), v("next"), v("b")), (v("b"), v("next"), v("c"))]);
    t.upd("DELETE { ?x v:next ?y } INSERT { ?y v:prev ?x } WHERE { ?x v:next ?y }");
    assert!(!t.has(&v("a"), &v("next"), &v("b")) && !t.has(&v("b"), &v("next"), &v("c")));
    assert!(t.has(&v("b"), &v("prev"), &v("a")) && t.has(&v("c"), &v("prev"), &v("b")));
}

// sparql-update "DELETE/INSERT WHERE": Unbound template variable is skipped
#[test]
fn unbound_template_variable_is_skipped() {
    let t = T::new();
    t.assert(&[(v("alice"), v("name"), s("Alice"))]);
    let r = t.upd("INSERT { ?p v:ageCopy ?a } WHERE { ?p v:name ?n OPTIONAL { ?p v:age ?a } }");
    assert!(r.asserted.is_empty());
    assert_eq!(t.live().len(), 1);
}

// sparql-update "DELETE/INSERT WHERE": DELETE WHERE short form
#[test]
fn delete_where_short_form() {
    let t = T::new();
    t.assert(&[
        (v("alice"), v("tag"), s("x")),
        (v("bob"), v("tag"), s("x")),
        (v("bob"), v("age"), int(3)),
    ]);
    let r = t.upd("DELETE WHERE { ?p v:tag \"x\" }");
    assert_eq!(r.retracted.len(), 2);
    assert_eq!(t.live().len(), 1);
}

// sparql-update "DELETE/INSERT WHERE": Fresh blank node per solution
#[test]
fn fresh_blank_node_per_solution() {
    let t = T::new();
    let a = Value::iri(format!("{}type", vocab::RDF));
    t.assert(&[
        (v("alice"), a.clone(), v("Person")),
        (v("bob"), a, v("Person")),
    ]);
    t.upd("INSERT { ?p v:card _:c . _:c v:owner ?p } WHERE { ?p a v:Person }");
    let cards = t.col(
        "SELECT DISTINCT ?c WHERE { ?p v:card ?c . ?c v:owner ?p }",
        "c",
    );
    assert_eq!(cards.len(), 2);
}

// sparql-update "Time-scoped WHERE in updates": Restore a past value
#[test]
fn restore_a_past_value() {
    let t = T::new();
    let e_old = t.assert(&[(v("alice"), v("worksAt"), v("acme"))]).asserted[0]; // tx 1
    t.retract(&v("alice"), &v("worksAt"), &v("acme")); // tx 2
    let r = t.upd(
        "INSERT { v:alice v:worksAt ?c } WHERE { SERVICE <urn:tiramemsu:tm:asOf/1> { v:alice v:worksAt ?c } }",
    );
    assert!(t.has(&v("alice"), &v("worksAt"), &v("acme")));
    assert_eq!(r.asserted.len(), 1);
    assert_ne!(r.asserted[0], e_old);
    let old =
        t.db.history()
            .triples(None, None, None)
            .unwrap()
            .into_iter()
            .find(|x| x.eid == e_old)
            .unwrap();
    assert!(old.t_ret.is_some());
}

#[test]
fn using_sets_the_where_default_view() {
    let t = T::new();
    t.assert(&[(v("alice"), v("worksAt"), v("acme"))]); // tx 1
    t.retract(&v("alice"), &v("worksAt"), &v("acme")); // tx 2
    t.upd("INSERT { v:alice v:formerly ?c } USING <urn:tiramemsu:tm:asOf/1> WHERE { v:alice v:worksAt ?c }");
    assert!(t.has(&v("alice"), &v("formerly"), &v("acme")));
    // FROM NAMED / USING NAMED with a time IRI is accepted and has no effect
    t.upd("INSERT { v:x v:y v:z } USING NAMED <urn:tiramemsu:tm:asOf/1> WHERE { }");
    assert!(t.has(&v("x"), &v("y"), &v("z")));
}

// sparql-update "Time-scoped WHERE in updates": Time IRI in GRAPH of a WHERE pattern is rejected
#[test]
fn time_iri_in_graph_of_a_where_pattern_is_rejected() {
    let t = T::new();
    let before = t.last_t();
    let e = t.err(
        "INSERT { v:alice v:worksAt ?c } WHERE { GRAPH <urn:tiramemsu:tm:asOf/150> { v:alice v:worksAt ?c } }",
    );
    let (_, msg) = assert_parse(e);
    assert!(msg.contains("SERVICE"));
    assert_eq!(t.last_t(), before);
}

// sparql-update "Store constraints apply to SPARQL updates": Cardinality one replaces the old value
#[test]
fn cardinality_one_replaces_the_old_value() {
    let t = T::new();
    t.tx(|tx| {
        tx.assert(
            v("age"),
            Value::iri(vocab::SYS_CARDINALITY),
            Value::iri(vocab::SYS_ONE),
            Valid::ALWAYS,
        )?;
        tx.assert(v("alice"), v("age"), int(41), Valid::ALWAYS)?;
        Ok(())
    });
    let r = t.upd("INSERT DATA { v:alice v:age 42 }");
    assert!(t.has(&v("alice"), &v("age"), &int(42)) && !t.has(&v("alice"), &v("age"), &int(41)));
    assert!(r.retracted.iter().any(|(_, k)| *k == RetKind::Cardinality));
}

// sparql-update "Store constraints apply to SPARQL updates": Unique violation aborts
#[test]
fn unique_violation_aborts() {
    let t = T::new();
    t.tx(|tx| {
        tx.assert(
            v("email"),
            Value::iri(vocab::SYS_UNIQUE),
            Value::Bool(true),
            Valid::ALWAYS,
        )?;
        tx.assert(v("alice"), v("email"), s("a@x.org"), Valid::ALWAYS)?;
        Ok(())
    });
    let n = t.live().len();
    let e = t.err("INSERT DATA { v:bob v:email \"a@x.org\" }");
    assert!(matches!(e, Error::UniqueViolation { .. }), "{e:?}");
    assert_eq!(t.live().len(), n);
}

// sparql-update "Store constraints apply to SPARQL updates": Schema flag insertable
#[test]
fn schema_flag_insertable() {
    let t = T::new();
    t.upd("INSERT DATA { v:email sys:unique true }");
    t.upd("INSERT DATA { v:alice v:email \"a@x.org\" }");
    let e = t.err("INSERT DATA { v:bob v:email \"a@x.org\" }");
    assert!(matches!(e, Error::UniqueViolation { .. }), "{e:?}");
}

// sparql-update "Store constraints apply to SPARQL updates": Reserved namespace rejected
#[test]
fn reserved_namespace_rejected() {
    let t = T::new();
    let e = t.err("INSERT DATA { v:alice sys:supersedes v:bob }");
    assert!(
        matches!(&e, Error::ReservedNamespace(p) if p == "urn:tiramemsu:sys:supersedes"),
        "{e:?}"
    );
}

// sparql-update "Virtual predicates are read-only": Valid time cannot be inserted as a triple
#[test]
fn valid_time_cannot_be_inserted_as_a_triple() {
    let t = T::new();
    t.assert(&[(v("alice"), v("worksAt"), v("acme"))]);
    let n = t.last_t();
    let e = t.err(
        "INSERT { ?r tm:validFrom \"2020-01-01T00:00:00Z\"^^xsd:dateTime } WHERE { v:alice v:worksAt v:acme ~ ?r }",
    );
    assert!(
        matches!(&e, Error::ReservedNamespace(p) if p == "urn:tiramemsu:tm:validFrom"),
        "{e:?}"
    );
    assert_eq!(t.last_t(), n);
    // the virtual hop predicates are read-only too
    for p in [
        "sys:subject",
        "sys:object",
        "sys:predicate",
        "tm:txAdded",
        "tm:txRetracted",
        "tm:validTo",
        "tm:retractKind",
    ] {
        let e = t.err(&format!("INSERT DATA {{ v:a {p} v:b }}"));
        assert!(matches!(e, Error::ReservedNamespace(_)), "{p}: {e:?}");
    }
}
