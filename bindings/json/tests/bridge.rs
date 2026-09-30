//! The JSON bridge end to end: every operation the Node.js and Python wrappers call.

use serde_json::{json, Value as J};
use tiramemsu_json::Database;

fn open() -> (tempfile::TempDir, Database) {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(dir.path().join("m.db").to_str().unwrap(), &J::Null).unwrap();
    (dir, db)
}

fn v(s: &str) -> J {
    json!({ "iri": format!("urn:tiramemsu:v:{s}") })
}

fn works_at(db: &Database, view: J) -> Vec<J> {
    let r = db
        .call(
            "sparql",
            &json!({ "view": view, "text": "SELECT ?o WHERE { v:alice v:worksAt ?o }" }),
        )
        .unwrap();
    r["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["o"].clone())
        .collect()
}

/// Alice at Acme from 2020 (tx 1), corrected to end in 2024 (tx 2), then Globex from March 2024 (tx 3).
fn alice(db: &Database) -> J {
    db.call("transact", &json!({ "ops": [
        { "op": "assert", "s": v("alice"), "p": v("worksAt"), "o": v("acme"), "validFrom": "2020-01-01", "as": "job" },
        { "op": "assert", "s": {"ref": "job"}, "p": v("confidence"), "o": 0.8 }
    ]})).unwrap();
    let t2 = db
        .call(
            "transact",
            &json!({ "ops": [
                { "op": "supersede", "eid": 1, "patch": { "validTo": "2024-01-01" } }
            ]}),
        )
        .unwrap();
    db.call("transact", &json!({ "ops": [
        { "op": "assert", "s": v("alice"), "p": v("worksAt"), "o": v("globex"), "validFrom": "2024-03-01" }
    ]})).unwrap();
    t2
}

#[test]
fn a_transaction_reports_and_resolves_named_refs() {
    let (_d, db) = open();
    let r = db
        .call(
            "transact",
            &json!({ "ops": [
                { "op": "assert", "s": v("alice"), "p": v("worksAt"), "o": v("acme"), "as": "job" },
                { "op": "assert", "s": {"ref": "job"}, "p": v("source"), "o": "chat-1" },
                { "op": "assert", "s": v("alice"), "p": v("worksAt"), "o": v("acme") },
            ]}),
        )
        .unwrap();
    assert_eq!(r["t"], 1);
    assert_eq!(r["asserted"].as_array().unwrap().len(), 2);
    // `existing` lists matches that this transaction did not insert itself.
    assert!(r["existing"].as_array().unwrap().is_empty());
    assert_eq!(r["results"][0]["new"], true);
    assert_eq!(r["results"][2]["new"], false);
    assert_eq!(r["refs"]["job"], r["results"][0]["eid"]);
}

#[test]
fn time_travel_across_both_clocks() {
    let (_d, db) = open();
    alice(&db);
    let now = json!({ "kind": "now" });
    let mut both = works_at(&db, now);
    both.sort_by_key(|x| x.to_string());
    assert_eq!(both, vec![v("acme"), v("globex")]);
    let as_of = |t: u64, valid: Option<&str>| {
        let mut view = json!({ "kind": "asOf", "tx": t });
        if let Some(d) = valid {
            view["validAt"] = json!(d);
        }
        view
    };
    assert_eq!(works_at(&db, as_of(1, None)), vec![v("acme")]);
    // What we believed after tx 1 about 2026: still at Acme. We were wrong, and we remember it.
    assert_eq!(works_at(&db, as_of(1, Some("2026-01-01"))), vec![v("acme")]);
    assert_eq!(
        works_at(&db, json!({ "kind": "now", "validAt": "2026-01-01" })),
        vec![v("globex")]
    );
    assert!(works_at(&db, json!({ "kind": "now", "validAt": "2024-02-01" })).is_empty());
    // SPARQL sees a set of (s, p, o), so the two Acme episodes are one row; Cypher and
    // `triples` see one row per statement.
    assert_eq!(works_at(&db, json!({ "kind": "history" })).len(), 2);
    let r = db
        .call("cypher", &json!({ "view": {"kind": "history"}, "text": "MATCH (a {`@id`: 'v:alice'})-[:worksAt]->(c) RETURN c" }))
        .unwrap();
    assert_eq!(r["rows"].as_array().unwrap().len(), 3);
}

#[test]
fn triples_carry_their_lifetime_and_the_history_keeps_retracted_rows() {
    let (_d, db) = open();
    alice(&db);
    let rows = db
        .call("triples", &json!({ "view": {"kind": "history"}, "s": v("alice"), "p": v("worksAt"), "o": v("acme") }))
        .unwrap();
    let rows = rows.as_array().unwrap();
    assert_eq!(rows.len(), 2);
    let retracted: Vec<_> = rows.iter().filter(|r| !r["tRet"].is_null()).collect();
    assert_eq!(retracted.len(), 1);
    assert_eq!(retracted[0]["retKind"], "supersede");
    assert_eq!(retracted[0]["tRet"], 2);
    // A term that was never stored matches nothing instead of failing.
    let none = db.call("triples", &json!({ "s": v("nobody") })).unwrap();
    assert_eq!(none, json!([]));
}

#[test]
fn cypher_reads_layers_and_a_write_returns_its_report() {
    let (_d, db) = open();
    let w = db
        .call("cypherWrite", &json!({ "text": "CREATE (:Person {name: $n})-[:KNOWS]->(:Person {name: 'Bob'})", "params": { "n": "Alice" } }))
        .unwrap();
    assert!(!w["report"]["asserted"].as_array().unwrap().is_empty());
    let r = db
        .call(
            "cypher",
            &json!({ "text": "MATCH (p:Person)-[:KNOWS]->(q) RETURN p.name AS a, q.name AS b" }),
        )
        .unwrap();
    assert_eq!(r["columns"], json!(["a", "b"]));
    assert_eq!(r["rows"], json!([["Alice", "Bob"]]));
    let e = db
        .call("cypher", &json!({ "text": "CREATE (:Person)" }))
        .unwrap_err();
    assert_eq!(e.code(), "Unsupported");
}

#[test]
fn speculation_answers_what_if_and_keeps_nothing() {
    let (_d, db) = open();
    let r = db
        .call("with", &json!({
            "ops": [{ "op": "assert", "s": v("alice"), "p": v("worksAt"), "o": v("acme") }],
            "queries": [{ "op": "triples" }, { "op": "sparql", "text": "ASK { v:alice v:worksAt v:acme }" }],
        }))
        .unwrap();
    assert_eq!(r["results"][0].as_array().unwrap().len(), 1);
    assert_eq!(r["results"][1]["value"], true);
    assert_eq!(db.call("triples", &json!({})).unwrap(), json!([]));
    assert_eq!(db.call("events", &json!({})).unwrap(), json!([]));
}

#[test]
fn a_dry_run_reports_without_committing() {
    let (_d, db) = open();
    let r = db
        .call(
            "transact",
            &json!({
                "options": { "dryRun": true },
                "ops": [{ "op": "assert", "s": v("a"), "p": v("p"), "o": v("b") }],
            }),
        )
        .unwrap();
    assert_eq!(r["asserted"].as_array().unwrap().len(), 1);
    assert_eq!(db.call("triples", &json!({})).unwrap(), json!([]));
}

#[test]
fn retract_cascades_and_the_event_log_records_it() {
    let (_d, db) = open();
    db.call(
        "transact",
        &json!({ "ops": [
            { "op": "assert", "s": v("alice"), "p": v("worksAt"), "o": v("acme"), "as": "job" },
            { "op": "assert", "s": {"ref": "job"}, "p": v("confidence"), "o": 0.8 },
        ]}),
    )
    .unwrap();
    let r = db
        .call(
            "transact",
            &json!({ "ops": [{ "op": "retract", "eid": 1 }] }),
        )
        .unwrap();
    let kinds: Vec<_> = r["retracted"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["kind"].clone())
        .collect();
    assert_eq!(kinds, [json!("explicit"), json!("cascade")]);
    let events = db.call("events", &json!({ "since": 1 })).unwrap();
    assert_eq!(events.as_array().unwrap().len(), 2);
    assert_eq!(events[0]["op"], "retract");
    assert_eq!(db.call("triples", &json!({})).unwrap(), json!([]));
}

#[test]
fn paths_reach_through_a_chain() {
    let (_d, db) = open();
    db.call(
        "transact",
        &json!({ "ops": [
            { "op": "assert", "s": v("a"), "p": v("knows"), "o": v("b") },
            { "op": "assert", "s": v("b"), "p": v("knows"), "o": v("c") },
        ]}),
    )
    .unwrap();
    let r = db
        .call("path", &json!({ "start": v("a"), "path": "knows+" }))
        .unwrap();
    assert_eq!(r.as_array().unwrap().len(), 2);
    let r = db
        .call(
            "path",
            &json!({ "start": v("a"), "path": "knows+", "mode": "anyShortest" }),
        )
        .unwrap();
    assert!(r[0]["path"]["hops"].is_array());
}

#[test]
fn named_graphs_hold_statements_as_tags() {
    let (_d, db) = open();
    db.call(
        "transact",
        &json!({ "ops": [
            { "op": "assert", "s": v("alice"), "p": v("worksAt"), "o": v("acme"), "as": "job" },
            { "op": "addToGraph", "eid": {"ref": "job"}, "graph": v("session12") },
        ]}),
    )
    .unwrap();
    assert_eq!(
        db.call("graphs", &json!({})).unwrap(),
        json!([v("session12")])
    );
    assert_eq!(
        db.call("graphMembers", &json!({ "graph": v("session12") }))
            .unwrap(),
        json!([1])
    );
}

#[test]
fn terms_round_trip_including_what_json_cannot_hold() {
    let (_d, db) = open();
    db.call("transact", &json!({ "ops": [
        { "op": "assert", "s": v("x"), "p": v("big"), "o": {"$int": "9007199254740993"} },
        { "op": "assert", "s": v("x"), "p": v("whole"), "o": {"lex": "3", "datatype": "http://www.w3.org/2001/XMLSchema#double"} },
        { "op": "assert", "s": v("x"), "p": v("label"), "o": {"lex": "chat", "lang": "fr"} },
        { "op": "assert", "s": v("x"), "p": v("when"), "o": {"lex": "2026-09-30", "datatype": "http://www.w3.org/2001/XMLSchema#date"} },
    ]})).unwrap();
    let rows = db.call("triples", &json!({ "s": v("x") })).unwrap();
    let o = |p: &str| {
        rows.as_array()
            .unwrap()
            .iter()
            .find(|r| r["p"] == v(p))
            .unwrap()["o"]
            .clone()
    };
    assert_eq!(o("big"), json!({"$int": "9007199254740993"}));
    assert_eq!(
        o("whole"),
        json!({"lex": "3.0E0", "datatype": "http://www.w3.org/2001/XMLSchema#double"})
    );
    assert_eq!(o("label"), json!({"lex": "chat", "lang": "fr"}));
    assert_eq!(
        o("when")["datatype"],
        "http://www.w3.org/2001/XMLSchema#date"
    );
}

#[test]
fn errors_carry_a_code_and_bad_arguments_are_told_apart() {
    let (_d, db) = open();
    let e = db
        .call("sparql", &json!({ "text": "SELECT ?" }))
        .unwrap_err();
    assert_eq!(e.code(), "Parse");
    assert!(db
        .call_text("sparql", r#"{"text": "SELECT ?"}"#)
        .unwrap_err()
        .contains("\"code\":\"Parse\""));
    assert_eq!(
        db.call("nope", &json!({})).unwrap_err().code(),
        "InvalidArgument"
    );
    let e = db
        .call(
            "transact",
            &json!({ "ops": [{ "op": "assert", "s": v("a") }] }),
        )
        .unwrap_err();
    assert_eq!(e.code(), "InvalidArgument");
    let e = db.call(
        "transact",
        &json!({ "ops": [{ "op": "retract", "eid": 99 }] }),
    );
    assert_eq!(e.unwrap()["results"][0], json!(false));
    let e = db
        .call(
            "transact",
            &json!({ "ops": [{ "op": "confirm", "eid": 99 }] }),
        )
        .unwrap_err();
    assert_eq!(e.code(), "NotLive");
    let e = db
        .call(
            "sparql",
            &json!({ "view": {"kind": "asOf"}, "text": "ASK {}" }),
        )
        .unwrap_err();
    assert_eq!(e.code(), "InvalidArgument");
    let e = db
        .call(
            "transact",
            &json!({ "ops": [{ "op": "supersede", "eid": 1, "patch": { "s": v("z") } }] }),
        )
        .unwrap_err();
    assert!(matches!(e.code(), "InvalidPatch" | "NotLive"));
}

#[test]
fn a_failed_transaction_commits_nothing() {
    let (_d, db) = open();
    let e = db
        .call(
            "transact",
            &json!({ "ops": [
                { "op": "assert", "s": v("a"), "p": v("p"), "o": v("b") },
                { "op": "bogus" },
            ]}),
        )
        .unwrap_err();
    assert_eq!(e.code(), "InvalidArgument");
    assert_eq!(db.call("triples", &json!({})).unwrap(), json!([]));
}

#[test]
fn open_options_are_checked() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("o.db");
    let bad = Database::open(path.to_str().unwrap(), &json!({ "bogus": 1 })).unwrap_err();
    assert_eq!(bad.code(), "InvalidArgument");
    let db = Database::open(path.to_str().unwrap(), &json!({ "readers": 2 })).unwrap();
    assert_eq!(db.call("info", &J::Null).unwrap()["readers"], 2);
}
