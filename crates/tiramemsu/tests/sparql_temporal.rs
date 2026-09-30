//! `sparql-temporal-dataset`: time IRIs in `FROM`, default views and valid-time
//! filtering (`lat.md/query#Temporal Syntax`).
mod sparql_common;
use sparql_common::*;
use tiramemsu::*;

const P: &str = "urn:tiramemsu:tm:";

fn ms(text: &str) -> i64 {
    value::parse_datetime(text).unwrap().0
}

/// alice works at acme from tx 3, retracted in tx 7.
fn acme_3_7() -> T {
    let t = T::new();
    t.advance_to(2);
    t.assert(&[(v("alice"), v("worksAt"), v("acme"))]);
    t.advance_to(6);
    t.retract(&v("alice"), &v("worksAt"), &v("acme"));
    t
}

// sparql-temporal-dataset "Time IRI grammar": As of transaction number
#[test]
fn as_of_transaction_number() {
    let t = acme_3_7();
    let q = format!("SELECT ?c FROM <{P}asOf/5> WHERE {{ v:alice v:worksAt ?c }}");
    assert_eq!(t.col(&q, "c"), some(&[v("acme")]));
    let now = t.col("SELECT ?c WHERE { v:alice v:worksAt ?c }", "c");
    assert!(now.is_empty());
}

// sparql-temporal-dataset "Time IRI grammar": As of wall-clock instant
#[test]
fn as_of_wall_clock_instant() {
    let t = T::new();
    t.clock.set(ms("2026-09-01T10:00:00Z"));
    t.advance_to(3);
    assert_eq!(t.last_t(), 3);
    t.assert(&[(v("alice"), v("worksAt"), v("acme"))]);
    t.clock.set(ms("2026-09-01T13:00:00Z"));
    t.assert(&[(v("alice"), v("worksAt"), v("initech"))]);
    let q =
        format!("SELECT ?c FROM <{P}asOf/2026-09-01T12:00:00Z> WHERE {{ v:alice v:worksAt ?c }}");
    assert_eq!(t.col(&q, "c"), some(&[v("acme")]));
    // the state as of the later instant sees both
    let q = format!("SELECT ?c FROM <{P}asOf/2026-09-01T13:00:00Z> WHERE {{ v:alice v:worksAt ?c }} ORDER BY ?c");
    assert_eq!(t.col(&q, "c"), some(&[v("acme"), v("initech")]));
}

// sparql-temporal-dataset "Time IRI grammar": Instant before the first transaction
#[test]
fn instant_before_the_first_transaction() {
    let t = T::new();
    t.clock.set(ms("2026-01-01T00:00:00Z"));
    t.assert(&[(v("alice"), v("worksAt"), v("acme"))]);
    let q = format!("SELECT * FROM <{P}asOf/1970-01-02> WHERE {{ ?s ?p ?o }}");
    assert!(t.sel(&q).rows.is_empty());
}

// sparql-temporal-dataset "Time IRI grammar": Time IRI offset resolves to an instant
#[test]
fn offset_resolves_to_the_same_transaction() {
    let t = T::new();
    t.clock.set(ms("2026-09-01T10:00:00Z"));
    t.assert(&[(v("a"), v("p"), v("one"))]);
    t.clock.set(ms("2026-09-01T13:00:00Z"));
    t.assert(&[(v("a"), v("p"), v("two"))]);
    let plus2 =
        format!("SELECT ?o FROM <{P}asOf/2026-09-01T14:00:00+02:00> WHERE {{ v:a v:p ?o }}");
    let utc = format!("SELECT ?o FROM <{P}asOf/2026-09-01T12:00:00Z> WHERE {{ v:a v:p ?o }}");
    assert_eq!(t.sel(&plus2).rows, t.sel(&utc).rows);
    assert_eq!(t.col(&utc, "o"), some(&[v("one")]));
}

// sparql-temporal-dataset "Time IRI grammar": Malformed time IRI
#[test]
fn malformed_time_iri_is_a_parse_error() {
    let t = T::new();
    let e = t.err(&format!(
        "SELECT * FROM <{P}asOf/yesterday> WHERE {{ ?s ?p ?o }}"
    ));
    let (_, msg) = assert_parse(e);
    assert!(msg.contains(&format!("{P}asOf/yesterday")), "{msg}");
}

// sparql-temporal-dataset "Default view without time clauses": Past episodes are visible by default
#[test]
fn past_episodes_are_visible_by_default() {
    let t = T::new();
    let (y2020, y2022) = (ms("2020-01-01T00:00:00Z"), ms("2022-01-01T00:00:00Z"));
    t.tx(|tx| {
        tx.assert(
            v("alice"),
            v("worksAt"),
            v("acme"),
            Valid::between(y2020, y2022),
        )?;
        Ok(())
    });
    assert_eq!(
        t.col("SELECT ?c WHERE { v:alice v:worksAt ?c }", "c"),
        some(&[v("acme")])
    );
}

// sparql-temporal-dataset "FROM sets the query default view": Combining asOf and validAt
#[test]
fn combining_as_of_and_valid_at() {
    let t = T::new();
    let (a, b) = (ms("2025-01-01T00:00:00Z"), ms("2026-01-01T00:00:00Z"));
    t.tx(|tx| {
        // valid on 2025-03-01, live from tx 1
        tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::between(a, b))?;
        // valid on 2025-03-01 too, but only added in tx 2
        Ok(())
    });
    t.tx(|tx| {
        tx.assert(v("alice"), v("worksAt"), v("late"), Valid::between(a, b))?;
        tx.assert(v("alice"), v("worksAt"), v("other"), Valid::from(b))?;
        Ok(())
    });
    let q = format!(
        "SELECT ?c FROM <{P}asOf/1> FROM <{P}validAt/2025-03-01> WHERE {{ v:alice v:worksAt ?c }}"
    );
    assert_eq!(t.col(&q, "c"), some(&[v("acme")]));
    let q = format!(
        "SELECT ?c FROM <{P}validAt/2025-03-01> WHERE {{ v:alice v:worksAt ?c }} ORDER BY ?c"
    );
    assert_eq!(t.col(&q, "c"), some(&[v("acme"), v("late")]));
}

// sparql-temporal-dataset "FROM sets the query default view": FROM overrides the API view's transaction part only
#[test]
fn from_overrides_only_the_transaction_part() {
    let t = T::new();
    let (a, b) = (ms("2025-01-01T00:00:00Z"), ms("2026-01-01T00:00:00Z"));
    t.tx(|tx| {
        tx.assert(v("x"), v("p"), v("early"), Valid::between(a, b))?;
        tx.assert(v("x"), v("p"), v("never"), Valid::from(b))?;
        Ok(())
    });
    t.tx(|tx| {
        tx.assert(v("x"), v("p"), v("late"), Valid::between(a, b))?;
        Ok(())
    });
    // API view: as of tx 2, valid at 2025-01-01; the query says asOf/1
    let view = t.db.as_of(TimeRef::Tx(2)).valid_at(a);
    let q = format!("SELECT ?o FROM <{P}asOf/1> WHERE {{ v:x v:p ?o }}");
    assert_eq!(sel_on(&view, &q).rows, vec![vec![Some(v("early"))]]);
    // without FROM the API view applies unchanged
    let q = "SELECT ?o WHERE { v:x v:p ?o } ORDER BY ?o";
    assert_eq!(
        sel_on(&view, q).rows,
        vec![vec![Some(v("early"))], vec![Some(v("late"))]]
    );
}

// sparql-temporal-dataset "FROM sets the query default view": Conflicting selectors rejected
#[test]
fn conflicting_selectors_rejected() {
    let t = T::new();
    let e = t.err(&format!(
        "SELECT * FROM <{P}asOf/5> FROM <{P}history> WHERE {{ ?s ?p ?o }}"
    ));
    assert_unsupported(e, "conflicting time selectors");
}

// sparql-temporal-dataset "FROM sets the query default view": FROM applies inside subqueries and EXISTS
#[test]
fn from_applies_inside_exists_and_subqueries() {
    let t = T::new();
    t.assert(&[(v("alice"), v("a"), v("Person"))]); // tx 1
    t.assert(&[(v("alice"), v("worksAt"), v("acme"))]); // tx 2
    t.assert(&[(v("bob"), v("a"), v("Person"))]); // tx 3
    let q = format!(
        "SELECT ?p FROM <{P}asOf/1> WHERE {{ ?p v:a v:Person FILTER EXISTS {{ ?p v:worksAt v:acme }} }}"
    );
    // as of tx 1 alice has no employer: the EXISTS pattern reads as of tx 1 too
    assert!(t.sel(&q).rows.is_empty());
    let q = format!(
        "SELECT ?p FROM <{P}asOf/2> WHERE {{ ?p v:a v:Person FILTER EXISTS {{ ?p v:worksAt v:acme }} }}"
    );
    assert_eq!(t.col(&q, "p"), some(&[v("alice")]));
    // and inside a subquery
    let q = format!(
        "SELECT ?p FROM <{P}asOf/1> WHERE {{ {{ SELECT ?p WHERE {{ ?p v:a v:Person }} }} }}"
    );
    assert_eq!(t.col(&q, "p"), some(&[v("alice")]));
}

// sparql-temporal-dataset "Valid-time filtering": Half-open interval end
#[test]
fn half_open_interval_end() {
    let t = T::new();
    t.tx(|tx| {
        tx.assert(
            v("alice"),
            v("worksAt"),
            v("acme"),
            Valid::between(ms("2025-01-01T00:00:00Z"), ms("2026-03-01T00:00:00Z")),
        )?;
        Ok(())
    });
    assert!(!t.ask(&format!(
        "ASK FROM <{P}validAt/2026-03-01> {{ v:alice v:worksAt v:acme }}"
    )));
    assert!(t.ask(&format!(
        "ASK FROM <{P}validAt/2026-02-28> {{ v:alice v:worksAt v:acme }}"
    )));
}

// sparql-temporal-dataset "Valid-time filtering": Unbounded statements always valid
#[test]
fn unbounded_statements_are_always_valid() {
    let t = T::new();
    t.assert(&[(v("alice"), v("name"), s("Alice"))]);
    assert!(t.ask(&format!(
        "ASK FROM <{P}validAt/1900-01-01> {{ v:alice v:name \"Alice\" }}"
    )));
}

// sparql-temporal-dataset "Time IRIs are plain IRIs elsewhere": Time IRI as data
#[test]
fn time_iri_as_data() {
    let t = T::new();
    let iri = Value::iri(format!("{P}asOf/150"));
    t.assert(&[(v("doc"), v("ref"), iri.clone())]);
    let q = format!("SELECT ?x WHERE {{ v:doc v:ref ?x FILTER(?x = <{P}asOf/150>) }}");
    assert_eq!(t.col(&q, "x"), some(&[iri]));
}

// named-graphs "Dataset clauses": a non-time FROM IRI names a graph, it is not an error
#[test]
fn ordinary_graph_iri_names_a_graph() {
    let t = T::new();
    let s = t.sel("SELECT * FROM <http://example.org/graph1> WHERE { ?s ?p ?o }");
    assert!(s.rows.is_empty());
}
