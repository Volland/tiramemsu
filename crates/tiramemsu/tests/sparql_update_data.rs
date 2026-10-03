//! `sparql-update`: INSERT DATA and DELETE DATA (task 8.3).
#![cfg(feature = "sparql")]
mod sparql_common;
use sparql_common::*;
use tiramemsu::*;

// sparql-update "INSERT DATA asserts idempotently": Repeated insert is idempotent
#[test]
fn repeated_insert_is_idempotent() {
    let t = T::new();
    let r1 = t.upd("INSERT DATA { v:alice v:worksAt v:acme }");
    let r2 = t.upd("INSERT DATA { v:alice v:worksAt v:acme }");
    assert_eq!(t.live().len(), 1);
    assert_eq!(r2.existing, r1.asserted);
    assert!(r2.asserted.is_empty());
}

// sparql-update "INSERT DATA asserts idempotently": Blank nodes are fresh per request
#[test]
fn blank_nodes_are_fresh_per_request() {
    let t = T::new();
    t.upd("INSERT DATA { _:b v:name \"anon\" }");
    t.upd("INSERT DATA { _:b v:name \"anon\" }");
    let got = t.col("SELECT DISTINCT ?x WHERE { ?x v:name \"anon\" }", "x");
    assert_eq!(got.len(), 2);
    assert!(got.iter().all(|x| matches!(x, Some(Value::BNode(_)))));
    // one blank node per label within a request
    t.upd("INSERT DATA { _:c v:name \"one\" . _:c v:age 1 }");
    let r = t.sel("SELECT ?x WHERE { ?x v:name \"one\" . ?x v:age 1 }");
    assert_eq!(r.rows.len(), 1);
}

// sparql-update "INSERT DATA asserts idempotently": Canonical literal stored
#[test]
fn canonical_literal_stored() {
    let t = T::new();
    t.upd("INSERT DATA { v:alice v:age \"041\"^^xsd:integer }");
    assert_eq!(
        t.col("SELECT ?a WHERE { v:alice v:age ?a }", "a"),
        some(&[int(41)])
    );
    let json =
        t.db.now()
            .sparql("SELECT ?a WHERE { v:alice v:age ?a }")
            .unwrap()
            .write_sparql_json()
            .unwrap();
    assert!(json.contains("\"value\":\"41\""));
}

#[test]
fn inserted_statements_have_unbounded_valid_time() {
    let t = T::new();
    t.upd("INSERT DATA { v:a v:b v:c }");
    let row = t.db.now().triples(None, None, None).unwrap()[0];
    assert_eq!((row.v_from, row.v_to), (None, None));
}

/// alice works at acme twice (two episodes), with an annotation on the first.
fn episodes() -> (T, Eid, Eid) {
    let t = T::new();
    let ms = |d: &str| value::parse_datetime(d).unwrap().0;
    let (mut a, mut b) = (None, None);
    t.tx(|tx| {
        a = Some(
            tx.assert(
                v("alice"),
                v("worksAt"),
                v("acme"),
                Valid::between(ms("2020-01-01T00:00:00Z"), ms("2022-01-01T00:00:00Z")),
            )?
            .eid(),
        );
        b = Some(
            tx.assert(
                v("alice"),
                v("worksAt"),
                v("acme"),
                Valid::from(ms("2024-01-01T00:00:00Z")),
            )?
            .eid(),
        );
        tx.assert(
            a.unwrap(),
            v("confidence"),
            Value::Decimal("0.8".into()),
            Valid::ALWAYS,
        )?;
        tx.assert(v("alice"), v("age"), int(41), Valid::ALWAYS)?;
        Ok(())
    });
    (t, a.unwrap(), b.unwrap())
}

// sparql-update "DELETE DATA retracts with cascade": All episodes retracted
#[test]
fn all_episodes_retracted() {
    let (t, a, b) = episodes();
    let r = t.upd("DELETE DATA { v:alice v:worksAt v:acme }");
    let explicit: Vec<Eid> = r
        .retracted
        .iter()
        .filter(|(_, k)| *k == RetKind::Explicit)
        .map(|(e, _)| *e)
        .collect();
    assert_eq!(explicit, vec![a, b]);
    assert!(!t.has(&v("alice"), &v("worksAt"), &v("acme")));
}

// sparql-update "DELETE DATA retracts with cascade": Cascade retracts annotations
#[test]
fn cascade_retracts_annotations() {
    let (t, a, _) = episodes();
    let r = t.upd("DELETE DATA { v:alice v:worksAt v:acme }");
    assert!(r.retracted.contains(&(a, RetKind::Explicit)));
    let cascaded: Vec<_> = r
        .retracted
        .iter()
        .filter(|(_, k)| *k == RetKind::Cascade)
        .collect();
    assert_eq!(cascaded.len(), 1);
    // the previous transaction still shows both
    let before = t.db.as_of(TimeRef::Tx(r.t.0 - 1));
    let got = sel_on(
        &before,
        "SELECT ?c WHERE { v:alice v:worksAt v:acme {| v:confidence ?c |} }",
    );
    assert_eq!(got.rows.len(), 1);
    // retraction rows share one t_ret
    let hist = t.db.history().triples(None, None, None).unwrap();
    let rets: std::collections::BTreeSet<_> = hist.iter().filter_map(|x| x.t_ret).collect();
    assert_eq!(rets.len(), 1);
}

// sparql-update "DELETE DATA retracts with cascade": Other triples about the same nodes survive
#[test]
fn other_triples_about_the_same_nodes_survive() {
    let (t, _, _) = episodes();
    t.upd("DELETE DATA { v:alice v:worksAt v:acme }");
    assert!(t.has(&v("alice"), &v("age"), &int(41)));
}
