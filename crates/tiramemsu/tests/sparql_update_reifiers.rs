//! `sparql-rdf12-annotations`, update side (task 8.5).
#![cfg(feature = "sparql")]
mod sparql_common;
use sparql_common::*;
use tiramemsu::*;

fn dec(text: &str) -> Value {
    Value::Decimal(text.to_string())
}

fn no_reifies_stored(t: &T) {
    let reifies = Value::iri(format!("{}reifies", vocab::RDF));
    assert!(t.live().iter().all(|(_, p, _)| *p != reifies));
    assert!(t
        .db
        .history()
        .triples(None, None, None)
        .unwrap()
        .iter()
        .all(|x| { t.db.now().decode(x.p).unwrap() != reifies }));
}

// sparql-rdf12-annotations "Inserting annotations creates layer triples on the eid": INSERT DATA with annotation
#[test]
fn insert_data_with_annotation() {
    let t = T::new();
    let r = t.upd(
        "INSERT DATA { v:alice v:worksAt v:acme {| v:confidence 0.8 ; v:source v:crawler |} }",
    );
    assert_eq!(r.asserted.len(), 3);
    let e1 = r.asserted[0];
    let live = t.live();
    assert_eq!(live.len(), 3);
    let view = t.db.now();
    let rows = view.triples(None, None, None).unwrap();
    let on_e1: Vec<_> = rows.iter().filter(|x| x.s == e1.oid()).collect();
    assert_eq!(on_e1.len(), 2);
    no_reifies_stored(&t);
}

// sparql-rdf12-annotations "Inserting annotations creates layer triples on the eid": Annotation insert is idempotent
#[test]
fn annotation_insert_is_idempotent() {
    let t = T::new();
    let first = t.upd(
        "INSERT DATA { v:alice v:worksAt v:acme {| v:confidence 0.8 ; v:source v:crawler |} }",
    );
    let second = t.upd(
        "INSERT DATA { v:alice v:worksAt v:acme {| v:confidence 0.8 ; v:source v:crawler |} }",
    );
    assert!(second.asserted.is_empty());
    let mut a = first.asserted.clone();
    let mut b = second.existing.clone();
    a.sort();
    b.sort();
    assert_eq!(a, b);
    assert_eq!(t.live().len(), 3);
}

// sparql-rdf12-annotations "Inserting annotations creates layer triples on the eid": Annotating an existing statement
#[test]
fn annotating_an_existing_statement() {
    let t = T::new();
    let e1 = t.assert(&[(v("alice"), v("worksAt"), v("acme"))]).asserted[0];
    let r = t.upd("INSERT DATA { v:alice v:worksAt v:acme {| v:confidence 0.9 |} }");
    assert_eq!(r.existing, vec![e1]);
    assert_eq!(r.asserted.len(), 1);
    let got = t.col(
        "SELECT ?c WHERE { v:alice v:worksAt v:acme {| v:confidence ?c |} }",
        "c",
    );
    assert_eq!(got, some(&[dec("0.9")]));
    let rows = t.db.now().triples(Some(e1.oid()), None, None).unwrap();
    assert_eq!(rows.len(), 1);
}

// sparql-rdf12-annotations "Inserting annotations creates layer triples on the eid": Reference to a reified triple asserts it
#[test]
fn reference_to_a_reified_triple_asserts_it() {
    let t = T::new();
    let r = t.upd("INSERT DATA { v:belief9 v:supportedBy << v:alice v:worksAt v:acme >> }");
    assert_eq!(r.asserted.len(), 2);
    assert!(t.has(&v("alice"), &v("worksAt"), &v("acme")));
    let e1 =
        t.db.now()
            .triples(
                None,
                Some(t.db.now().encode(&v("worksAt")).unwrap().unwrap()),
                None,
            )
            .unwrap()[0]
            .eid;
    assert!(t.has(&v("belief9"), &v("supportedBy"), &Value::Stmt(e1)));
    no_reifies_stored(&t);
}

// sparql-rdf12-annotations "Inserting annotations creates layer triples on the eid": Template attaches to a bound eid
#[test]
fn template_attaches_to_a_bound_eid() {
    let t = T::new();
    let ms = |d: &str| value::parse_datetime(d).unwrap().0;
    let mut eids = Vec::new();
    t.tx(|tx| {
        eids.push(
            tx.assert(
                v("alice"),
                v("worksAt"),
                v("acme"),
                Valid::between(ms("2020-01-01T00:00:00Z"), ms("2022-01-01T00:00:00Z")),
            )?
            .eid(),
        );
        eids.push(
            tx.assert(
                v("alice"),
                v("worksAt"),
                v("acme"),
                Valid::from(ms("2024-01-01T00:00:00Z")),
            )?
            .eid(),
        );
        Ok(())
    });
    t.upd("INSERT { ?r v:reviewed true } WHERE { v:alice v:worksAt v:acme ~ ?r }");
    for e in eids {
        assert!(t.has(&Value::Stmt(e), &v("reviewed"), &Value::Bool(true)));
    }
}

// sparql-rdf12-annotations "Reifier restrictions in updates": IRI reifier rejected
#[test]
fn iri_reifier_rejected() {
    let t = T::new();
    let n = t.last_t();
    let e = t.err("INSERT DATA { v:alice v:worksAt v:acme ~ v:myReifier {| v:confidence 0.8 |} }");
    assert_unsupported(e, "reifier that is not a statement");
    assert_eq!(t.last_t(), n);
    assert!(t.live().is_empty());
}

// sparql-rdf12-annotations "Reifier restrictions in updates": Statement IRI reifier accepted
#[test]
fn statement_iri_reifier_accepted() {
    let t = T::new();
    let e1 = t.assert(&[(v("alice"), v("worksAt"), v("acme"))]).asserted[0];
    let q = format!(
        "INSERT DATA {{ v:alice v:worksAt v:acme ~ <urn:tiramemsu:stmt:{}> {{| v:confidence 0.8 |}} }}",
        e1.n()
    );
    let r = t.upd(&q);
    assert_eq!(r.existing, vec![e1]);
    assert!(t.has(&Value::Stmt(e1), &v("confidence"), &dec("0.8")));
    // a statement IRI whose content differs is rejected
    let q = format!(
        "INSERT DATA {{ v:alice v:worksAt v:other ~ <urn:tiramemsu:stmt:{}> {{| v:confidence 0.9 |}} }}",
        e1.n()
    );
    assert_unsupported(t.err(&q), "reifier that is not a statement");
}

// sparql-rdf12-annotations "Reifier restrictions in updates": One reifier for two triples rejected
#[test]
fn one_reifier_for_two_triples_rejected() {
    let t = T::new();
    let e = t.err(
        "INSERT DATA { _:r rdf:reifies <<( v:a v:b v:c )>> . _:r rdf:reifies <<( v:d v:e v:f )>> }",
    );
    assert_unsupported(e, "reifier of more than one triple");
    assert!(t.live().is_empty());
}

/// e1 and e2: two episodes of (alice worksAt acme); only e1 has a source.
fn two_episodes() -> (T, Eid, Eid) {
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
        tx.assert(a.unwrap(), v("source"), v("crawler"), Valid::ALWAYS)?;
        tx.assert(a.unwrap(), v("confidence"), dec("0.8"), Valid::ALWAYS)?;
        Ok(())
    });
    (t, a.unwrap(), b.unwrap())
}

// sparql-rdf12-annotations "Deleting through reifiers is eid-precise": Retract one episode only
#[test]
fn retract_one_episode_only() {
    let (t, e1, e2) = two_episodes();
    let r = t.upd(
        "DELETE { ?s v:worksAt ?o ~ ?r } WHERE { ?s v:worksAt ?o ~ ?r {| v:source v:crawler |} }",
    );
    assert!(r.retracted.contains(&(e1, RetKind::Explicit)));
    assert_eq!(
        r.retracted
            .iter()
            .filter(|(_, k)| *k == RetKind::Cascade)
            .count(),
        2
    );
    assert!(!r.retracted.iter().any(|(e, _)| *e == e2));
    let live: Vec<Eid> =
        t.db.now()
            .triples(None, None, None)
            .unwrap()
            .iter()
            .map(|x| x.eid)
            .collect();
    assert_eq!(live, vec![e2]);
}

// sparql-rdf12-annotations "Deleting through reifiers is eid-precise": Retract an annotation only
#[test]
fn retract_an_annotation_only() {
    let (t, e1, _) = two_episodes();
    let r = t.upd("DELETE { ?r v:confidence ?c } WHERE { v:alice v:worksAt v:acme ~ ?r {| v:confidence ?c |} }");
    assert_eq!(r.retracted.len(), 1);
    assert!(!t.has(&Value::Stmt(e1), &v("confidence"), &dec("0.8")));
    assert!(t.has(&v("alice"), &v("worksAt"), &v("acme")));
    assert!(t.has(&Value::Stmt(e1), &v("source"), &v("crawler")));
}

#[test]
fn delete_data_with_a_statement_iri_reifier_is_eid_precise() {
    let (t, e1, e2) = two_episodes();
    let q = format!(
        "DELETE DATA {{ v:alice v:worksAt v:acme ~ <urn:tiramemsu:stmt:{}> }}",
        e2.n()
    );
    let r = t.upd(&q);
    assert_eq!(r.retracted, vec![(e2, RetKind::Explicit)]);
    assert!(
        t.db.now()
            .triples(Some(e1.oid()), None, None)
            .unwrap()
            .len()
            == 2
    );
    assert!(t
        .live()
        .iter()
        .any(|(s, p, _)| *s == v("alice") && *p == v("worksAt")));
}
