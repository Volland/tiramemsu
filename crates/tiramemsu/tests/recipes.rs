//! Layer recipes (`lat.md/recipes.md`): questions answered by combining statement
//! ids, layers, transaction metadata, paths and time scopes, with no new feature.
mod sparql_common;
use sparql_common::*;
use tiramemsu::*;

fn ms(d: &str) -> i64 {
    value::parse_datetime(d).unwrap().0
}

/// The skolem IRI of a statement in angle brackets, for use in a query text.
fn stmt_iri(e: Eid) -> String {
    Value::Stmt(e).to_string()
}

/// e0 = (doc7 says "Alice joined Acme"), e1 = (alice worksAt acme) derived from
/// e0, and belief9 supported by e1. Then the derivation link is retracted.
/// Returns (t, e0, e1, the tx before the retraction).
fn evidence_fixture() -> (T, Eid, Eid, u64) {
    let t = T::new();
    let (mut e0, mut e1) = (None, None);
    let t1 = t
        .tx(|tx| {
            let src = tx
                .assert(v("doc7"), v("says"), s("Alice joined Acme"), Valid::ALWAYS)?
                .eid();
            let fact = tx
                .assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?
                .eid();
            tx.assert(fact, v("derivedFrom"), src, Valid::ALWAYS)?;
            tx.assert(v("belief9"), v("supportedBy"), fact, Valid::ALWAYS)?;
            (e0, e1) = (Some(src), Some(fact));
            Ok(())
        })
        .t
        .0;
    let (e0, e1) = (e0.unwrap(), e1.unwrap());
    t.tx(|tx| {
        tx.retract_matching(Some(e1.oid()), None, Some(e0.oid()))?;
        Ok(())
    });
    (t, e0, e1, t1)
}

// @lat: [[tests#Recipes#Evidence Chains Travel In Time]]
#[test]
fn evidence_chain_as_of_an_earlier_transaction() {
    let (t, e0, e1, t1) = evidence_fixture();
    let q = |scope: &str| {
        format!(
            "SELECT ?src WHERE {{ SERVICE <urn:tiramemsu:tm:{scope}> {{ \
               v:belief9 v:supportedBy/v:derivedFrom* ?src }} }} ORDER BY ?src"
        )
    };
    // today the belief rests on the fact alone: the derivation was retracted
    assert_eq!(t.col(&q("asOf/9999"), "src"), some(&[Value::Stmt(e1)]));
    // as of the first transaction it also rested on the document statement
    assert_eq!(
        t.col(&q(&format!("asOf/{t1}")), "src"),
        some(&[Value::Stmt(e0), Value::Stmt(e1)])
    );
    // a virtual hop crosses from the supporting statement to its subject
    assert_eq!(
        t.col(
            "SELECT ?who WHERE { v:belief9 v:supportedBy/sys:subject ?who }",
            "who"
        ),
        some(&[v("alice")])
    );
}

// @lat: [[tests#Recipes#Provenance From Transaction Metadata]]
#[test]
fn facts_by_author_and_reasons_for_retraction() {
    let t = T::new();
    t.tx(|tx| {
        tx.meta(Value::iri(vocab::SYS_AUTHOR), v("agent7"))?;
        tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?;
        tx.assert(v("bob"), v("worksAt"), v("acme"), Valid::ALWAYS)?;
        Ok(())
    });
    t.tx(|tx| {
        tx.meta(Value::iri(vocab::SYS_AUTHOR), v("agent9"))?;
        tx.assert(v("carol"), v("worksAt"), v("globex"), Valid::ALWAYS)?;
        Ok(())
    });
    let by7 = t.col(
        "SELECT ?who WHERE { ?who v:worksAt ?c ~ ?r . ?r tm:txAdded ?t . \
         ?t sys:author v:agent7 } ORDER BY ?who",
        "who",
    );
    assert_eq!(by7, some(&[v("alice"), v("bob")]));

    t.tx(|tx| {
        tx.meta(Value::iri(vocab::SYS_REASON), s("user correction"))?;
        let bob = tx.encode(v("bob"))?;
        tx.retract_matching(Some(bob), None, None)?;
        Ok(())
    });
    let why = t.sel(
        "SELECT ?who ?why FROM <urn:tiramemsu:tm:history> WHERE { \
         ?who v:worksAt ?c ~ ?r . ?r tm:txRetracted ?t . ?t sys:reason ?why }",
    );
    assert_eq!(
        why.rows,
        vec![vec![Some(v("bob")), Some(s("user correction"))]]
    );
}

// @lat: [[tests#Recipes#Edit Lineage Follows Supersedes]]
#[test]
fn lineage_of_a_corrected_fact() {
    let t = T::new();
    let mut e1 = None;
    t.tx(|tx| {
        e1 = Some(
            tx.assert(v("alice"), v("age"), int(30), Valid::ALWAYS)?
                .eid(),
        );
        Ok(())
    });
    let e1 = e1.unwrap();
    let mut e2 = None;
    t.tx(|tx| {
        e2 = Some(tx.supersede(
            e1,
            Patch {
                o: Some(int(31)),
                ..Patch::default()
            },
        )?);
        Ok(())
    });
    let e2 = e2.unwrap();
    let mut e3 = None;
    t.tx(|tx| {
        e3 = Some(tx.supersede(
            e2,
            Patch {
                v_from: Some(Some(ms("2025-01-01T00:00:00Z"))),
                ..Patch::default()
            },
        )?);
        Ok(())
    });
    let e3 = e3.unwrap();
    let lineage = t.col(
        "SELECT ?old WHERE { v:alice v:age ?a ~ ?r . ?r sys:supersedes+ ?old } ORDER BY ?old",
        "old",
    );
    assert_eq!(lineage, some(&[Value::Stmt(e1), Value::Stmt(e2)]));
    // the current fact is the head of the chain
    let head = t.col("SELECT ?r WHERE { v:alice v:age ?a ~ ?r }", "r");
    assert_eq!(head, some(&[Value::Stmt(e3)]));
    // and the chain reads back from the head by its skolem IRI too
    let q = format!(
        "SELECT ?old WHERE {{ {} sys:supersedes+ ?old }} ORDER BY ?old",
        stmt_iri(e3)
    );
    assert_eq!(t.col(&q, "old"), some(&[Value::Stmt(e1), Value::Stmt(e2)]));
}

// @lat: [[tests#Recipes#Contradictions Between Sources]]
#[test]
fn contradictions_between_authors_with_overlapping_valid_time() {
    let t = T::new();
    let span = |a: &str, b: Option<&str>| Valid {
        from: Some(ms(a)),
        to: b.map(ms),
    };
    t.tx(|tx| {
        tx.meta(Value::iri(vocab::SYS_AUTHOR), v("agent7"))?;
        tx.assert(
            v("alice"),
            v("worksAt"),
            v("acme"),
            span("2020-01-01T00:00:00Z", Some("2023-01-01T00:00:00Z")),
        )?;
        Ok(())
    });
    t.tx(|tx| {
        tx.meta(Value::iri(vocab::SYS_AUTHOR), v("agent9"))?;
        // overlaps acme in 2022: a contradiction
        tx.assert(
            v("alice"),
            v("worksAt"),
            v("globex"),
            span("2022-01-01T00:00:00Z", Some("2024-01-01T00:00:00Z")),
        )?;
        // a later episode that overlaps neither: not a contradiction
        tx.assert(
            v("alice"),
            v("worksAt"),
            v("initech"),
            span("2024-01-01T00:00:00Z", None),
        )?;
        Ok(())
    });
    let rows = t.sel(
        "SELECT ?s ?o1 ?o2 ?a1 ?a2 WHERE { \
           ?s v:worksAt ?o1 ~ ?r1 . ?s v:worksAt ?o2 ~ ?r2 . \
           FILTER(STR(?o1) < STR(?o2)) \
           ?r1 tm:txAdded ?t1 . ?t1 sys:author ?a1 . \
           ?r2 tm:txAdded ?t2 . ?t2 sys:author ?a2 . \
           FILTER(?a1 != ?a2) \
           OPTIONAL { ?r1 tm:validFrom ?f1 } OPTIONAL { ?r1 tm:validTo ?u1 } \
           OPTIONAL { ?r2 tm:validFrom ?f2 } OPTIONAL { ?r2 tm:validTo ?u2 } \
           FILTER((!BOUND(?f1) || !BOUND(?u2) || ?f1 < ?u2) && \
                  (!BOUND(?f2) || !BOUND(?u1) || ?f2 < ?u1)) }",
    );
    assert_eq!(
        rows.rows,
        vec![vec![
            Some(v("alice")),
            Some(v("acme")),
            Some(v("globex")),
            Some(v("agent7")),
            Some(v("agent9")),
        ]]
    );
}
