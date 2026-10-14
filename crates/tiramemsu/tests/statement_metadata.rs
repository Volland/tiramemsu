//! Valid-time writes from SPARQL Update, and statement ids and transactions read
//! from a triple in SPARQL and Cypher (OpenSpec change `add-query-statement-metadata`).
#![cfg(all(feature = "sparql", feature = "cypher"))]

use tiramemsu::cypher_frontend::{CypherParams, CypherResult, CypherValue};
use tiramemsu::*;

const DAY: i64 = 86_400_000;
/// 2020-01-01T00:00:00Z
const Y2020: i64 = 18_262 * DAY;
/// 2024-01-01T00:00:00Z
const Y2024: i64 = 19_723 * DAY;

fn v(s: &str) -> Value {
    Value::iri(format!("urn:tiramemsu:v:{s}"))
}

fn sys(s: &str) -> Value {
    Value::iri(format!("urn:tiramemsu:sys:{s}"))
}

struct T {
    _dir: tempfile::TempDir,
    db: Db,
}

impl T {
    fn new() -> T {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path().join("m.db"), OpenOptions::default()).unwrap();
        T { _dir: dir, db }
    }

    fn tx(&self, f: impl FnOnce(&mut Tx<'_>) -> Result<()>) -> TxReport {
        self.db.transact(TxOptions::default(), f).unwrap()
    }

    fn upd(&self, q: &str) -> TxReport {
        match self
            .db
            .now()
            .sparql(q)
            .unwrap_or_else(|e| panic!("{q}: {e}"))
        {
            SparqlResult::Update(r) => r,
            other => panic!("not an update: {other:?}"),
        }
    }

    fn err(&self, q: &str) -> Error {
        self.db.now().sparql(q).expect_err(q)
    }

    fn sel(&self, q: &str) -> Solutions {
        match self
            .db
            .now()
            .sparql(q)
            .unwrap_or_else(|e| panic!("{q}: {e}"))
        {
            SparqlResult::Solutions(s) => s,
            other => panic!("not a select: {other:?}"),
        }
    }

    fn cy(&self, q: &str) -> CypherResult {
        self.db
            .now()
            .cypher(q, &CypherParams::new())
            .unwrap_or_else(|e| panic!("{q}: {e}"))
    }

    fn cy_history(&self, q: &str) -> CypherResult {
        self.db
            .history()
            .cypher(q, &CypherParams::new())
            .unwrap_or_else(|e| panic!("{q}: {e}"))
    }

    fn cy_write(&self, q: &str) -> Result<CypherResult> {
        self.db
            .cypher_write(TxOptions::default(), q, &CypherParams::new())
    }

    fn last_t(&self) -> u64 {
        self.db
            .history()
            .triples(None, None, None)
            .unwrap()
            .iter()
            .map(|t| t.t_add.0)
            .max()
            .unwrap_or(0)
    }

    /// The live statements with predicate `p`, as `(eid, v_from, v_to)`.
    fn intervals(&self, p: &str) -> Vec<(Eid, Option<i64>, Option<i64>)> {
        let view = self.db.now();
        let Some(pid) = view.encode(&v(p)).unwrap() else {
            return Vec::new();
        };
        view.triples(None, Some(pid), None)
            .unwrap()
            .iter()
            .map(|t| (t.eid, t.v_from, t.v_to))
            .collect()
    }

    /// Every live statement whose predicate is in the `tm:` namespace.
    fn tm_triples(&self) -> usize {
        let view = self.db.now();
        view.triples(None, None, None)
            .unwrap()
            .iter()
            .filter(|t| {
                matches!(view.decode(t.p).unwrap(), Value::Iri(i) if i.starts_with("urn:tiramemsu:tm:"))
            })
            .count()
    }
}

fn row(r: &CypherResult) -> &[CypherValue] {
    assert_eq!(r.rows.len(), 1, "{:?}", r.rows);
    &r.rows[0]
}

// ---- SPARQL: valid-time writes --------------------------------------------------

// @tg: verifies:: [[tests#Statement Metadata#SPARQL Inserts A Fact With Its Valid Time]] {since: "2026-10-14", level: unit}
#[test]
fn sparql_inserts_a_fact_with_its_valid_time() {
    let t = T::new();
    let r = t.upd(
        r#"INSERT DATA { v:alice v:worksAt v:acme {| tm:validFrom "2020-01-01"^^xsd:date ;
                                                    tm:validTo "2024-01-01T00:00:00Z"^^xsd:dateTime |} }"#,
    );
    assert_eq!(r.asserted.len(), 1);
    assert_eq!(
        t.intervals("worksAt"),
        vec![(r.asserted[0], Some(Y2020), Some(Y2024))]
    );
    assert_eq!(t.tm_triples(), 0, "valid time is never stored as a triple");
    let s = t.sel(
        "SELECT ?f ?t WHERE { v:alice v:worksAt v:acme ~ ?e . ?e tm:validFrom ?f ; tm:validTo ?t }",
    );
    assert_eq!(s.rows.len(), 1);
    // an overlapping re-insert returns the first statement unchanged
    let again = t.upd(
        r#"INSERT DATA { v:alice v:worksAt v:acme {| tm:validFrom "2021-01-01"^^xsd:date |} }"#,
    );
    assert!(again.asserted.is_empty());
    assert_eq!(again.existing, r.asserted);
    assert_eq!(
        t.intervals("worksAt"),
        vec![(r.asserted[0], Some(Y2020), Some(Y2024))]
    );
    // epoch milliseconds
    t.upd("INSERT DATA { v:a v:p v:b {| tm:validFrom 1000 |} }");
    assert_eq!(t.intervals("p")[0].1, Some(1000));
}

// @tg: verifies:: [[tests#Statement Metadata#SPARQL Closes And Clears An Interval]] {since: "2026-10-14", level: unit}
#[test]
fn sparql_closes_and_clears_an_interval() {
    let t = T::new();
    let first = t.tx(|tx| {
        let e = tx
            .assert(v("alice"), v("worksAt"), v("acme"), Valid::from(Y2020))?
            .eid();
        tx.assert(e, v("confidence"), Value::Double(0.8), Valid::ALWAYS)?;
        Ok(())
    });
    let old = first.asserted[0];
    let r = t.upd(
        r#"INSERT { ?e tm:validTo "2024-01-01"^^xsd:date } WHERE { v:alice v:worksAt v:acme ~ ?e }"#,
    );
    // the statement and its annotation are replayed under new eids
    assert_eq!(r.superseded.len(), 2);
    let new = r.superseded.iter().find(|(o, _)| *o == old).unwrap().1;
    assert_eq!(
        t.intervals("worksAt"),
        vec![(new, Some(Y2020), Some(Y2024))]
    );
    // the annotation moved to the new statement
    let s = t.sel("SELECT ?c WHERE { v:alice v:worksAt v:acme {| v:confidence ?c |} }");
    assert_eq!(s.rows.len(), 1);
    // clear the end again
    let r = t.upd(
        "DELETE { ?e tm:validTo ?t } WHERE { v:alice v:worksAt v:acme ~ ?e . ?e tm:validTo ?t }",
    );
    assert!(r.superseded.iter().any(|(o, _)| *o == new));
    let (_, from, to) = t.intervals("worksAt")[0];
    assert_eq!((from, to), (Some(Y2020), None));
    // a delete of a value that is not the current bound is a no-op
    let stmt = format!("<urn:tiramemsu:stmt:{}>", t.intervals("worksAt")[0].0.n());
    let r = t.upd(&format!(
        r#"DELETE DATA {{ {stmt} tm:validFrom "1999-01-01"^^xsd:date }}"#
    ));
    assert!(r.superseded.is_empty());
    // a patch that changes nothing writes nothing
    let r = t.upd(&format!(
        r#"INSERT DATA {{ {stmt} tm:validFrom "2020-01-01"^^xsd:date }}"#
    ));
    assert!(r.superseded.is_empty() && r.asserted.is_empty());
}

// @tg: verifies:: [[tests#Statement Metadata#SPARQL Replaces Both Bounds In One Supersede]] {since: "2026-10-14", level: unit}
#[test]
fn sparql_replaces_both_bounds_in_one_supersede() {
    let t = T::new();
    t.tx(|tx| {
        tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::from(Y2020))?;
        Ok(())
    });
    let r = t.upd(
        r#"DELETE { ?e tm:validFrom ?f }
           INSERT { ?e tm:validFrom "2019-06-01"^^xsd:date ; tm:validTo "2023-01-01"^^xsd:date }
           WHERE  { v:alice v:worksAt v:acme ~ ?e . ?e tm:validFrom ?f }"#,
    );
    assert_eq!(r.superseded.len(), 1);
    let (_, from, to) = t.intervals("worksAt")[0];
    assert_eq!((from, to), (Some(18_048 * DAY), Some(19_358 * DAY)));
}

// @tg: verifies:: [[tests#Statement Metadata#SPARQL Valid Time In A Graph Block]] {since: "2026-10-14", level: unit}
#[test]
fn sparql_valid_time_in_a_graph_block() {
    let t = T::new();
    let r = t.upd("INSERT DATA { GRAPH v:g { v:a v:p v:b {| tm:validFrom 5 |} } }");
    assert_eq!(t.intervals("p")[0].1, Some(5));
    // one membership, for the statement; none for the valid-time triple
    assert_eq!(r.memberships.len(), 1);
    assert_eq!(t.tm_triples(), 0);
}

// @tg: verifies:: [[tests#Statement Metadata#SPARQL Valid Time Errors]] {since: "2026-10-14", level: unit}
#[test]
fn sparql_valid_time_errors() {
    let t = T::new();
    t.tx(|tx| {
        tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::from(Y2020))?;
        Ok(())
    });
    let n = t.last_t();
    let e = t.err(r#"INSERT DATA { v:alice tm:validFrom "2020-01-01"^^xsd:date }"#);
    assert!(
        matches!(e, Error::InvalidQuery { .. }),
        "not a statement: {e:?}"
    );
    let e = t.err(r#"INSERT DATA { v:a v:p v:b {| tm:validFrom "soon" |} }"#);
    assert!(matches!(e, Error::InvalidQuery { .. }), "wrong type: {e:?}");
    let e = t.err(
        r#"INSERT { ?e tm:validTo "2023-01-01"^^xsd:date, "2022-01-01"^^xsd:date } WHERE { v:alice v:worksAt v:acme ~ ?e }"#,
    );
    assert!(matches!(e, Error::InvalidQuery { .. }), "two values: {e:?}");
    let e = t.err(
        r#"INSERT { ?e tm:validTo "2019-01-01"^^xsd:date } WHERE { v:alice v:worksAt v:acme ~ ?e }"#,
    );
    assert!(matches!(e, Error::InvalidPatch { .. }), "inverted: {e:?}");
    let e = t.err(r#"INSERT DATA { v:a v:p v:b {| tm:validFrom 10 ; tm:validTo 5 |} }"#);
    assert!(
        matches!(e, Error::InvalidInterval { .. }),
        "inverted on assert: {e:?}"
    );
    assert_eq!(t.last_t(), n, "nothing was written");
    assert_eq!(t.intervals("worksAt")[0].2, None);
}

// @tg: verifies:: [[tests#Statement Metadata#SPARQL Transaction Time Is Read-Only]] {since: "2026-10-14", level: unit}
#[test]
fn sparql_transaction_time_is_read_only() {
    let t = T::new();
    t.tx(|tx| {
        tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?;
        Ok(())
    });
    for p in [
        "txAdded",
        "txRetracted",
        "addedAt",
        "retractedAt",
        "retractKind",
    ] {
        let e = t.err(&format!(
            "INSERT {{ ?r tm:{p} 1 }} WHERE {{ v:alice v:worksAt v:acme ~ ?r }}"
        ));
        match e {
            Error::Unsupported { feature } => {
                assert!(feature.contains(&format!("tm:{p}")), "{feature}")
            }
            other => panic!("{p}: {other:?}"),
        }
    }
    // hop predicates stay reserved
    let e = t.err("INSERT DATA { v:a sys:subject v:b }");
    assert!(matches!(e, Error::ReservedNamespace(_)), "{e:?}");
}

// ---- SPARQL: statement id and transaction ----------------------------------------

fn authored(t: &T) -> TxReport {
    t.tx(|tx| {
        tx.meta(sys("author"), Value::str("agent-7"))?;
        tx.meta(sys("reason"), Value::str("onboarding"))?;
        tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::from(Y2020))?;
        Ok(())
    })
}

// @tg: verifies:: [[tests#Statement Metadata#SPARQL Reads Statement Id And Transaction]] {since: "2026-10-14", level: unit}
#[test]
fn sparql_reads_statement_id_and_transaction() {
    let t = T::new();
    let r = authored(&t);
    let eid = *r.asserted.last().unwrap();
    let s = t.sel(
        "SELECT ?e ?tx ?a ?why WHERE { v:alice v:worksAt v:acme ~ ?e . ?e tm:txAdded ?tx . ?tx sys:author ?a ; sys:reason ?why }",
    );
    assert_eq!(s.rows.len(), 1);
    let get = |n: &str| s.rows[0][s.col(n).unwrap()].clone();
    assert_eq!(get("e"), Some(Value::Stmt(eid)));
    assert_eq!(get("tx"), Some(Value::Tx(r.t)));
    assert_eq!(get("a"), Some(Value::str("agent-7")));
    assert_eq!(get("why"), Some(Value::str("onboarding")));
    // everything the transaction wrote: its two metadata statements and the fact
    let s = t.sel(&format!(
        "SELECT ?e WHERE {{ ?s ?p ?o ~ ?e . ?e tm:txAdded <urn:tiramemsu:tx:{}> }}",
        r.t.0
    ));
    assert_eq!(s.rows.len(), 3);
}

// ---- Cypher: statement id and transaction nodes -----------------------------------

// @tg: verifies:: [[tests#Statement Metadata#Cypher Statement Id And Transaction Node]] {since: "2026-10-14", level: unit}
#[test]
fn cypher_statement_id_and_transaction_node() {
    let t = T::new();
    let r = authored(&t);
    let eid = *r.asserted.last().unwrap();
    let res = t.cy(
        "MATCH ()-[r:worksAt]->() WITH r, tiramemsu.transaction(r) AS t \
         RETURN tiramemsu.statementId(r), elementId(r), t.number, t.`sys:author`, labels(t), \
                elementId(t), t.instant = r.addedAt, tiramemsu.retraction(r)",
    );
    let x = row(&res);
    assert_eq!(x[0], CypherValue::Integer(eid.n() as i64));
    assert_eq!(
        x[1],
        CypherValue::String(format!("urn:tiramemsu:stmt:{}", eid.n()))
    );
    assert_eq!(x[2], CypherValue::Integer(r.t.0 as i64));
    assert_eq!(x[3], CypherValue::String("agent-7".into()));
    assert_eq!(
        x[4],
        CypherValue::List(vec![CypherValue::String("Transaction".into())])
    );
    assert_eq!(
        x[5],
        CypherValue::String(format!("urn:tiramemsu:tx:{}", r.t.0))
    );
    assert_eq!(x[6], CypherValue::Boolean(true));
    assert_eq!(x[7], CypherValue::Null);
    // a transaction IRI in a map binds the node
    let res = t.cy(&format!(
        "MATCH (t {{`@id`: 'urn:tiramemsu:tx:{}'}}) RETURN t.`sys:reason`",
        r.t.0
    ));
    assert_eq!(row(&res)[0], CypherValue::String("onboarding".into()));
    // null in, null out; a non-statement is an error
    let res = t.cy("RETURN tiramemsu.statementId(null), tiramemsu.transaction(null)");
    assert_eq!(row(&res), &[CypherValue::Null, CypherValue::Null]);
    let e =
        t.db.now()
            .cypher("RETURN tiramemsu.statementId(1)", &CypherParams::new())
            .expect_err("not a statement");
    assert!(
        matches!(e, Error::Eval { .. } | Error::InvalidQuery { .. }),
        "{e:?}"
    );
}

// @tg: verifies:: [[tests#Statement Metadata#Cypher Reads Why A Fact Was Retracted]] {since: "2026-10-14", level: unit}
#[test]
fn cypher_reads_why_a_fact_was_retracted() {
    let t = T::new();
    let r = authored(&t);
    let eid = *r.asserted.last().unwrap();
    t.tx(|tx| {
        tx.meta(sys("reason"), Value::str("left the company"))?;
        tx.retract(eid)?;
        Ok(())
    });
    let res = t.cy_history(
        "MATCH ()-[r:worksAt]->() RETURN tiramemsu.retraction(r).`sys:reason`, tiramemsu.transaction(r).`sys:reason`",
    );
    assert_eq!(
        row(&res),
        &[
            CypherValue::String("left the company".into()),
            CypherValue::String("onboarding".into())
        ]
    );
}

// @tg: verifies:: [[tests#Statement Metadata#Cypher Transaction Nodes Are Read-Only]] {since: "2026-10-14", level: unit}
#[test]
fn cypher_transaction_nodes_are_read_only() {
    let t = T::new();
    authored(&t);
    let n = t.last_t();
    for q in [
        "MATCH ()-[r:worksAt]->() WITH tiramemsu.transaction(r) AS t SET t.`sys:author` = 'me'",
        "MATCH ()-[r:worksAt]->() WITH tiramemsu.transaction(r) AS t SET t.note = 'x'",
        "MATCH ()-[r:worksAt]->() WITH tiramemsu.transaction(r) AS t REMOVE t.`sys:author`",
        "MATCH ()-[r:worksAt]->() WITH tiramemsu.transaction(r) AS t CREATE (t)-[:approved]->(:X)",
        "MATCH ()-[r:worksAt]->() WITH tiramemsu.transaction(r) AS t DETACH DELETE t",
    ] {
        let e = t.cy_write(q).expect_err(q);
        assert!(matches!(e, Error::Unsupported { .. }), "{q}: {e:?}");
    }
    assert_eq!(t.last_t(), n, "nothing was written");
}

// ---- both dialects ------------------------------------------------------------------

// @tg: verifies:: [[tests#Statement Metadata#Dialects Agree On Statement Metadata]] {since: "2026-10-14", level: unit}
#[test]
fn dialects_agree_on_statement_metadata() {
    let t = T::new();
    t.upd(r#"INSERT DATA { v:a v:p v:b {| tm:validFrom "2020-01-01"^^xsd:date |} }"#);
    let res = t.cy("MATCH ()-[r:p]->() RETURN r.validFrom");
    assert_eq!(row(&res)[0], CypherValue::DateTime { ms: Y2020, tz: 0 });
    t.cy_write("MATCH ()-[r:p]->() SET r.validTo = date('2024-01-01')")
        .unwrap();
    let s =
        t.sel("SELECT ?e ?tx ?to WHERE { v:a v:p v:b ~ ?e . ?e tm:txAdded ?tx ; tm:validTo ?to }");
    assert_eq!(s.rows.len(), 1);
    let get = |n: &str| s.rows[0][s.col(n).unwrap()].clone().unwrap();
    assert!(matches!(get("to"), Value::DateTime { ms, .. } if ms == Y2024));
    let res = t.cy("MATCH ()-[r:p]->() RETURN elementId(r), elementId(tiramemsu.transaction(r))");
    let x = row(&res);
    assert_eq!(x[0], CypherValue::String(get("e").lexical()));
    assert_eq!(x[1], CypherValue::String(get("tx").lexical()));
}
