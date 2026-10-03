//! `sparql-rdf12-annotations`, query side: reifiers, annotation syntax, nesting,
//! triple terms, the virtual `rdf:reifies` and views (task 6.4).
#![cfg(feature = "sparql")]
mod sparql_common;
use sparql_common::*;
use tiramemsu::*;

fn dec(text: &str) -> Value {
    Value::Decimal(text.to_string())
}

/// e1 = (alice worksAt acme) with (e1 confidence 0.8) and (e1 source crawler).
fn fixture() -> (T, Eid) {
    let t = T::new();
    let mut e1 = None;
    t.tx(|tx| {
        let e = tx
            .assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?
            .eid();
        tx.assert(e, v("confidence"), dec("0.8"), Valid::ALWAYS)?;
        tx.assert(e, v("source"), v("crawler"), Valid::ALWAYS)?;
        e1 = Some(e);
        Ok(())
    });
    (t, e1.unwrap())
}

// sparql-rdf12-annotations "Reifiers bind statement eids": Reifier binds the eid
#[test]
fn reifier_binds_the_eid() {
    let (t, e1) = fixture();
    let got = t.col("SELECT ?r WHERE { v:alice v:worksAt v:acme ~ ?r }", "r");
    assert_eq!(got, vec![Some(Value::Stmt(e1))]);
}

// sparql-rdf12-annotations "Reifiers bind statement eids": Explicit rdf:reifies form is equivalent
#[test]
fn explicit_rdf_reifies_form_is_equivalent() {
    let (t, e1) = fixture();
    let got = t.col(
        "SELECT ?r WHERE { ?r rdf:reifies <<( v:alice v:worksAt v:acme )>> }",
        "r",
    );
    assert_eq!(got, vec![Some(Value::Stmt(e1))]);
}

// sparql-rdf12-annotations "Reifiers bind statement eids": One row per episode
#[test]
fn one_row_per_episode() {
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
    let mut got = t.col("SELECT ?r WHERE { v:alice v:worksAt ?c ~ ?r }", "r");
    got.sort_by_key(|x| format!("{x:?}"));
    let mut want: Vec<Option<Value>> = eids.into_iter().map(|e| Some(Value::Stmt(e))).collect();
    want.sort_by_key(|x| format!("{x:?}"));
    assert_eq!(got, want);
}

// sparql-rdf12-annotations "Reifiers bind statement eids": Reified triple as subject
#[test]
fn reified_triple_as_subject() {
    let (t, _) = fixture();
    let got = t.col(
        "SELECT ?c WHERE { << v:alice v:worksAt v:acme >> v:confidence ?c }",
        "c",
    );
    assert_eq!(got, some(&[dec("0.8")]));
}

// sparql-rdf12-annotations "Reifiers bind statement eids": Reifier constant that is not a statement
#[test]
fn reifier_constant_that_is_not_a_statement() {
    let (t, _) = fixture();
    assert!(!t.ask("ASK { v:alice v:worksAt v:acme ~ v:someIri }"));
}

// sparql-rdf12-annotations "Annotation syntax matches layer triples": Annotation value is read
#[test]
fn annotation_value_is_read() {
    let (t, _) = fixture();
    let r = t
        .sel("SELECT ?c ?s WHERE { v:alice v:worksAt v:acme {| v:confidence ?c ; v:source ?s |} }");
    assert_eq!(r.rows, vec![vec![Some(dec("0.8")), Some(v("crawler"))]]);
}

// sparql-rdf12-annotations "Annotation syntax matches layer triples": Missing annotation removes the solution
#[test]
fn missing_annotation_removes_the_solution() {
    let (t, _) = fixture();
    t.assert(&[(v("bob"), v("worksAt"), v("acme"))]);
    let got = t.col(
        "SELECT ?p WHERE { ?p v:worksAt v:acme {| v:confidence ?c |} }",
        "p",
    );
    assert_eq!(got, some(&[v("alice")]));
}

// sparql-rdf12-annotations "Annotation syntax matches layer triples": Optional annotation
#[test]
fn optional_annotation() {
    let (t, _) = fixture();
    t.assert(&[(v("bob"), v("worksAt"), v("acme"))]);
    let r = t.sel(
        "SELECT ?p ?c WHERE { ?p v:worksAt v:acme ~ ?r OPTIONAL { ?r v:confidence ?c } } ORDER BY ?p",
    );
    assert_eq!(
        r.rows,
        vec![
            vec![Some(v("alice")), Some(dec("0.8"))],
            vec![Some(v("bob")), None]
        ]
    );
}

// sparql-rdf12-annotations "Nested layers": Annotation on an annotation
#[test]
fn annotation_on_an_annotation() {
    let t = T::new();
    t.tx(|tx| {
        let e1 = tx
            .assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?
            .eid();
        let e2 = tx
            .assert(e1, v("confidence"), dec("0.8"), Valid::ALWAYS)?
            .eid();
        tx.assert(e2, v("method"), s("llm-extraction"), Valid::ALWAYS)?;
        Ok(())
    });
    let r = t.sel(
        "SELECT ?c ?m WHERE { v:alice v:worksAt v:acme {| v:confidence ?c ~ ?r2 {| v:method ?m |} |} }",
    );
    assert_eq!(
        r.rows,
        vec![vec![Some(dec("0.8")), Some(s("llm-extraction"))]]
    );
}

// sparql-rdf12-annotations "Nested layers": Belief referencing a fact
#[test]
fn belief_referencing_a_fact() {
    let (t, e1) = fixture();
    t.tx(|tx| {
        tx.assert(v("belief9"), v("supportedBy"), e1, Valid::ALWAYS)?;
        Ok(())
    });
    let got = t.col(
        "SELECT ?b WHERE { ?b v:supportedBy ?r . ?r rdf:reifies <<( v:alice v:worksAt ?c )>> }",
        "b",
    );
    assert_eq!(got, some(&[v("belief9")]));
}

// sparql-rdf12-annotations "Nested layers": Nested triple term
#[test]
fn nested_triple_term() {
    let (t, e1) = fixture();
    t.tx(|tx| {
        let e5 = tx.assert(v("bob"), v("says"), e1, Valid::ALWAYS)?.eid();
        tx.assert(v("carol"), v("doubts"), e5, Valid::ALWAYS)?;
        Ok(())
    });
    let got = t.col(
        "SELECT ?who WHERE { ?who v:doubts <<( v:bob v:says <<( v:alice v:worksAt v:acme )>> )>> }",
        "who",
    );
    assert_eq!(got, some(&[v("carol")]));
}

// sparql-rdf12-annotations "Triple terms denote statements": Triple term in object position
#[test]
fn triple_term_in_object_position() {
    let (t, e1) = fixture();
    t.tx(|tx| {
        tx.assert(v("belief9"), v("supportedBy"), e1, Valid::ALWAYS)?;
        Ok(())
    });
    assert!(t.ask("ASK { v:belief9 v:supportedBy <<( v:alice v:worksAt v:acme )>> }"));
    assert!(!t.ask("ASK { v:belief9 v:supportedBy <<( v:alice v:worksAt v:nowhere )>> }"));
}

// sparql-rdf12-annotations "rdf:reifies is virtual": Variable predicate does not see rdf:reifies
#[test]
fn variable_predicate_does_not_see_rdf_reifies() {
    let t = T::new();
    t.assert(&[(v("alice"), v("worksAt"), v("acme"))]);
    assert_eq!(
        t.col("SELECT ?p WHERE { ?s ?p ?o }", "p"),
        some(&[v("worksAt")])
    );
}

// sparql-rdf12-annotations "rdf:reifies is virtual": rdf:reifies with variable object
#[test]
fn rdf_reifies_with_variable_object() {
    let t = T::new();
    let e = t.err("SELECT ?t WHERE { ?r rdf:reifies ?t }");
    assert_unsupported(e, "rdf:reifies without triple term");
    let e = t.err("SELECT ?t WHERE { ?s ?p <<( v:a v:b v:c )>> }");
    assert_unsupported(e, "variable predicate with triple term");
}

// sparql-rdf12-annotations "rdf:reifies is virtual": Triple function unsupported
#[test]
fn triple_function_unsupported() {
    let t = T::new();
    let e = t.err("SELECT ?x WHERE { ?x v:about ?o FILTER(isTRIPLE(?o)) }");
    assert_unsupported(e, "isTRIPLE");
}

// sparql-rdf12-annotations "Reifiers follow the pattern's view": History shows a superseded fact and its replacement
#[test]
fn history_shows_a_superseded_fact_and_its_replacement() {
    let (t, e1) = fixture();
    let mut e10 = None;
    t.tx(|tx| {
        e10 = Some(tx.supersede(
            e1,
            Patch {
                v_from: Some(Some(1_000_000)),
                ..Patch::default()
            },
        )?);
        Ok(())
    });
    let e10 = e10.unwrap();
    let hist = t.col(
        "SELECT ?r FROM <urn:tiramemsu:tm:history> WHERE { v:alice v:worksAt v:acme ~ ?r }",
        "r",
    );
    let mut got: Vec<String> = hist.iter().map(|x| format!("{x:?}")).collect();
    got.sort();
    let mut want = vec![
        format!("{:?}", Some(Value::Stmt(e1))),
        format!("{:?}", Some(Value::Stmt(e10))),
    ];
    want.sort();
    assert_eq!(got, want);
    assert_eq!(
        t.col("SELECT ?r WHERE { v:alice v:worksAt v:acme ~ ?r }", "r"),
        vec![Some(Value::Stmt(e10))]
    );
}

// sparql-rdf12-annotations "Reifiers follow the pattern's view": Cascaded annotation visible as of before retraction
#[test]
fn cascaded_annotation_visible_as_of_before_retraction() {
    let (t, _) = fixture(); // tx 1
    t.advance_to(8);
    t.retract(&v("alice"), &v("worksAt"), &v("acme")); // tx 9 retracts e1 and cascades
    let q = "SELECT ?c WHERE { v:alice v:worksAt v:acme {| v:confidence ?c |} }";
    assert!(t.sel(q).rows.is_empty());
    let r = sel_on(&t.db.as_of(TimeRef::Tx(8)), q);
    assert_eq!(r.rows, vec![vec![Some(dec("0.8"))]]);
}

// sparql-rdf12-annotations "Same eid across dialects and the API": API eid equals SPARQL reifier
// @lat: [[tests#Query#Dual View Binds Same Eid]]
#[test]
fn api_eid_equals_sparql_reifier() {
    let t = T::new();
    let mut e = None;
    t.tx(|tx| {
        e = Some(
            tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?
                .eid(),
        );
        Ok(())
    });
    let e = e.unwrap();
    t.tx(|tx| {
        tx.assert(e, v("confidence"), dec("0.8"), Valid::ALWAYS)?;
        Ok(())
    });
    let got = t.col("SELECT ?r WHERE { v:alice v:worksAt v:acme ~ ?r }", "r");
    assert_eq!(got, vec![Some(Value::Stmt(e))]);
    let q = format!(
        "SELECT ?c WHERE {{ <urn:tiramemsu:stmt:{}> v:confidence ?c }}",
        e.n()
    );
    assert_eq!(t.col(&q, "c"), some(&[dec("0.8")]));
}
