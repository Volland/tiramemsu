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
    assert_eq!(r["provenanceGaps"], json!([]));
    let plain = db.call("sparql", &json!({ "text": text })).unwrap();
    assert!(plain.get("provenance").is_none());
    assert!(plain.get("provenanceGaps").is_none());
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

// add-mcp-adapter "Incomplete provenance" and query-only text on the bridge
// @lat: [[tests#Query Provenance#Bridge Reports Provenance Gaps And Query Only]]
#[test]
fn sparql_reports_provenance_gaps_and_refuses_updates_when_query_only() {
    let (_d, db) = open();
    db.call(
        "transact",
        &json!({ "ops": [
            { "op": "assert", "s": v("a"), "p": v("knows"), "o": v("b") },
            { "op": "assert", "s": v("b"), "p": v("knows"), "o": v("c") }
        ] }),
    )
    .unwrap();
    let r = db
        .call(
            "sparql",
            &json!({ "text": "SELECT ?x WHERE { v:a v:knows+ ?x }", "provenance": true }),
        )
        .unwrap();
    assert_eq!(r["rows"].as_array().unwrap().len(), 2);
    assert_eq!(r["provenance"], json!([[], []]));
    assert_eq!(r["provenanceGaps"], json!(["recursivePath"]));
    let e = db
        .call(
            "sparql",
            &json!({ "text": "INSERT DATA { v:x v:y v:z }", "queryOnly": true }),
        )
        .unwrap_err();
    assert_eq!(e.code(), "Unsupported");
    assert_eq!(
        db.call("events", &J::Null)
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let e = db
        .call("sparql", &json!({ "text": "ASK {}", "queryOnly": 1 }))
        .unwrap_err();
    assert_eq!(e.code(), "InvalidArgument");
    let r = db
        .call(
            "sparql",
            &json!({ "text": "ASK { v:a v:knows v:b }", "queryOnly": true }),
        )
        .unwrap();
    assert_eq!(r, json!({ "kind": "ask", "value": true }));
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

// @lat: [[tests#Memory Conflict Review#Bridge Conflicts And Previews]]
#[test]
fn the_bridge_reports_conflicts_and_previews_bundles() {
    let (_d, db) = open();
    let sys = |s: &str| json!({ "iri": format!("urn:tiramemsu:sys:{s}") });
    let r = db
        .call(
            "transact",
            &json!({ "ops": [
                { "op": "meta", "p": sys("author"), "o": v("agent7") },
                { "op": "assert", "s": v("alice"), "p": v("worksAt"), "o": v("acme"), "validFrom": 0, "as": "a" },
                { "op": "assert", "s": {"ref": "a"}, "p": v("confidence"), "o": 0.8 },
                { "op": "assert", "s": v("alice"), "p": v("worksAt"), "o": v("initech"), "validFrom": 10, "validTo": 20 },
                { "op": "assert", "s": v("bob"), "p": v("worksAt"), "o": v("acme"), "validTo": 10 },
                { "op": "assert", "s": v("bob"), "p": v("worksAt"), "o": v("initech"), "validFrom": 10 },
                { "op": "assert", "s": v("email"), "p": sys("unique"), "o": true },
                { "op": "assert", "s": v("carol"), "p": v("email"), "o": "c@x.org" }
            ]}),
        )
        .unwrap();
    let events = db.call("events", &json!({ "since": 0 })).unwrap();
    let c = db.call("conflicts", &json!({})).unwrap();
    assert_eq!(c.as_array().unwrap().len(), 1, "{c}");
    assert_eq!(c[0]["s"], v("alice"));
    assert_eq!(c[0]["declaredMany"], false);
    assert_eq!(
        c[0]["overlaps"],
        json!([{ "validFrom": 10, "validTo": 20 }])
    );
    let acme = &c[0]["values"][0];
    assert_eq!(acme["o"], v("acme"));
    let e = &acme["statements"][0];
    assert_eq!(e["eid"], r["refs"]["a"]);
    assert_eq!(e["confidence"], 0.8);
    assert_eq!(e["authors"], json!([v("agent7")]));
    assert_eq!(e["validTo"], J::Null);
    assert_eq!(c[0]["values"][1]["statements"][0]["confidence"], J::Null);
    // filters: an unknown subject matches nothing; the history view is refused
    let none = db.call("conflicts", &json!({ "s": v("nobody") })).unwrap();
    assert_eq!(none, json!([]));
    let bob = db
        .call("conflicts", &json!({ "s": v("bob"), "limit": 5 }))
        .unwrap();
    assert_eq!(bob, json!([]));
    let err = db
        .call("conflicts", &json!({ "view": { "kind": "history" } }))
        .unwrap_err();
    assert_eq!(err.code(), "Unsupported");
    let err = db
        .call("conflicts", &json!({ "subject": v("alice") }))
        .unwrap_err();
    assert_eq!(err.code(), "InvalidArgument");
    // inspection wrote nothing
    assert_eq!(db.call("events", &json!({ "since": 0 })).unwrap(), events);

    // a bundle from another database, previewed and then applied
    let (_e, src) = open();
    let s = src
        .call(
            "transact",
            &json!({ "ops": [
                { "op": "assert", "s": v("dave"), "p": v("email"), "o": "d@x.org", "as": "m" },
                { "op": "assert", "s": {"ref": "m"}, "p": v("confidence"), "o": 0.5 }
            ]}),
        )
        .unwrap();
    let bundle = src
        .call("bundle", &json!({ "eid": s["refs"]["m"] }))
        .unwrap();
    let p = db
        .call("previewBundle", &json!({ "bundle": bundle }))
        .unwrap();
    assert_eq!(p["wouldCommit"], true, "{p}");
    assert_eq!(p["failure"], J::Null);
    assert_eq!(p["statements"].as_array().unwrap().len(), 2);
    assert_eq!(p["report"]["asserted"], p["burned"]["statements"]);
    assert_eq!(p["scope"]["basis"], r["t"]);
    assert_eq!(p["scope"]["reserved"], false);
    assert_eq!(db.call("events", &json!({ "since": 0 })).unwrap(), events);
    // a schema failure is the preview's outcome, not an error
    let (_f, src2) = open();
    let s2 = src2
        .call(
            "transact",
            &json!({ "ops": [{ "op": "assert", "s": v("eve"), "p": v("email"), "o": "c@x.org", "as": "m" }] }),
        )
        .unwrap();
    let clash = src2
        .call("bundle", &json!({ "eid": s2["refs"]["m"] }))
        .unwrap();
    let p = db
        .call(
            "previewBundle",
            &json!({ "bundle": clash, "budget": { "timeoutMs": 5000 } }),
        )
        .unwrap();
    assert_eq!(p["wouldCommit"], false);
    assert_eq!(p["failure"]["code"], "UniqueViolation");
    assert_eq!(p["statements"], J::Null);
    assert_eq!(db.call("events", &json!({ "since": 0 })).unwrap(), events);
    let err = db
        .call("previewBundle", &json!({ "bundle": { "format": "x" } }))
        .unwrap_err();
    assert_eq!(err.code(), "InvalidTerm");
    // applying is an explicit write that validates again
    let t = db
        .call(
            "transact",
            &json!({ "ops": [{ "op": "importBundle", "bundle": bundle }] }),
        )
        .unwrap();
    assert_eq!(t["asserted"].as_array().unwrap().len(), 2);
    let err = db
        .call(
            "transact",
            &json!({ "ops": [{ "op": "importBundle", "bundle": clash }] }),
        )
        .unwrap_err();
    assert_eq!(err.code(), "UniqueViolation");
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

// @lat: [[tests#Text Retrieval#Bridge Text Recall]]
#[test]
fn text_recall_crosses_the_bridge_with_evidence_and_error_codes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.db");
    let path = path.to_str().unwrap();
    // without the option the index is not built
    let plain = Database::open(path, &J::Null).unwrap();
    plain
        .call("transact", &json!({ "ops": [
            { "op": "assert", "s": v("alice"), "p": v("note"), "o": "met at the lisbon offsite", "as": "n" },
            { "op": "assert", "s": {"ref": "n"}, "p": v("confidence"), "o": 0.9 },
            { "op": "assert", "s": v("bob"), "p": v("note"), "o": "lisbon" }
        ]}))
        .unwrap();
    let e = plain
        .call("textSearch", &json!({ "text": "lisbon" }))
        .unwrap_err();
    assert_eq!(e.code(), "TextIndexUnavailable");
    assert_eq!(
        plain.call("enableTextIndex", &json!({})).unwrap(),
        json!({ "built": true })
    );
    assert_eq!(
        plain.call("rebuildTextIndex", &json!({})).unwrap(),
        json!({ "values": 2 })
    );
    drop(plain);
    let db = Database::open(path, &json!({ "textIndex": true })).unwrap();
    let hits = db
        .call(
            "textSearch",
            &json!({ "text": "lisbon", "view": { "kind": "now" } }),
        )
        .unwrap();
    let hits = hits.as_array().unwrap();
    assert_eq!(hits.len(), 2);
    let alice = hits.iter().find(|h| h["eid"] == 1).unwrap();
    assert_eq!(alice["text"], "met at the lisbon offsite");
    assert_eq!(alice["s"], v("alice"));
    assert_eq!(alice["evidence"]["confidence"], 0.9);
    assert_eq!(alice["evidence"]["tAdd"], 1);
    let bob = hits.iter().find(|h| h["eid"] == 3).unwrap();
    assert_eq!(bob["evidence"]["confidence"], J::Null); // absent, not invented
    assert_eq!(bob["lang"], J::Null);
    assert!(hits[0]["rank"] == 1 && hits[1]["rank"] == 2);
    // options: limit, mode, predicates; a budget bounds the call
    let top = db
        .call(
            "textSearch",
            &json!({ "text": "offsite nowhere", "mode": "any", "limit": 1,
                                      "predicates": [v("note")] }),
        )
        .unwrap();
    assert_eq!(top.as_array().unwrap().len(), 1);
    let none = db
        .call(
            "textSearch",
            &json!({ "text": "lisbon", "graphs": [v("nowhere")] }),
        )
        .unwrap();
    assert_eq!(none, json!([]));
    let e = db
        .call(
            "textSearch",
            &json!({ "text": "lisbon", "budget": { "maxRows": 1 } }),
        )
        .unwrap_err();
    assert_eq!(e.code(), "ResultLimitExceeded");
    for bad in [
        json!({ "text": "lisbon", "mode": "near" }),
        json!({ "text": "lisbon", "nope": 1 }),
        json!({}),
    ] {
        assert_eq!(
            db.call("textSearch", &bad).unwrap_err().code(),
            "InvalidArgument"
        );
    }
    assert_eq!(
        db.call("textSearch", &json!({ "text": "  " }))
            .unwrap_err()
            .code(),
        "InvalidQuery"
    );
    // the query languages reach the same recall through the bridge
    let s = db
        .call(
            "sparql",
            &json!({ "text": "SELECT ?e WHERE { ?e tm:textMatch \"lisbon\" ; tm:textLimit 1 }" }),
        )
        .unwrap();
    assert_eq!(s["rows"].as_array().unwrap().len(), 1);
    let c = db
        .call(
            "cypher",
            &json!({ "text": "CALL tiramemsu.text.search('lisbon') YIELD rank RETURN rank" }),
        )
        .unwrap();
    assert_eq!(c["rows"], json!([[1], [2]]));
    assert_eq!(
        Database::open(path, &json!({ "textIndex": 1 }))
            .unwrap_err()
            .code(),
        "InvalidArgument"
    );
}

// @lat: [[tests#Saved Answers#Bridge Saved Answers]]
#[test]
fn saved_answers_cross_the_bridge() {
    let (_d, db) = open();
    db.call(
        "transact",
        &json!({ "ops": [
            { "op": "assert", "s": v("alice"), "p": v("worksAt"), "o": v("acme") }
        ]}),
    )
    .unwrap();
    let a = db
        .call(
            "saveAnswer",
            &json!({ "name": "employer", "text": "SELECT ?o WHERE { v:alice v:worksAt ?o }" }),
        )
        .unwrap();
    assert_eq!(a["status"], "fresh");
    assert_eq!(a["language"], "sparql");
    assert_eq!(a["view"], json!({ "kind": "now" }));
    assert_eq!(a["dependencies"], json!([1]));
    assert_eq!(a["coverage"], json!(["mutableView"]));
    assert_eq!(a["checkpoint"], 1);
    // the result has the shape of a live `sparql` call
    assert_eq!(a["result"]["kind"], "select");
    assert_eq!(a["result"]["rows"], json!([{ "o": v("acme") }]));
    // a parameterized Cypher answer on a fixed view keeps its parameters and view
    let c = db
        .call(
            "saveAnswer",
            &json!({
                "name": "who",
                "language": "cypher",
                "text": "MATCH (p)-[:worksAt]->(c) WHERE $min >= 0 RETURN count(*) AS n",
                "params": { "min": 1 },
                "view": { "kind": "asOf", "tx": 1 },
            }),
        )
        .unwrap();
    assert_eq!(c["params"], json!({ "min": 1 }));
    assert_eq!(c["view"], json!({ "kind": "asOf", "tx": 1 }));
    assert_eq!(c["coverage"], json!(["noProvenance"]));
    assert_eq!(c["result"]["rows"], json!([[1]]));
    // supersede the cited statement: stale with the triggering event, once
    db.call(
        "transact",
        &json!({ "ops": [
            { "op": "supersede", "eid": 1, "patch": { "o": v("globex") } }
        ]}),
    )
    .unwrap();
    let marks = db.call("checkSavedAnswers", &J::Null).unwrap();
    assert_eq!(marks.as_array().unwrap().len(), 1);
    assert_eq!(marks[0]["name"], "employer");
    assert_eq!(marks[0]["status"], "stale");
    assert_eq!(marks[0]["cause"], "supportRetracted");
    assert_eq!(marks[0]["event"]["eid"], 1);
    assert_eq!(marks[0]["event"]["kind"], "supersede");
    assert_eq!(db.call("checkSavedAnswers", &J::Null).unwrap(), json!([]));
    let stale = db
        .call("savedAnswer", &json!({ "name": "employer" }))
        .unwrap();
    assert_eq!(stale["status"], "stale");
    assert_eq!(stale["invalidation"], marks[0]);
    // a cancelled refresh fails and keeps the mark and the checkpoint
    db.call("cancel", &json!({ "key": "r1" })).unwrap();
    let e = db
        .call(
            "refreshAnswer",
            &json!({ "name": "employer", "budget": { "cancelKey": "r1" } }),
        )
        .unwrap_err();
    assert_eq!(e.code(), "Cancelled");
    let after = db
        .call("savedAnswer", &json!({ "name": "employer" }))
        .unwrap();
    assert_eq!(after["status"], "stale");
    assert_eq!(after["checkpoint"], 1);
    assert!(after["error"].as_str().unwrap().contains("cancel"));
    // a successful refresh clears it
    let fresh = db
        .call("refreshAnswer", &json!({ "name": "employer" }))
        .unwrap();
    assert_eq!(fresh["status"], "fresh");
    assert_eq!(fresh["revision"], 2);
    assert_eq!(fresh["result"]["rows"], json!([{ "o": v("globex") }]));
    assert_eq!(fresh["error"], J::Null);
    let all = db.call("savedAnswers", &J::Null).unwrap();
    assert_eq!(all.as_array().unwrap().len(), 2);
    assert_eq!(
        db.call("deleteSavedAnswer", &json!({ "name": "who" }))
            .unwrap(),
        json!({ "deleted": true })
    );
    assert_eq!(
        db.call("savedAnswer", &json!({ "name": "who" })).unwrap(),
        J::Null
    );
    assert_eq!(
        db.call("refreshAnswer", &json!({ "name": "who" }))
            .unwrap_err()
            .code(),
        "SavedAnswerNotFound"
    );
    for (op, args) in [
        (
            "saveAnswer",
            json!({ "text": "SELECT * WHERE { ?s ?p ?o }" }),
        ),
        (
            "saveAnswer",
            json!({ "name": "x", "language": "gql", "text": "MATCH" }),
        ),
        ("savedAnswer", json!({})),
    ] {
        assert_eq!(db.call(op, &args).unwrap_err().code(), "InvalidArgument");
    }
}

// temporal-language-paths: SPARQL and Cypher temporal syntax with a parameterized
// start, and completeness reporting, through the bridge
// @lat: [[tests#Temporal Path Syntax#Bridge Temporal Paths And Completeness]]
#[test]
fn temporal_path_syntax_and_completeness_cross_the_bridge() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(
        dir.path().join("t.db").to_str().unwrap(),
        &json!({ "pathMaxHops": 2 }),
    )
    .unwrap();
    db.call(
        "transact",
        &json!({ "ops": [
            { "op": "assert", "s": v("a"), "p": v("id"), "o": "a" },
            { "op": "assert", "s": v("a"), "p": v("met"), "o": v("b"), "validFrom": 1, "validTo": 5 },
            { "op": "assert", "s": v("b"), "p": v("met"), "o": v("c"), "validFrom": 3, "validTo": 9 },
            { "op": "assert", "s": v("c"), "p": v("met"), "o": v("d"), "validFrom": 4, "validTo": 9 },
        ]}),
    )
    .unwrap();
    // SPARQL with a start parameter: the rows of the path API
    let sparql = |start: J, completeness: bool| {
        db.call(
            "sparql",
            &json!({
                "text": "SELECT ?y ?t WHERE { SERVICE <urn:tiramemsu:tm:timeRespecting/$start> \
                         { v:a v:met+ ?y . ?y tm:arrival ?t } } ORDER BY ?t",
                "params": { "start": start },
                "pathCompleteness": completeness,
            }),
        )
        .unwrap()
    };
    let r = sparql(json!(2), false);
    let rows: Vec<(J, J)> = r["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| (row["y"].clone(), row["t"].clone()))
        .collect();
    let api = db
        .call(
            "path",
            &json!({ "start": v("a"), "path": "met+", "timeRespecting": { "after": 2 } }),
        )
        .unwrap();
    let api: Vec<(J, J)> = api
        .as_array()
        .unwrap()
        .iter()
        .map(|row| (row["end"].clone(), row["arrival"].clone()))
        .collect();
    assert_eq!(rows, api);
    assert_eq!(
        rows,
        vec![(v("b"), json!(2)), (v("c"), json!(3)), (v("d"), json!(4))]
    );
    // without the flag the response has no completeness member
    assert!(r.get("pathCompleteness").is_none());
    let r = sparql(json!("1970-01-01T00:00:00.002Z"), true);
    assert_eq!(r["rows"].as_array().unwrap().len(), 3);
    assert_eq!(
        r["pathCompleteness"],
        json!({ "kind": "exhaustive", "maxHops": null, "complete": true })
    );
    // Cypher: the same journeys, capped at 2 hops by the database option
    let cy = |completeness: bool| {
        db.call(
            "cypher",
            &json!({
                "text": "MATCH TIME RESPECTING AFTER $start ARRIVAL AS t (x {id:'a'})-[:met*]->(y) \
                         RETURN t ORDER BY t",
                "params": { "start": 2 },
                "pathCompleteness": completeness,
            }),
        )
        .unwrap()
    };
    let r = cy(false);
    assert_eq!(r["rows"], json!([[2], [3]]));
    assert!(r.get("pathCompleteness").is_none());
    assert_eq!(
        cy(true)["pathCompleteness"],
        json!({ "kind": "cap", "maxHops": 2, "complete": false })
    );
    // the path op: a plain array unless the completeness is asked for
    let r = db
        .call(
            "path",
            &json!({ "start": v("a"), "path": "met+", "mode": "trail", "capped": true,
                     "completeness": true }),
        )
        .unwrap();
    assert_eq!(r["rows"].as_array().unwrap().len(), 2);
    assert_eq!(
        r["completeness"],
        json!({ "kind": "cap", "maxHops": 2, "complete": false })
    );
    let r = db
        .call(
            "path",
            &json!({ "start": v("a"), "path": "met+", "maxHops": 1, "completeness": true }),
        )
        .unwrap();
    assert_eq!(
        r["completeness"],
        json!({ "kind": "bound", "maxHops": 1, "complete": true })
    );
    assert!(db
        .call("path", &json!({ "start": v("a"), "path": "met+" }))
        .unwrap()
        .is_array());
    for bad in [
        json!({ "start": v("a"), "path": "met+", "capped": "yes" }),
        json!({ "start": v("a"), "path": "met+", "completeness": 1 }),
    ] {
        assert_eq!(db.call("path", &bad).unwrap_err().code(), "InvalidArgument");
    }
    let e = db
        .call(
            "sparql",
            &json!({ "text": "SELECT ?y WHERE { v:a v:met+ ?y }", "params": { "x": true } }),
        )
        .unwrap_err();
    assert_eq!(e.code(), "InvalidArgument");
}

// cyclic-join-execution over the bridge: `lftj` opts in, `explainSparql` names the
// native route or the fallback reason, and both routes return the same rows
// @lat: [[tests#Cyclic Joins#Bridge Exposes LFTJ Routing]]
#[test]
fn lftj_option_and_explain_over_the_bridge() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("m.db");
    let path = path.to_str().unwrap();
    let plain = Database::open(path, &J::Null).unwrap();
    plain
        .call(
            "sparql",
            &json!({ "text": "INSERT DATA { v:a v:k v:b . v:b v:k v:c . v:c v:k v:a . v:a v:k v:c }" }),
        )
        .unwrap();
    let tri = "SELECT ?x ?y ?z WHERE { ?x v:k ?y . ?y v:k ?z . ?z v:k ?x }";
    let ex = plain
        .call("explainSparql", &json!({ "text": tri }))
        .unwrap();
    let notes: Vec<&J> = ex["regions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| &r["note"])
        .collect();
    assert!(notes.contains(&&json!("cyclicLftjDisabled")), "{ex}");
    assert_eq!(ex["shortCircuit"], false);
    assert!(ex["sql"].as_str().unwrap().starts_with("SELECT"));
    let rows = |db: &Database| {
        let r = db.call("sparql", &json!({ "text": tri })).unwrap();
        let mut rows: Vec<String> = r["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(J::to_string)
            .collect();
        rows.sort();
        rows
    };
    let want = rows(&plain);
    assert_eq!(want.len(), 3);
    drop(plain);
    // estimate threshold above the data: SQL with the reason
    let gated = Database::open(path, &json!({ "lftj": true, "lftjMinRows": 1000 })).unwrap();
    let ex = gated
        .call("explainSparql", &json!({ "text": tri }))
        .unwrap();
    assert_eq!(ex["regions"][0]["kind"], "sql", "{ex}");
    assert_eq!(ex["regions"][0]["note"], "lftjBelowEstimate");
    drop(gated);
    let native = Database::open(path, &json!({ "lftj": true, "lftjMinRows": 0 })).unwrap();
    let ex = native
        .call("explainSparql", &json!({ "text": tri }))
        .unwrap();
    let lftj = ex["regions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["kind"] == "nativeLftj")
        .unwrap_or_else(|| panic!("{ex}"));
    assert_eq!(lftj["note"], "lftjNative");
    assert!(ex["sql"].as_str().unwrap().contains("tm_lftj("));
    assert_eq!(rows(&native), want);
    // a typed option error
    let e = Database::open(path, &json!({ "lftj": 1 })).err().unwrap();
    assert_eq!(e.code(), "InvalidArgument");
    let e = native
        .call(
            "explainSparql",
            &json!({ "text": "INSERT DATA { v:a v:k v:a }" }),
        )
        .unwrap_err();
    assert_eq!(e.code(), "Unsupported");
}
