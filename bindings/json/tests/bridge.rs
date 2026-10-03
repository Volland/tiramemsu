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

// json-bridge "Time-respecting path"
#[test]
fn time_respecting_paths_report_arrival() {
    let (_d, db) = open();
    db.call(
        "transact",
        &json!({ "ops": [
            { "op": "assert", "s": v("a"), "p": v("met"), "o": v("b"), "validFrom": 1, "validTo": 5 },
            { "op": "assert", "s": v("b"), "p": v("met"), "o": v("c"), "validFrom": 3, "validTo": 9 },
        ]}),
    )
    .unwrap();
    let arrivals = |tr: J| {
        let r = db
            .call(
                "path",
                &json!({ "start": v("a"), "path": "met+", "timeRespecting": tr }),
            )
            .unwrap();
        r.as_array()
            .unwrap()
            .iter()
            .map(|row| (row["end"].clone(), row["arrival"].clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        arrivals(json!(true)),
        vec![(v("b"), json!(1)), (v("c"), json!(3))]
    );
    assert!(arrivals(json!({ "after": 6 })).is_empty());
    assert_eq!(
        arrivals(json!({ "after": 2 })),
        vec![(v("b"), json!(2)), (v("c"), json!(3))]
    );
    // without the option the rows carry a null arrival
    assert_eq!(
        arrivals(json!(null)),
        vec![(v("b"), J::Null), (v("c"), J::Null)]
    );
    for bad in [json!("yes"), json!({ "before": 1 })] {
        assert!(db
            .call(
                "path",
                &json!({ "start": v("a"), "path": "met+", "timeRespecting": bad })
            )
            .is_err());
    }
}

// json-bridge "Path inside a graph"
#[test]
fn paths_stay_inside_the_listed_graphs() {
    let (_d, db) = open();
    db.call(
        "transact",
        &json!({ "ops": [
            { "op": "assert", "s": v("a"), "p": v("knows"), "o": v("b"), "as": "ab" },
            { "op": "assert", "s": v("b"), "p": v("knows"), "o": v("c") },
            { "op": "addToGraph", "eid": {"ref": "ab"}, "graph": v("session12") },
        ]}),
    )
    .unwrap();
    let ends = |graphs: J| {
        let r = db
            .call(
                "path",
                &json!({ "start": v("a"), "path": "knows+", "graphs": graphs }),
            )
            .unwrap();
        r.as_array()
            .unwrap()
            .iter()
            .map(|row| row["end"].clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(ends(json!([v("session12")])), vec![v("b")]);
    assert_eq!(ends(json!(null)).len(), 2, "no filter");
    // a graph that was never written names no graph: no hop is taken
    assert!(ends(json!([v("nowhere")])).is_empty());
    assert!(db
        .call(
            "path",
            &json!({ "start": v("a"), "path": "knows+", "graphs": "session12" })
        )
        .is_err());
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

// json-bridge "Subject type violation"
// @lat: [[tests#Typed Layers#Bridge Reports Subject Type Mismatch]]
#[test]
fn subject_type_mismatch_has_its_own_code() {
    let (_d, db) = open();
    let sys = |s: &str| json!({ "iri": format!("urn:tiramemsu:sys:{s}") });
    db.call(
        "transact",
        &json!({ "ops": [{ "op": "assert", "s": v("confidence"), "p": sys("subjectType"), "o": sys("STMT") }] }),
    )
    .unwrap();
    let e = db
        .call(
            "transact",
            &json!({ "ops": [
                { "op": "assert", "s": v("alice"), "p": v("worksAt"), "o": v("acme") },
                { "op": "assert", "s": v("alice"), "p": v("confidence"), "o": 0.8 },
            ]}),
        )
        .unwrap_err();
    assert_eq!(e.code(), "SubjectTypeMismatch");
    assert_eq!(e.to_json()["code"], "SubjectTypeMismatch");
    let live = db.call("triples", &json!({})).unwrap();
    assert_eq!(live.as_array().unwrap().len(), 1, "{live}");
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

// json-bridge "Query results": SPARQL rows with provenance
// @lat: [[tests#Query Provenance#Bridge Returns Provenance]]
#[test]
fn sparql_rows_can_carry_provenance() {
    let (_d, db) = open();
    db.call(
        "transact",
        &json!({ "ops": [{ "op": "assert", "s": v("a"), "p": v("p"), "o": v("b") }] }),
    )
    .unwrap();
    let text = "SELECT ?o WHERE { v:a v:p ?o }";
    let r = db
        .call("sparql", &json!({ "text": text, "provenance": true }))
        .unwrap();
    assert_eq!(r["rows"], json!([{ "o": v("b") }]));
    assert_eq!(r["provenance"], json!([[{ "stmt": 1 }]]));
    let plain = db.call("sparql", &json!({ "text": text })).unwrap();
    assert!(plain.get("provenance").is_none());
    let e = db
        .call(
            "sparql",
            &json!({ "text": "ASK { ?s ?p ?o }", "provenance": true }),
        )
        .unwrap_err();
    assert_eq!(e.code(), "Unsupported");
    let e = db
        .call("sparql", &json!({ "text": text, "provenance": "yes" }))
        .unwrap_err();
    assert_eq!(e.code(), "InvalidArgument");
}

#[test]
fn dependents_preview_a_retraction_on_any_view() {
    let (_d, db) = open();
    let r = db
        .call(
            "transact",
            &json!({ "ops": [
                { "op": "assert", "s": v("alice"), "p": v("worksAt"), "o": v("acme"), "as": "job" },
                { "op": "assert", "s": {"ref": "job"}, "p": v("source"), "o": "chat-1" },
                { "op": "assert", "s": v("belief9"), "p": v("supportedBy"), "o": {"ref": "job"} }
            ]}),
        )
        .unwrap();
    let job = r["refs"]["job"].clone();
    assert_eq!(
        db.call("dependents", &json!({ "eid": job })).unwrap(),
        r["asserted"]
    );
    // once retracted: nothing on the now view, the structure as of transaction 1
    db.call(
        "transact",
        &json!({ "ops": [{ "op": "retract", "eid": job }] }),
    )
    .unwrap();
    let then = json!({ "eid": { "stmt": job }, "view": { "kind": "asOf", "tx": 1 } });
    assert_eq!(db.call("dependents", &then).unwrap(), r["asserted"]);
    assert_eq!(
        db.call("dependents", &json!({ "eid": job })).unwrap(),
        json!([])
    );
    let e = db.call("dependents", &json!({})).unwrap_err();
    assert_eq!(e.code(), "InvalidArgument");
}

// @lat: [[tests#Fact Bundles#Bundles Cross The JSON Bridge]]
#[test]
fn a_bundle_moves_between_bridge_databases() {
    let (_d, a) = open();
    let (_e, b) = open();
    let r = a
        .call(
            "transact",
            &json!({ "ops": [
                { "op": "assert", "s": v("alice"), "p": v("worksAt"), "o": v("acme"), "validFrom": "2020-01-01", "as": "job" },
                { "op": "assert", "s": {"ref": "job"}, "p": v("confidence"), "o": 0.8 },
                { "op": "addToGraph", "eid": {"ref": "job"}, "graph": v("session12") }
            ]}),
        )
        .unwrap();
    let bundle = a
        .call("bundle", &json!({ "eid": r["refs"]["job"] }))
        .unwrap();
    assert_eq!(bundle["format"], "tiramemsu-bundle/1");
    assert_eq!(bundle["statements"].as_array().unwrap().len(), 3);
    let t = b
        .call(
            "transact",
            &json!({ "ops": [
                { "op": "importBundle", "bundle": bundle, "as": "fact" },
                { "op": "assert", "s": {"ref": "fact"}, "p": v("importedFrom"), "o": v("agentA") }
            ]}),
        )
        .unwrap();
    let res = &t["results"][0];
    assert_eq!(res["root"], t["refs"]["fact"]);
    assert_eq!(res["statements"].as_array().unwrap().len(), 3);
    assert!(res["statements"]
        .as_array()
        .unwrap()
        .iter()
        .all(|s| s["new"] == true));
    let members = b
        .call("graphMembers", &json!({ "graph": v("session12") }))
        .unwrap();
    assert_eq!(members, json!([t["refs"]["fact"]]));
    // valid time travelled with the fact
    let rows = b
        .call("triples", &json!({ "s": v("alice"), "p": v("worksAt") }))
        .unwrap();
    assert_eq!(
        rows[0]["validFrom"],
        a.call("triples", &json!({ "s": v("alice") })).unwrap()[0]["validFrom"]
    );
    // a malformed bundle is refused and commits nothing
    let e = b
        .call(
            "transact",
            &json!({ "ops": [{ "op": "importBundle", "bundle": { "format": "tiramemsu-bundle/9" } }] }),
        )
        .unwrap_err();
    assert_eq!(e.code(), "InvalidTerm");
}

#[test]
fn reserved_origins_and_exhausted_counters_have_codes() {
    let (_d, db) = open();
    // a statement number with a non-zero origin (payload bits 48..60) is refused
    let foreign = (1u64 << 48) | 5;
    let e = db
        .call(
            "transact",
            &json!({ "ops": [{ "op": "confirm", "eid": foreign }] }),
        )
        .unwrap_err();
    assert_eq!(e.code(), "Unsupported");
    assert!(e.to_string().contains("origin 1"), "{e}");
    let e = tiramemsu_json::BindError::Db(tiramemsu::Error::IdSpaceExhausted {
        kind: tiramemsu::Tag::Stmt,
    });
    assert_eq!(e.code(), "IdSpaceExhausted");
}

/// `n` statements `v:n<i> v:p i` in one transaction: enough for a cross join that
/// never finishes.
fn seed(db: &Database, n: usize) {
    let ops: Vec<J> = (0..n)
        .map(|i| json!({ "op": "assert", "s": v(&format!("n{i}")), "p": v("p"), "o": i }))
        .collect();
    db.call("transact", &json!({ "ops": ops })).unwrap();
}

const CROSS: &str = "SELECT (COUNT(*) AS ?c) WHERE { ?a v:p ?x . ?b v:p ?y . ?c2 v:p ?z }";

// query-budgets: the bridge's `budget` argument, `readerTimeoutMs` and error codes
// @lat: [[tests#Query Budgets#Bridge Budgets And Error Codes]]
#[test]
fn budgets_bound_bridge_calls_with_typed_codes() {
    let (_d, db) = open();
    seed(&db, 1_500);
    let r = db
        .call(
            "sparql",
            &json!({ "text": CROSS, "budget": { "timeoutMs": 100 } }),
        )
        .unwrap_err();
    assert_eq!(r.code(), "DeadlineExceeded");
    let r = db
        .call(
            "sparql",
            &json!({ "text": "SELECT ?s WHERE { ?s v:p ?o }", "budget": { "maxRows": 10 } }),
        )
        .unwrap_err();
    assert_eq!(r.code(), "ResultLimitExceeded");
    assert_eq!(
        r.to_json()["message"],
        "result exceeds the limit of 10 rows"
    );
    let r = db
        .call("triples", &json!({ "budget": { "maxBytes": 64 } }))
        .unwrap_err();
    assert_eq!(r.code(), "ResultLimitExceeded");
    // a fitting budget changes nothing
    let few = db
        .call(
            "triples",
            &json!({ "s": v("n1"), "budget": { "maxRows": 5, "timeoutMs": 10000 } }),
        )
        .unwrap();
    assert_eq!(few.as_array().unwrap().len(), 1);
    // an interrupted write commits nothing
    let r = db
        .call(
            "cypherWrite",
            &json!({
                "text": "MATCH (a), (b), (c) WHERE a.p >= 0 AND b.p >= 0 AND c.p >= 0 CREATE (a)-[:q]->(b)",
                "budget": { "timeoutMs": 100 }
            }),
        )
        .unwrap_err();
    assert_eq!(r.code(), "DeadlineExceeded");
    // a cancel key that nobody cancels changes nothing
    db.call(
        "transact",
        &json!({ "ops": [{ "op": "assert", "s": v("x"), "p": v("q"), "o": v("y") }],
                 "budget": { "cancelKey": "kept" } }),
    )
    .unwrap();
    assert_eq!(
        db.call("cancel", &json!({ "key": "w" })).unwrap()["running"],
        false
    );
    let r = db
        .call(
            "transact",
            &json!({ "ops": [{ "op": "assert", "s": v("z"), "p": v("q"), "o": v("y") }],
                     "budget": { "cancelKey": "w" } }),
        )
        .unwrap_err();
    assert_eq!(r.code(), "Cancelled"); // cancelled before it started
    assert_eq!(
        db.call("triples", &json!({ "p": v("q") }))
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
    // malformed budgets are argument errors
    for bad in [
        json!(5),
        json!({ "timeoutMs": -1 }),
        json!({ "bogus": 1 }),
        json!({ "cancelKey": 3 }),
    ] {
        let e = db.call("triples", &json!({ "budget": bad })).unwrap_err();
        assert_eq!(e.code(), "InvalidArgument", "{bad}");
    }
    assert_eq!(
        db.call("cancel", &json!({})).unwrap_err().code(),
        "InvalidArgument"
    );

    // the database-wide reader timeout
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("r.db");
    let db = Database::open(
        path.to_str().unwrap(),
        &json!({ "readers": 1, "readerTimeoutMs": 50 }),
    )
    .unwrap();
    assert_eq!(db.call("triples", &json!({})).unwrap(), json!([]));
    assert_eq!(
        Database::open(path.to_str().unwrap(), &json!({ "readerTimeoutMs": "x" }))
            .unwrap_err()
            .code(),
        "InvalidArgument"
    );
}

// query-budgets: `cancel` from another thread stops a running call
// @lat: [[tests#Query Budgets#Bridge Cancels A Running Call]]
#[test]
fn cancel_stops_a_running_call_from_another_thread() {
    let (_d, db) = open();
    seed(&db, 1_500);
    let db = std::sync::Arc::new(db);
    let canceller = {
        let db = db.clone();
        std::thread::spawn(move || {
            // wait until the call holds its key, then cancel it
            for _ in 0..500 {
                std::thread::sleep(std::time::Duration::from_millis(10));
                let r = db.call("cancel", &json!({ "key": "q1" })).unwrap();
                if r["running"] == json!(true) {
                    return true;
                }
            }
            false
        })
    };
    let t0 = std::time::Instant::now();
    let e = db
        .call(
            "sparql",
            &json!({ "text": CROSS, "budget": { "cancelKey": "q1" } }),
        )
        .unwrap_err();
    assert_eq!(e.code(), "Cancelled");
    assert!(t0.elapsed() < std::time::Duration::from_secs(10));
    assert!(canceller.join().unwrap());
    // other keys are unaffected
    let ok = db
        .call(
            "triples",
            &json!({ "s": v("n0"), "budget": { "cancelKey": "fresh" } }),
        )
        .unwrap();
    assert_eq!(ok.as_array().unwrap().len(), 1);
}

fn chunk_ops(c: usize, n: usize) -> J {
    J::Array(
        (0..n)
            .map(|i| json!({ "op": "assert", "s": v(&format!("c{c}-{i}")), "p": v("p"), "o": i }))
            .collect(),
    )
}

// @lat: [[tests#Bulk Import#Bridge Import Sessions]]
#[test]
fn bulk_import_sessions_cross_the_bridge() {
    let (_d, db) = open();
    let s = db.call("importBegin", &json!({})).unwrap()["session"].clone();
    assert_eq!(db.call("info", &json!({})).unwrap()["importActive"], true);
    // the lease refuses ordinary writes and a second session
    let e = db
        .call("transact", &json!({ "ops": chunk_ops(9, 1) }))
        .unwrap_err();
    assert_eq!(e.code(), "ImportInProgress");
    assert_eq!(
        db.call("importBegin", &json!({})).unwrap_err().code(),
        "ImportInProgress"
    );
    let r = db
        .call(
            "importChunk",
            &json!({ "session": s, "ops": chunk_ops(0, 3) }),
        )
        .unwrap();
    assert_eq!(r["t"], 1);
    assert_eq!(r["results"][0]["new"], true);
    assert_eq!(r["progress"]["chunks"], 1);
    // a rejected chunk is counted and leaves no trace
    let e = db
        .call(
            "importChunk",
            &json!({ "session": s, "ops": [{ "op": "confirm", "eid": 99 }] }),
        )
        .unwrap_err();
    assert_eq!(e.code(), "NotLive");
    let e = db
        .call(
            "importChunk",
            &json!({ "session": s, "ops": chunk_ops(1, 1), "budget": { "cancelKey": "k" } }),
        )
        .map(|_| ());
    assert!(e.is_ok());
    let p = db.call("importProgress", &json!({ "session": s })).unwrap();
    assert_eq!(p["chunks"], 2);
    assert_eq!(p["rejected"], 1);
    assert_eq!(p["asserted"], 4);
    assert_eq!(p["txs"], json!([1, 2]));
    let f = db.call("importFinish", &json!({ "session": s })).unwrap();
    assert_eq!(f["analyzed"], true);
    assert_eq!(f["statisticsDue"], false);
    assert_eq!(f["maintenanceError"], J::Null);
    assert_eq!(f["progress"]["txs"], json!([1, 2]));
    assert!(f["progress"]["maintenanceMs"].as_f64().unwrap() >= 0.0);
    // the session is gone and writes resume
    assert_eq!(
        db.call("importProgress", &json!({ "session": s }))
            .unwrap_err()
            .code(),
        "InvalidArgument"
    );
    db.call("transact", &json!({ "ops": chunk_ops(2, 1) }))
        .unwrap();

    // cancel keeps the committed chunk and leaves the statistics due
    let s = db.call("importBegin", &json!({})).unwrap()["session"].clone();
    db.call(
        "importChunk",
        &json!({ "session": s, "ops": chunk_ops(3, 2) }),
    )
    .unwrap();
    let p = db.call("importCancel", &json!({ "session": s })).unwrap();
    assert_eq!(p["chunks"], 1);
    let info = db.call("info", &json!({})).unwrap();
    assert_eq!(info["importActive"], false);
    assert_eq!(info["statisticsDue"], true);
    let rows = db
        .call("triples", &json!({ "view": { "kind": "now" } }))
        .unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 7);
    assert_eq!(
        db.call("importChunk", &json!({ "session": "x", "ops": [] }))
            .unwrap_err()
            .code(),
        "InvalidArgument"
    );
}
