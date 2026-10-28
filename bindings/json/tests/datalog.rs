//! Datalog through the bridge (feature `datalog`): `datalog`, `datalogReason`,
//! `programInfo`, `explainDatalog`, `check` and `storeProgram`.

use serde_json::{json, Value as J};
use tiramemsu_json::Database;

const REPORTS: &str =
    "reports(X, Y) :- v:manager(X, Y).\nreports(X, Z) :- reports(X, Y), v:manager(Y, Z).\n";

fn open() -> (tempfile::TempDir, Database) {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(dir.path().join("m.db").to_str().unwrap(), &J::Null).unwrap();
    (dir, db)
}

fn v(s: &str) -> J {
    json!({ "iri": format!("urn:tiramemsu:v:{s}") })
}

fn chain(db: &Database) {
    db.call(
        "transact",
        &json!({ "ops": [
            { "op": "assert", "s": v("alice"), "p": v("manager"), "o": v("bob") },
            { "op": "assert", "s": v("bob"), "p": v("manager"), "o": v("carol") },
        ]}),
    )
    .unwrap();
}

// @tg: verifies:: [[openspec:datalog-programs#Datalog read surfaces#Bridge goal query]] {level: integration}
#[test]
fn bridge_goal_query() {
    let (_d, db) = open();
    chain(&db);
    let r = db
        .call(
            "datalog",
            &json!({ "program": REPORTS, "goal": "reports(v:alice, X)" }),
        )
        .unwrap();
    assert_eq!(r["status"], "complete");
    assert_eq!(r["columns"], json!(["X"]));
    assert_eq!(r["rows"][0]["values"], json!([v("bob")]));
    assert_eq!(r["rows"][1]["values"], json!([v("carol")]));
    assert_eq!(r["rows"][0]["valid"], json!([{ "from": null, "to": null }]));
}

#[test]
fn bridge_store_program_info_check_reason_explain() {
    let (_d, db) = open();
    chain(&db);
    let policies = format!(
        "{REPORTS}constraint no_self error :- reports(X, X).\nv:above(X, Y) :- reports(Y, X).\n"
    );
    let t = db
        .call(
            "transact",
            &json!({ "ops": [ { "op": "storeProgram", "program": "urn:tiramemsu:v:p", "text": policies } ] }),
        )
        .unwrap();
    assert!(t["results"][0]["eid"].as_u64().is_some());
    let info = db
        .call("programInfo", &json!({ "stored": "urn:tiramemsu:v:p" }))
        .unwrap();
    assert_eq!(info["rules"], 3);
    assert_eq!(
        info["constraints"],
        json!([{ "name": "no_self", "severity": "error" }])
    );
    let c = db
        .call("check", &json!({ "stored": "urn:tiramemsu:v:p" }))
        .unwrap();
    assert_eq!(c["violated"], json!([]));
    assert_eq!(c["checked"], json!(["no_self"]));
    let r = db
        .call(
            "datalogReason",
            &json!({ "stored": "urn:tiramemsu:v:p", "proof": true }),
        )
        .unwrap();
    assert_eq!(r["conclusions"].as_array().unwrap().len(), 3);
    let e = db
        .call(
            "explainDatalog",
            &json!({ "program": REPORTS, "goal": "reports(v:alice, X)" }),
        )
        .unwrap();
    assert!(e["plan"]
        .as_array()
        .unwrap()
        .iter()
        .any(|l| l.as_str().unwrap().starts_with("access")));
    let err = db
        .call("datalog", &json!({ "program": "p(X) :- v:q(X, Y" }))
        .unwrap_err();
    assert_eq!(err.code(), "DatalogParse");
}

// @tg: verifies:: [[openspec:datalog-constraints#Constraint surfaces#Bridge refusal carries violations]] {level: integration}
#[test]
fn bridge_refusal_carries_violations() {
    let (_d, db) = open();
    let ceo = |who: &str| json!({ "op": "assert", "s": v(who), "p": v("role"), "o": v("CEO") });
    db.call("transact", &json!({ "ops": [ceo("a")] })).unwrap();
    let r = db
        .call(
            "transact",
            &json!({ "ops": [
                { "op": "storeProgram", "program": "urn:tiramemsu:v:p",
                  "text": "constraint one_ceo error :- v:role(X, v:CEO), v:role(Y, v:CEO), X != Y." },
                { "op": "enforce", "program": "urn:tiramemsu:v:p" },
            ]}),
        )
        .unwrap();
    assert!(r["results"][1]["eid"].as_u64().is_some());
    let err = db
        .call("transact", &json!({ "ops": [ceo("b")] }))
        .unwrap_err();
    assert_eq!(err.code(), "ConstraintViolated");
    let j = err.to_json();
    assert_eq!(j["constraint"], "one_ceo");
    let vs = j["violations"].as_array().unwrap();
    assert_eq!(vs.len(), 2);
    assert!(vs[0]["bindings"]["X"].is_object());
    assert_eq!(vs[0]["eids"].as_array().unwrap().len(), 2);
    assert_eq!(vs[0]["valid"], json!([{ "from": null, "to": null }]));
    // a report counts the checks
    let ok = db
        .call(
            "transact",
            &json!({ "ops": [ { "op": "assert", "s": v("a"), "p": v("name"), "o": "Ada" } ] }),
        )
        .unwrap();
    assert_eq!(ok["constraintChecks"], 0);
}
