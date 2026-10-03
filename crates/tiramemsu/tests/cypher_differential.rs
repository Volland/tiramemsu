//! `dialect-differential-testing`: equivalent SPARQL and Cypher queries over shared
//! fixtures return identical normalised results.
#![cfg(all(feature = "sparql", feature = "cypher"))]
mod cypher_common;
mod differential;

use differential::fixtures;
use differential::normalise::*;
use differential::runner::*;
use tiramemsu::*;

fn sparql_on() -> bool {
    cfg!(feature = "sparql")
}

// dialect-differential-testing "Simple pair passes" and the whole corpus
// @lat: [[tests#Query#Differential SPARQL Cypher]]
#[test]
fn corpus_pairs_return_identical_results() {
    let pairs = corpus();
    let mut failures = Vec::new();
    let mut pending = Vec::new();
    for p in &pairs {
        match run_pair(p, sparql_on()) {
            Outcome::Pass => {}
            Outcome::Pending => pending.push(p.name.clone()),
            Outcome::Fail(why) => failures.push(why),
        }
    }
    if !pending.is_empty() {
        println!("pending (no SPARQL front end): {pending:?}");
    }
    assert!(
        failures.is_empty(),
        "{} pair(s) differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
    if sparql_on() {
        assert!(pending.is_empty());
    }
}

// "Corpus completeness check"
#[test]
fn corpus_is_complete() {
    let pairs = corpus();
    assert!(completeness(&pairs).is_ok(), "{:?}", completeness(&pairs));
    assert!(pairs.len() >= 40, "{} pairs", pairs.len());
    // a corpus without a category names it
    let fewer: Vec<Pair> = pairs
        .iter()
        .filter(|p| p.category != "history")
        .cloned()
        .collect();
    let missing = completeness(&fewer).unwrap_err();
    assert!(missing.contains(&"history".to_string()), "{missing:?}");
    let few: Vec<Pair> = pairs.iter().take(3).cloned().collect();
    assert!(completeness(&few)
        .unwrap_err()
        .iter()
        .any(|m| m.contains("at least 40")));
}

// "Fixture reproducibility"
#[test]
fn fixtures_are_reproducible() {
    let dump = |fx: &fixtures::Fx| {
        (
            fx.db.read_sql("SELECT eid, s, p, o, t_add, t_ret, v_from, v_to, ret_kind FROM triple ORDER BY eid").unwrap(),
            fx.db.read_sql("SELECT t, instant FROM tx ORDER BY t").unwrap(),
        )
    };
    assert_eq!(fixtures::date_ms(2026, 9, 1), fixtures::BASE);
    for name in fixtures::NAMES {
        let (a, b) = (fixtures::load(name), fixtures::load(name));
        assert_eq!(dump(&a), dump(&b), "fixture {name}");
        assert!(!dump(&a).0.is_empty(), "fixture {name} is empty");
    }
}

// "Null and unbound compare equal"
#[test]
fn null_and_unbound_compare_equal() {
    assert_eq!(from_term(&None), from_cypher(&CypherValue::Null, false));
    let p = corpus()
        .into_iter()
        .find(|p| p.name == "optional age is missing for dave")
        .unwrap();
    assert_eq!(run_pair(&p, true), Outcome::Pass);
    let fx = fixtures::load("employment");
    let rows = cypher_rows(&fx.db.now(), &p).unwrap();
    // columns are projected sorted by SPARQL variable: (age, p)
    assert!(rows.iter().any(|r| r[0] == Canon::Missing));
}

// "Numeric normalisation" / "Datetime offsets survive normalisation"
#[test]
fn numeric_and_datetime_normalisation() {
    let int = Some(Value::literal("42", Some(vocab::XSD_INTEGER), None));
    assert_eq!(
        from_term(&int),
        from_cypher(&CypherValue::Integer(42), false)
    );
    let dec = Some(Value::literal("42.0", Some(vocab::XSD_DECIMAL), None));
    assert_eq!(
        from_term(&dec),
        from_cypher(&CypherValue::Float(42.0), false)
    );
    let sp = Some(Value::literal(
        "2026-03-01T12:00:00+02:00",
        Some(vocab::XSD_DATETIME),
        None,
    ));
    let cy = |tz: i16| CypherValue::DateTime {
        ms: 1_772_359_200_000,
        tz,
    };
    // 2026-03-01T10:00:00Z is 1_772_359_200_000 ms
    assert_eq!(from_term(&sp), from_cypher(&cy(120), false));
    assert_ne!(
        from_term(&sp),
        from_cypher(&cy(0), false),
        "same instant, another offset, is a mismatch"
    );
    // transactions and skolem IRIs
    assert_eq!(
        from_term(&Some(Value::iri("urn:tiramemsu:tx:5"))),
        from_cypher(&CypherValue::Integer(5), true)
    );
    assert_eq!(
        from_term(&Some(Value::iri("urn:tiramemsu:stmt:3"))),
        from_cypher(&CypherValue::String("urn:tiramemsu:stmt:3".into()), false)
    );
}

// "Mismatch report"
#[test]
fn mismatch_report_names_the_pair_and_the_extra_row() {
    let mut p = corpus()
        .into_iter()
        .find(|p| p.name == "label is rdf:type")
        .unwrap();
    p.cypher = "MATCH (p:Person) WHERE p.name <> 'Dave' RETURN p".into();
    match run_pair(&p, true) {
        Outcome::Fail(msg) => {
            assert!(msg.contains("label is rdf:type"), "{msg}");
            assert!(msg.contains("MATCH (p:Person) WHERE"), "{msg}");
            assert!(msg.contains("SELECT ?p WHERE"), "{msg}");
            assert!(
                msg.contains("only in SPARQL") && msg.contains("dave"),
                "{msg}"
            );
            assert!(!msg.contains("only in Cypher"), "{msg}");
        }
        other => panic!("{other:?}"),
    }
}

// "SPARQL front end missing": every Cypher query compiles and runs, and the SPARQL side is pending
#[test]
fn cypher_side_runs_without_the_sparql_side() {
    for p in corpus() {
        assert_eq!(run_pair(&p, false), Outcome::Pending, "{}", p.name);
    }
}
