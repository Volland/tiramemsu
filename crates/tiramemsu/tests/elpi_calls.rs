//! ELPI goals inside SPARQL and Cypher (OpenSpec change `add-elpi-query-calls`):
//! one test per scenario of the change's delta spec, each on SQLite and, with
//! the `redb` feature, on redb, where every backend must give the same rows.
#![cfg(all(feature = "sparql", feature = "cypher"))]

use std::sync::Arc;

use tiramemsu::*;

fn v(s: &str) -> Value {
    Value::iri(format!("urn:tiramemsu:v:{s}"))
}

fn backends() -> Vec<Backend> {
    #[allow(unused_mut)]
    let mut out = vec![Backend::Sqlite];
    #[cfg(feature = "redb")]
    out.push(Backend::Redb);
    out
}

fn each<R: std::fmt::Debug + PartialEq>(scenario: impl Fn(&Db) -> R) {
    let mut seen: Vec<(Backend, R)> = Vec::new();
    for b in backends() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(
            dir.path().join(format!("c.{}", b.name())),
            OpenOptions {
                backend: b,
                clock: Arc::new(ManualClock::new(1_000)),
                ..OpenOptions::default()
            },
        )
        .unwrap();
        seen.push((b, scenario(&db)));
    }
    for (b, r) in &seen[1..] {
        assert_eq!(
            r,
            &seen[0].1,
            "{} differs from {}",
            b.name(),
            seen[0].0.name()
        );
    }
}

#[cfg(feature = "elpi")]
const REPORTS: &str = r#"
pred reports o:term, o:term.
reports X Y :- tm.triple X (iri "urn:tiramemsu:v:manager") Y _.
reports X Z :- tm.triple X (iri "urn:tiramemsu:v:manager") Y _, reports Y Z.
"#;

/// alice → bob → carol → dave; bob and carol have an email.
fn org(db: &Db) {
    db.transact(TxOptions::default(), |tx| {
        tx.assert(v("alice"), v("manager"), v("bob"), Valid::ALWAYS)?;
        tx.assert(v("bob"), v("manager"), v("carol"), Valid::ALWAYS)?;
        tx.assert(v("carol"), v("manager"), v("dave"), Valid::ALWAYS)?;
        tx.assert(v("bob"), v("email"), Value::str("bob@x"), Valid::ALWAYS)?;
        tx.assert(v("carol"), v("email"), Value::str("carol@x"), Valid::ALWAYS)?;
        #[cfg(feature = "elpi")]
        tx.store_elpi_program(v("reporting"), REPORTS)?;
        Ok(())
    })
    .unwrap();
}

fn sel(db: &Db, q: &str) -> Result<Solutions> {
    match db.now().sparql(q)? {
        SparqlResult::Solutions(s) => Ok(s),
        other => panic!("not solutions: {other:?}"),
    }
}

const GOAL: &str = r#"reports (iri \"urn:tiramemsu:v:alice\") Y"#;

fn emails_query() -> String {
    format!(
        "SELECT ?Y ?mail {{ SERVICE <urn:tiramemsu:elpi:query> {{ [] elpi:program v:reporting ; \
         elpi:goal \"{GOAL}\" }} ?Y v:email ?mail }} ORDER BY ?mail"
    )
}

#[cfg(feature = "elpi")]
fn mails(s: &Solutions) -> Vec<String> {
    (0..s.rows.len())
        .map(|i| format!("{:?}", s.get(i, "mail")))
        .collect()
}

// @tg: verifies:: [[openspec:elpi-query-calls#ELPI goal calls#Join with a pattern]] {level: integration}
#[test]
#[cfg(feature = "elpi")]
fn join_with_a_pattern() {
    each(|db| {
        org(db);
        let s = sel(db, &emails_query()).unwrap();
        // the same people View::elpi finds, joined to their emails
        let direct = db
            .now()
            .elpi(
                &ElpiProgram::Stored(v("reporting")),
                r#"reports (iri "urn:tiramemsu:v:alice") Y"#,
                &ElpiOptions::default(),
            )
            .unwrap();
        assert_eq!(direct.rows.len(), 3);
        mails(&s)
    });
}

// @tg: verifies:: [[openspec:elpi-query-calls#ELPI goal calls#Cypher call then match]] {level: integration}
#[test]
#[cfg(feature = "elpi")]
fn cypher_call_then_match() {
    each(|db| {
        org(db);
        let q = r#"CALL tiramemsu.elpi.query('v:reporting', 'reports (iri "urn:tiramemsu:v:alice") Y') YIELD Y MATCH (m) WHERE m = Y AND m.email IS NOT NULL RETURN m.email AS mail ORDER BY mail"#;
        let c = db.now().cypher(q, &CypherParams::new()).unwrap();
        let from_cypher: Vec<String> = c.rows.iter().map(|r| format!("{:?}", r[0])).collect();
        let from_sparql = mails(&sel(db, &emails_query()).unwrap());
        assert_eq!(from_cypher.len(), from_sparql.len());
        from_cypher
    });
}

// @tg: verifies:: [[openspec:elpi-query-calls#ELPI goal calls#Computed value refused]] {level: integration}
#[test]
#[cfg(feature = "elpi")]
fn computed_value_refused() {
    each(|db| {
        org(db);
        let q = "SELECT ?S { SERVICE <urn:tiramemsu:elpi:query> { [] elpi:text \
                 \"pred greet o:term.\\ngreet (str \\\"never stored\\\").\" ; elpi:goal \"greet S\" } }";
        matches!(sel(db, q), Err(Error::Unsupported { .. }))
    });
}

// @tg: verifies:: [[openspec:elpi-query-calls#ELPI call feature gating#No ELPI provider]] {level: integration}
#[test]
#[cfg(not(feature = "elpi"))]
fn no_elpi_provider() {
    each(|db| {
        org(db);
        matches!(sel(db, &emails_query()), Err(Error::Unsupported { .. }))
    });
}
