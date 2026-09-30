//! `sparql-temporal-dataset`: per-group time scopes with `SERVICE`, the `GRAPH`
//! rejections and the statement-time virtual predicates.
mod sparql_common;
use sparql_common::*;
use tiramemsu::*;

const P: &str = "urn:tiramemsu:tm:";

fn ms(text: &str) -> i64 {
    value::parse_datetime(text).unwrap().0
}

/// tx 1 asserts alice worksAt acme; tx 2 supersedes it by initech; tx 3 is empty.
fn changed_fact() -> T {
    let t = T::new();
    t.assert(&[(v("alice"), v("worksAt"), v("acme"))]);
    t.tx(|tx| {
        let old = tx.encode(v("acme"))?;
        let (a, p) = (tx.encode(v("alice"))?, tx.encode(v("worksAt"))?);
        tx.retract_matching(Some(a), Some(p), Some(old))?;
        tx.assert(v("alice"), v("worksAt"), v("initech"), Valid::ALWAYS)?;
        Ok(())
    });
    t.advance_to(3);
    t
}

// sparql-temporal-dataset "SERVICE scopes a group in time": Before and after values of a changed fact
// @lat: [[tests#Query#Per Pattern Time Scopes]]
#[test]
fn before_and_after_values_of_a_changed_fact() {
    let t = changed_fact();
    let q = format!(
        "SELECT ?before ?after WHERE {{ SERVICE <{P}asOf/1> {{ v:alice v:worksAt ?before }} \
         v:alice v:worksAt ?after . FILTER(?before != ?after) }}"
    );
    let s = t.sel(&q);
    assert_eq!(s.rows, vec![vec![Some(v("acme")), Some(v("initech"))]]);
    // the same scope written with GRAPH is a Parse error that names SERVICE
    let g = format!(
        "SELECT ?before ?after WHERE {{ GRAPH <{P}asOf/1> {{ v:alice v:worksAt ?before }} \
         v:alice v:worksAt ?after . FILTER(?before != ?after) }}"
    );
    let (_, msg) = assert_parse(t.err(&g));
    assert!(msg.contains("SERVICE"), "{msg}");
}

// sparql-temporal-dataset "SERVICE scopes a group in time": Nested scopes combine parts
#[test]
fn nested_scopes_combine_parts() {
    let t = T::new();
    let (a, b) = (ms("2021-01-01T00:00:00Z"), ms("2022-01-01T00:00:00Z"));
    t.tx(|tx| {
        tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::between(a, b))?;
        tx.assert(v("alice"), v("worksAt"), v("elsewhere"), Valid::from(b))?;
        Ok(())
    });
    t.tx(|tx| {
        tx.assert(v("alice"), v("worksAt"), v("late"), Valid::between(a, b))?;
        Ok(())
    });
    let q = format!(
        "SELECT ?c WHERE {{ SERVICE <{P}asOf/1> {{ SERVICE <{P}validAt/2021-06-01> {{ v:alice v:worksAt ?c }} }} }}"
    );
    assert_eq!(t.col(&q, "c"), some(&[v("acme")]));
}

// sparql-temporal-dataset "SERVICE scopes a group in time": Inner scope overrides outer scope for the same part
#[test]
fn inner_scope_overrides_outer_for_the_same_part() {
    let t = changed_fact();
    let q = format!(
        "SELECT ?c WHERE {{ SERVICE <{P}asOf/3> {{ SERVICE <{P}asOf/1> {{ v:alice v:worksAt ?c }} }} }}"
    );
    assert_eq!(t.col(&q, "c"), some(&[v("acme")]));
}

// sparql-temporal-dataset "SERVICE scopes a group in time": Inner scope overrides FROM
#[test]
fn inner_scope_overrides_from() {
    let t = changed_fact();
    let q = format!(
        "SELECT ?c FROM <{P}asOf/1> WHERE {{ SERVICE <{P}history> {{ v:alice v:worksAt ?c }} }} ORDER BY ?c"
    );
    assert_eq!(t.col(&q, "c"), some(&[v("acme"), v("initech")]));
}

// sparql-temporal-dataset "SERVICE scopes a group in time": History scope for one pattern
#[test]
fn history_scope_for_one_pattern() {
    let t = changed_fact();
    let q = format!(
        "SELECT ?old WHERE {{ v:alice v:worksAt ?now . SERVICE <{P}history> {{ v:alice v:worksAt ?old }} \
         FILTER(?old != ?now) }}"
    );
    assert_eq!(t.col(&q, "old"), some(&[v("acme")]));
}

// sparql-temporal-dataset "SERVICE scopes a group in time": SERVICE SILENT is the same scope
#[test]
fn service_silent_is_the_same_scope() {
    let t = changed_fact();
    let plain = format!("SELECT ?c WHERE {{ SERVICE <{P}asOf/1> {{ v:alice v:worksAt ?c }} }}");
    let silent =
        format!("SELECT ?c WHERE {{ SERVICE SILENT <{P}asOf/1> {{ v:alice v:worksAt ?c }} }}");
    assert_eq!(t.sel(&plain).rows, t.sel(&silent).rows);
    assert_eq!(t.col(&silent, "c"), some(&[v("acme")]));
}

// sparql-temporal-dataset "SERVICE scopes a group in time": Malformed time IRI in SERVICE
#[test]
fn malformed_time_iri_in_service() {
    let t = T::new();
    let e = t.err(&format!(
        "SELECT * WHERE {{ SERVICE SILENT <{P}asOf/yesterday> {{ ?s ?p ?o }} }}"
    ));
    let (_, msg) = assert_parse(e);
    assert!(msg.contains(&format!("{P}asOf/yesterday")), "{msg}");
}

// sparql-temporal-dataset "GRAPH does not carry time": Time IRI in GRAPH is rejected
#[test]
fn time_iri_in_graph_is_rejected() {
    let t = changed_fact();
    let e = t.err(&format!(
        "SELECT ?c WHERE {{ GRAPH <{P}asOf/150> {{ v:alice v:worksAt ?c }} }}"
    ));
    let (_, msg) = assert_parse(e);
    assert!(msg.contains("SERVICE"), "{msg}");
}

// sparql-temporal-dataset "Non-time SERVICE is unsupported": Federated SERVICE rejected
// sparql-query "Unsupported features fail before execution": Federated SERVICE is rejected
#[test]
fn federated_service_rejected() {
    let t = T::new();
    let e = t.err("SELECT * WHERE { SERVICE <http://dbpedia.org/sparql> { ?s ?p ?o } }");
    assert_unsupported(e, "SERVICE");
    let e = t.err("SELECT * WHERE { SERVICE SILENT <http://dbpedia.org/sparql> { ?s ?p ?o } }");
    assert_unsupported(e, "SERVICE");
}

// sparql-temporal-dataset "Non-time SERVICE is unsupported": SERVICE variable rejected
#[test]
fn service_variable_rejected() {
    let t = T::new();
    assert_unsupported(
        t.err("SELECT * WHERE { SERVICE ?ep { ?s ?p ?o } }"),
        "SERVICE",
    );
}

// sparql-temporal-dataset "Non-time graphs are unsupported": GRAPH inside a time scope is still a named graph
#[test]
fn graph_inside_a_time_scope_is_a_named_graph() {
    let t = T::new();
    let e = t.err(&format!(
        "SELECT * WHERE {{ SERVICE <{P}asOf/150> {{ GRAPH <http://example.org/g> {{ ?s ?p ?o }} }} }}"
    ));
    assert_unsupported(e, "named graph");
}

// sparql-query "Unsupported features fail before execution": GRAPH variable is rejected
#[test]
fn graph_variable_rejected() {
    let t = T::new();
    assert_unsupported(
        t.err("SELECT ?g WHERE { GRAPH ?g { ?s ?p ?o } }"),
        "GRAPH variable",
    );
}

// sparql-temporal-dataset "Statement-time virtual predicates": When was a fact added, and by whom
#[test]
fn when_was_a_fact_added_and_by_whom() {
    let t = T::new();
    t.advance_to(41);
    t.tx(|tx| {
        tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?;
        tx.meta(Value::iri(vocab::SYS_AUTHOR), v("agent7"))?;
        Ok(())
    });
    let s = t.sel(
        "SELECT ?t ?a WHERE { v:alice v:worksAt v:acme ~ ?r . ?r tm:txAdded ?t . ?t sys:author ?a }",
    );
    assert_eq!(
        s.rows,
        vec![vec![Some(Value::Tx(TxId(42))), Some(v("agent7"))]]
    );
}

// sparql-temporal-dataset "Statement-time virtual predicates": Live statement has no retraction
#[test]
fn live_statement_has_no_retraction() {
    let t = T::new();
    t.assert(&[(v("alice"), v("worksAt"), v("acme"))]);
    let s = t.sel(
        "SELECT ?r ?x WHERE { v:alice v:worksAt v:acme ~ ?r OPTIONAL { ?r tm:txRetracted ?x } }",
    );
    assert_eq!(s.rows.len(), 1);
    assert!(s.rows[0][0].is_some());
    assert_eq!(s.rows[0][1], None);
}

// sparql-temporal-dataset "Statement-time virtual predicates": Retraction time under history
#[test]
fn retraction_time_under_history() {
    let t = T::new();
    t.assert(&[(v("alice"), v("worksAt"), v("acme"))]);
    t.advance_to(49);
    t.retract(&v("alice"), &v("worksAt"), &v("acme"));
    assert_eq!(t.last_t(), 50);
    let q = format!(
        "SELECT ?x FROM <{P}history> WHERE {{ v:alice v:worksAt v:acme ~ ?r . ?r tm:txRetracted ?x }}"
    );
    assert_eq!(t.col(&q, "x"), some(&[Value::Tx(TxId(50))]));
}

// sparql-temporal-dataset "Statement-time virtual predicates": Valid-from value
#[test]
fn valid_from_value() {
    let t = T::new();
    t.tx(|tx| {
        tx.assert(
            v("alice"),
            v("worksAt"),
            v("acme"),
            Valid::from(ms("2020-01-01T00:00:00Z")),
        )?;
        Ok(())
    });
    let got = t.col(
        "SELECT ?f WHERE { v:alice v:worksAt v:acme ~ ?r . ?r tm:validFrom ?f }",
        "f",
    );
    assert_eq!(got, some(&[dt("2020-01-01T00:00:00Z")]));
}
