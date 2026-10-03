//! Layer recipes (`lat.md/recipes.md`): questions answered by combining statement
//! ids, layers, transaction metadata, paths and time scopes, with no new feature.
#![cfg(all(feature = "sparql", feature = "cypher"))]
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

// @lat: [[tests#Recipes#Metagraph Containers Nesting And Fold]]
#[test]
fn metagraph_containers_nesting_and_fold() {
    let t = T::new();
    // nested metavertices: team graphs inside payments inside org, team C outside
    t.upd(
        "INSERT DATA { \
           GRAPH v:teamA { v:alice v:owns v:billing } \
           GRAPH v:teamB { v:bob v:owns v:search } \
           GRAPH v:teamC { v:carol v:owns v:ads } \
           v:teamA v:within v:payments . v:payments v:within v:org . v:teamB v:within v:org }",
    );
    // a cycle does not change the answer
    t.upd("INSERT DATA { v:org v:within v:payments }");
    let deep =
        "SELECT ?who WHERE { ?g v:within* v:org . GRAPH ?g { ?who v:owns ?svc } } ORDER BY ?who";
    assert_eq!(t.col(deep, "who"), some(&[v("alice"), v("bob")]));
    // Cypher reaches the same memberships through the dual view; a variable-length
    // match gives one row per path, so the cycle needs DISTINCT
    let r = t
        .db
        .now()
        .cypher(
            "MATCH (a)-[r:owns]->(b), (r)-[:`sys:inGraph`]->(g)-[:within*0..]->(top {`@id`: 'v:org'}) RETURN DISTINCT a, g",
            &CypherParams::default(),
        )
        .unwrap();
    assert_eq!(r.rows.len(), 2, "{:?}", r.rows);
    // nesting is a statement, so it has both clocks
    let before = t.last_t();
    t.upd("DELETE DATA { v:teamB v:within v:org }");
    assert_eq!(t.col(deep, "who"), some(&[v("alice")]));
    let then = format!(
        "SELECT ?who WHERE {{ SERVICE <urn:tiramemsu:tm:asOf/{before}> {{ \
           ?g v:within* v:org . GRAPH ?g {{ ?who v:owns ?svc }} }} }} ORDER BY ?who"
    );
    assert_eq!(t.col(&then, "who"), some(&[v("alice"), v("bob")]));

    // fold is a write of tags: no statement is copied
    t.upd(
        "INSERT { GRAPH v:episode1 { ?s v:owns ?o } . v:episode1 v:summarizes \"who owns what\" } \
         WHERE { ?s v:owns ?o }",
    );
    assert_eq!(t.sel("SELECT ?s WHERE { ?s v:owns ?o }").rows.len(), 3);
    // unfold is a read
    assert_eq!(
        t.sel("SELECT ?s ?p ?o WHERE { GRAPH v:episode1 { ?s ?p ?o } }")
            .rows
            .len(),
        3
    );

    // an edge holds a subgraph, and keeps it through a correction
    let r = t.upd("INSERT DATA { v:p7 v:enrolledIn v:trial3 }");
    let edge = r.asserted[0];
    t.upd(&format!(
        "INSERT DATA {{ GRAPH {} {{ v:drSmith v:role v:investigator . v:site9 v:hosts v:p7 }} }}",
        stmt_iri(edge)
    ));
    let inside =
        "SELECT ?s WHERE { ?p v:enrolledIn ?trial ~ ?e . GRAPH ?e { ?s ?q ?o } } ORDER BY ?s";
    assert_eq!(t.col(inside, "s"), some(&[v("drSmith"), v("site9")]));
    t.tx(|tx| {
        tx.supersede(edge, Patch::object(v("trial4")))?;
        Ok(())
    });
    assert_eq!(t.col(inside, "s"), some(&[v("drSmith"), v("site9")]));
    assert!(t.ask(
        "ASK { v:p7 v:enrolledIn v:trial4 ~ ?e . GRAPH ?e { v:drSmith v:role v:investigator } }"
    ));
    // retracting the edge empties the container and keeps the facts
    t.upd("DELETE DATA { v:p7 v:enrolledIn v:trial4 }");
    assert!(t.col(inside, "s").is_empty());
    assert!(t.has(&v("drSmith"), &v("role"), &v("investigator")));

    // n-ary: the meeting is the edge, its extra participants are role statements
    t.upd("INSERT DATA { v:alice v:meets v:bob {| v:with v:carol ; v:with v:dave |} }");
    assert_eq!(
        t.col(
            "SELECT ?x WHERE { ?a v:meets ?b ~ ?e . ?e v:with ?x } ORDER BY ?x",
            "x"
        ),
        some(&[v("carol"), v("dave")])
    );
}
