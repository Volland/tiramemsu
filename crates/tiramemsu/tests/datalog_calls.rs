//! Datalog goals inside SPARQL and Cypher (OpenSpec change `add-datalog-query-calls`):
//! one test per scenario of the change's delta spec, each on SQLite and, with the
//! `redb` feature, on redb, where every backend must give the same rows.
#![cfg(all(feature = "datalog", feature = "sparql", feature = "cypher"))]

use std::sync::Arc;
use std::time::Duration;

use tiramemsu::*;

const D: i64 = 86_400_000;

fn v(s: &str) -> Value {
    Value::iri(format!("urn:tiramemsu:v:{s}"))
}

/// January 1 of `y`, epoch ms.
fn year(y: i64) -> i64 {
    let days = |y: i64| {
        let y1 = y - 1;
        365 * (y1 - 1969) + (y1 / 4 - 1969 / 4) - (y1 / 100 - 1969 / 100) + (y1 / 400 - 1969 / 400)
    };
    days(y) * D
}

fn backends() -> Vec<Backend> {
    #[allow(unused_mut)]
    let mut out = vec![Backend::Sqlite];
    #[cfg(feature = "redb")]
    out.push(Backend::Redb);
    out
}

fn open(dir: &tempfile::TempDir, backend: Backend) -> Db {
    Db::open(
        dir.path().join(format!("d.{}", backend.name())),
        OpenOptions {
            backend,
            clock: Arc::new(ManualClock::new(1_000)),
            ..OpenOptions::default()
        },
    )
    .unwrap()
}

/// Runs `scenario` on a fresh database of every backend; what it returns must be
/// equal on every backend.
fn each<R: std::fmt::Debug + PartialEq>(scenario: impl Fn(&Db) -> R) {
    let mut seen: Vec<(Backend, R)> = Vec::new();
    for b in backends() {
        let dir = tempfile::tempdir().unwrap();
        let db = open(&dir, b);
        let r = scenario(&db);
        seen.push((b, r));
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

const REPORTS: &str =
    "reports(X, Y) :- v:manager(X, Y).\nreports(X, Z) :- reports(X, Y), v:manager(Y, Z).\n";

/// alice -> bob -> carol -> dave; bob and carol have an email, dave has none.
fn org(db: &Db) {
    db.transact(TxOptions::default(), |tx| {
        tx.store_program(v("reporting"), REPORTS)?;
        tx.assert(v("alice"), v("manager"), v("bob"), Valid::ALWAYS)?;
        tx.assert(v("bob"), v("manager"), v("carol"), Valid::ALWAYS)?;
        tx.assert(v("carol"), v("manager"), v("dave"), Valid::ALWAYS)?;
        tx.assert(v("bob"), v("email"), Value::str("bob@x.org"), Valid::ALWAYS)?;
        tx.assert(
            v("carol"),
            v("email"),
            Value::str("carol@x.org"),
            Valid::ALWAYS,
        )?;
        Ok(())
    })
    .unwrap();
}

const SERVICE: &str = "SERVICE <urn:tiramemsu:dl:query>";

fn sel(db: &Db, q: &str) -> Solutions {
    match db.now().sparql(q).unwrap_or_else(|e| panic!("{q}: {e}")) {
        SparqlResult::Solutions(s) => s,
        other => panic!("not solutions: {other:?}"),
    }
}

fn lex(s: &Solutions) -> Vec<String> {
    let mut out: Vec<String> = s.rows.iter().map(|r| format!("{r:?}")).collect();
    out.sort();
    out
}

fn node(c: &CypherValue) -> String {
    match c {
        CypherValue::Node(n) => n.term.to_string(),
        other => format!("{other:?}"),
    }
}

// @tg: verifies:: [[openspec:datalog-query-calls#SPARQL goal service#Goal joined with a pattern]] {level: integration}
#[test]
fn goal_joined_with_a_pattern() {
    each(|db| {
        org(db);
        let s = sel(
            db,
            &format!(
                "SELECT ?X ?mail {{ {SERVICE} {{ [] dl:program v:reporting ; \
                 dl:goal \"reports(v:alice, X)\" }} ?X v:email ?mail }}"
            ),
        );
        // bob and carol are above alice and have an email; dave has none
        assert_eq!(s.rows.len(), 2);
        assert!(s.get(0, "X").is_some());
        lex(&s)
    });
}

// @tg: verifies:: [[openspec:datalog-query-calls#SPARQL goal service#Intervals as columns]] {level: integration}
#[test]
fn intervals_as_columns() {
    each(|db| {
        db.transact(TxOptions::default(), |tx| {
            tx.store_program(v("reporting"), REPORTS)?;
            tx.assert(
                v("alice"),
                v("manager"),
                v("bob"),
                Valid::between(year(2020), year(2023)),
            )?;
            Ok(())
        })
        .unwrap();
        let s = sel(
            db,
            &format!(
                "SELECT ?X ?f ?t {{ {SERVICE} {{ [] dl:program v:reporting ; \
                 dl:goal \"reports(v:alice, X)\" ; dl:validFrom ?f ; dl:validTo ?t }} }}"
            ),
        );
        assert_eq!(s.rows.len(), 1);
        assert_eq!(s.get(0, "X"), Some(&v("bob")));
        let at = |ms| Value::DateTime { ms, tz: Some(0) };
        assert_eq!(s.get(0, "f"), Some(&at(year(2020))));
        assert_eq!(s.get(0, "t"), Some(&at(year(2023))));
        lex(&s)
    });
}

// @tg: verifies:: [[openspec:datalog-query-calls#SPARQL goal service#Variable goal refused]] {level: integration}
#[test]
fn variable_goal_refused() {
    each(|db| {
        org(db);
        let shape = |q: String| {
            let e = db.now().sparql(&q).unwrap_err();
            assert!(matches!(e, Error::DatalogCallShape { .. }), "{q}: {e:?}");
        };
        shape(format!(
            "SELECT ?X {{ {SERVICE} {{ [] dl:program v:reporting ; dl:goal ?g }} }}"
        ));
        // a program that is not a constant, a missing goal or program, and both forms
        shape(format!(
            "SELECT ?X {{ {SERVICE} {{ [] dl:program ?p ; dl:goal \"reports(v:alice, X)\" }} }}"
        ));
        shape(format!(
            "SELECT ?X {{ {SERVICE} {{ [] dl:program v:reporting }} }}"
        ));
        shape(format!(
            "SELECT ?X {{ {SERVICE} {{ [] dl:goal \"reports(v:alice, X)\" }} }}"
        ));
        shape(format!(
            "SELECT ?X {{ {SERVICE} {{ [] dl:program v:reporting ; dl:text \"a(X) :- v:p(X).\" ; \
             dl:goal \"reports(v:alice, X)\" }} }}"
        ));
    });
}

const CYPHER_EMAILS: &str = "CALL tiramemsu.datalog.query('v:reporting', 'reports(v:alice, X)') \
     YIELD X MATCH (m) WHERE m = X AND m.email IS NOT NULL RETURN X, m.email AS mail ORDER BY X";

fn cypher_rows(db: &Db, q: &str, params: &CypherParams) -> CypherResult {
    db.now()
        .cypher(q, params)
        .unwrap_or_else(|e| panic!("{q}: {e}"))
}

// @tg: verifies:: [[openspec:datalog-query-calls#Cypher goal procedure#Call then match]] {level: integration}
#[test]
fn cypher_call_then_match() {
    each(|db| {
        org(db);
        let c = cypher_rows(db, CYPHER_EMAILS, &CypherParams::new());
        let s = sel(
            db,
            &format!(
                "SELECT ?X ?mail {{ {SERVICE} {{ [] dl:program v:reporting ; \
                 dl:goal \"reports(v:alice, X)\" }} ?X v:email ?mail }}"
            ),
        );
        let mut from_cypher: Vec<(String, String)> = c
            .rows
            .iter()
            .map(|r| (node(&r[0]), format!("{:?}", r[1])))
            .collect();
        from_cypher.sort();
        let mut from_sparql: Vec<(String, String)> = s
            .rows
            .iter()
            .map(|r| {
                (
                    r[0].as_ref().unwrap().to_string(),
                    format!(
                        "{:?}",
                        CypherValue::String(r[1].as_ref().unwrap().lexical())
                    ),
                )
            })
            .collect();
        from_sparql.sort();
        assert_eq!(from_cypher.len(), 2);
        assert_eq!(from_cypher, from_sparql);
        from_cypher
    });
}

// @tg: verifies:: [[openspec:datalog-query-calls#Cypher goal procedure#Parameters are constants]] {level: integration}
#[test]
fn parameters_are_constants() {
    each(|db| {
        org(db);
        let mut params = CypherParams::new();
        params.insert(
            "goal".into(),
            CypherValue::String("reports(v:zed, X)".into()),
        );
        db.transact(TxOptions::default(), |tx| {
            tx.assert(v("zed"), v("manager"), v("bob"), Valid::ALWAYS)?;
            Ok(())
        })
        .unwrap();
        watch_datalog_calls("reports(v:zed, X)");
        let c = cypher_rows(
            db,
            "CALL tiramemsu.datalog.query('v:reporting', $goal) YIELD X RETURN X ORDER BY X",
            &params,
        );
        // bob, carol, dave, from one call
        assert_eq!(c.rows.len(), 3);
        assert_eq!(watched_datalog_calls(), 1);
        // a row variable is not a constant
        let e = db
            .now()
            .cypher(
                "UNWIND ['reports(v:zed, X)'] AS g \
                 CALL tiramemsu.datalog.query('v:reporting', g) YIELD X RETURN X",
                &CypherParams::new(),
            )
            .unwrap_err();
        assert!(matches!(e, Error::DatalogCallShape { .. }), "{e:?}");
        c.rows.iter().map(|r| node(&r[0])).collect::<Vec<_>>()
    });
}

// @tg: verifies:: [[openspec:datalog-query-calls#Evaluated once per query#Optional side runs once]] {level: integration}
#[test]
fn optional_side_runs_once() {
    each(|db| {
        db.transact(TxOptions::default(), |tx| {
            tx.store_program(v("reporting"), REPORTS)?;
            tx.assert(v("zoe"), v("manager"), v("yan"), Valid::ALWAYS)?;
            tx.assert(v("yan"), v("manager"), v("xi"), Valid::ALWAYS)?;
            for i in 0..1000 {
                tx.assert(
                    v(&format!("n{i}")),
                    v("email"),
                    Value::str("m"),
                    Valid::ALWAYS,
                )?;
            }
            tx.assert(v("yan"), v("email"), Value::str("m"), Valid::ALWAYS)?;
            Ok(())
        })
        .unwrap();
        let q = format!(
            "SELECT ?s ?X {{ ?s v:email ?m OPTIONAL {{ {SERVICE} {{ [] dl:program v:reporting ; \
             dl:goal \"reports(v:zoe, X)\" }} FILTER(?X = ?s) }} }}"
        );
        watch_datalog_calls("reports(v:zoe, X)");
        let with_optional = sel(db, &q);
        assert_eq!(
            watched_datalog_calls(),
            1,
            "the goal ran once for 1001 rows"
        );
        // the call's rows computed separately, then joined
        let rows = sel(
            db,
            &format!(
                "SELECT ?X {{ {SERVICE} {{ [] dl:program v:reporting ; \
                 dl:goal \"reports(v:zoe, X)\" }} }}"
            ),
        );
        assert_eq!(rows.rows.len(), 2);
        let matched: Vec<_> = with_optional
            .rows
            .iter()
            .filter(|r| r[1].is_some())
            .collect();
        // yan is the one email holder among the call's rows
        assert_eq!(matched.len(), 1);
        assert_eq!(with_optional.rows.len(), 1001);
        // explain shows one evaluation: one `tm_datalog` call in the plan
        let ex = db.now().explain_sparql(&q).unwrap();
        if let Some(sql) = &ex.sql {
            assert_eq!(sql.matches("tm_datalog(").count(), 1, "{sql}");
        }
        lex(&with_optional)
    });
}

// @tg: verifies:: [[openspec:datalog-query-calls#Same rows as the Rust call#Three surfaces agree]] {level: integration}
#[test]
fn three_surfaces_agree() {
    each(|db| {
        db.transact(TxOptions::default(), |tx| {
            tx.store_program(v("reporting"), REPORTS)?;
            tx.assert(
                v("alice"),
                v("manager"),
                v("bob"),
                Valid::between(year(2020), year(2023)),
            )?;
            tx.assert(
                v("alice"),
                v("manager"),
                v("bob"),
                Valid::between(year(2025), year(2026)),
            )?;
            tx.assert(v("bob"), v("manager"), v("carol"), Valid::ALWAYS)?;
            tx.assert(v("carol"), v("manager"), v("dave"), Valid::from(year(2021)))?;
            Ok(())
        })
        .unwrap();
        let program = DatalogProgram::Stored(v("reporting"));
        let r = db
            .now()
            .datalog(
                &program,
                Some("reports(v:alice, X)"),
                &DatalogOptions::default(),
            )
            .unwrap();
        let mut rust: Vec<(String, Option<i64>, Option<i64>)> = r
            .rows
            .iter()
            .flat_map(|row| {
                row.valid
                    .iter()
                    .map(|iv| (row.values[0].to_string(), iv.from, iv.to))
            })
            .collect();
        rust.sort();
        assert!(rust.len() >= 4);

        let s = sel(
            db,
            &format!(
                "SELECT ?X ?f ?t {{ {SERVICE} {{ [] dl:program v:reporting ; \
                 dl:goal \"reports(v:alice, X)\" ; dl:validFrom ?f ; dl:validTo ?t }} }}"
            ),
        );
        let ms = |c: Option<&Value>| match c {
            Some(Value::DateTime { ms, .. }) => Some(*ms),
            None => None,
            other => panic!("not a date-time: {other:?}"),
        };
        let mut sparql: Vec<_> = s
            .rows
            .iter()
            .map(|r| {
                (
                    r[0].as_ref().unwrap().to_string(),
                    ms(r[1].as_ref()),
                    ms(r[2].as_ref()),
                )
            })
            .collect();
        sparql.sort();

        let c = cypher_rows(
            db,
            "CALL tiramemsu.datalog.query('v:reporting', 'reports(v:alice, X)') \
             YIELD X, validFrom, validTo RETURN X, validFrom, validTo",
            &CypherParams::new(),
        );
        let cms = |c: &CypherValue| match c {
            CypherValue::DateTime { ms, .. } => Some(*ms),
            CypherValue::Null => None,
            other => panic!("not a date-time: {other:?}"),
        };
        let mut cypher: Vec<_> = c
            .rows
            .iter()
            .map(|r| (node(&r[0]), cms(&r[1]), cms(&r[2])))
            .collect();
        cypher.sort();
        assert_eq!(rust, sparql);
        assert_eq!(rust, cypher);
        rust
    });
}

// @tg: verifies:: [[openspec:datalog-query-calls#Budgets apply to calls#Deadline inside a query]] {level: integration}
#[test]
fn deadline_inside_a_query() {
    each(|db| {
        db.transact(TxOptions::default(), |tx| {
            tx.store_program(v("reporting"), REPORTS)?;
            for i in 0..3000 {
                tx.assert(
                    v(&format!("n{i}")),
                    v("manager"),
                    v(&format!("n{}", i + 1)),
                    Valid::ALWAYS,
                )?;
            }
            Ok(())
        })
        .unwrap();
        let budget = QueryBudget {
            timeout: Some(Duration::from_millis(1)),
            ..Default::default()
        };
        let e = db
            .now()
            .with_budget(&budget)
            .sparql(&format!(
                "SELECT ?X ?Y {{ {SERVICE} {{ [] dl:program v:reporting ; dl:goal \"reports(X, Y)\" }} }}"
            ))
            .unwrap_err();
        assert!(matches!(e, Error::DeadlineExceeded { .. }), "{e:?}");
    });
}

// @tg: verifies:: [[openspec:datalog-query-calls#Provenance gap for calls#Provenance marks the call]] {level: integration}
#[test]
fn provenance_marks_the_call() {
    each(|db| {
        org(db);
        let opts = SparqlOptions {
            provenance: true,
            ..Default::default()
        };
        let q = format!(
            "SELECT ?X ?mail {{ {SERVICE} {{ [] dl:program v:reporting ; \
             dl:goal \"reports(v:alice, X)\" }} ?X v:email ?mail }}"
        );
        let SparqlResult::Solutions(s) = db.now().sparql_with(&q, &opts).unwrap() else {
            panic!("not solutions")
        };
        assert_eq!(s.rows.len(), 2);
        assert!(s.provenance_gaps.contains(&ProvenanceGap::DatalogCall));
        assert_eq!(s.provenance_complete(), Some(false));
        // the email statements matched by the ordinary pattern are cited
        for i in 0..s.rows.len() {
            assert_eq!(s.provenance(i).unwrap().len(), 1);
        }
        s.provenance_gaps.len()
    });
}
