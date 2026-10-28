//! N3 reasoning through the bridge (feature `n3`): `reason`, `ruleSetInfo`,
//! `storeRules` and `materialize`.

use serde_json::{json, Value as J};
use tiramemsu::{Db, OpenOptions, Rules, TimeRef, TxOptions, Valid, Value};
use tiramemsu_json::Database;

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const SOCRATES: &str = "{ ?x a <urn:tiramemsu:v:Human> } => { ?x a <urn:tiramemsu:v:Mortal> }.";

fn open() -> (tempfile::TempDir, Database) {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(dir.path().join("m.db").to_str().unwrap(), &J::Null).unwrap();
    (dir, db)
}

fn v(s: &str) -> J {
    json!({ "iri": format!("urn:tiramemsu:v:{s}") })
}

fn human(db: &Database, who: &str) -> u64 {
    let r = db
        .call(
            "transact",
            &json!({ "ops": [
                { "op": "assert", "s": v(who), "p": { "iri": RDF_TYPE }, "o": v("Human") }
            ]}),
        )
        .unwrap();
    r["results"][0]["eid"].as_u64().unwrap()
}

fn live(db: &Database) -> usize {
    db.call("triples", &json!({}))
        .unwrap()
        .as_array()
        .unwrap()
        .len()
}

// n3-reasoning "Bridge reasoning": the bridge answers with the conclusions and
// proofs of `View::reason`, rendered as bridge terms.
// @tg: verifies:: [[openspec:n3-reasoning#Reasoning surfaces#Bridge reasoning]] {since: "2026-10-28", level: integration}
#[test]
fn bridge_reasoning() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("m.db"), OpenOptions::default()).unwrap();
    let r = db
        .transact(TxOptions::default(), |tx| {
            tx.assert(
                Value::iri("urn:tiramemsu:v:socrates"),
                Value::iri(RDF_TYPE),
                Value::iri("urn:tiramemsu:v:Human"),
                Valid::ALWAYS,
            )?;
            Ok(())
        })
        .unwrap();
    let e1 = r.asserted[0].n();
    // a pinned view, so the run's clock is the same on both sides
    let direct = db
        .as_of(TimeRef::Tx(r.t.0))
        .reason(Rules::Text(SOCRATES.into()), &Default::default())
        .unwrap();
    let expected = tiramemsu_json::n3::reason_json(&direct);
    let bridge = Database::from_db(db);
    let got = bridge
        .call(
            "reason",
            &json!({ "op": "reason", "rules": SOCRATES, "view": { "kind": "asOf", "tx": r.t.0 } }),
        )
        .unwrap();
    assert_eq!(got, expected);
    assert_eq!(
        got["conclusions"],
        json!([{
            "s": v("socrates"), "p": { "iri": RDF_TYPE }, "o": v("Mortal"),
            "validFrom": null, "validTo": null,
            "proof": { "ruleSet": null, "rule": 0, "premises": [{ "eid": e1 }] },
        }])
    );
    assert_eq!(got["status"], "complete");
    assert_eq!(got["loaded"], 1);
    assert_eq!(got["pruned"], true);
    assert_eq!(got["contradiction"], J::Null);
    // reasoning writes nothing
    assert_eq!(live(&bridge), 1);
}

// Options: valid-time intersection, an atemporal run, a window, a graph scope,
// N3 proof text, and the argument and parse errors.
// @tg: verifies:: [[openspec:n3-reasoning#Reasoning surfaces]] {since: "2026-10-28", level: integration}
#[test]
fn reason_options_and_errors() {
    let (_d, db) = open();
    db.call(
        "transact",
        &json!({ "ops": [
            { "op": "assert", "s": v("alice"), "p": v("worksAt"), "o": v("acme"),
              "validFrom": "2020-01-01", "validTo": "2025-01-01" },
            { "op": "assert", "s": v("acme"), "p": v("locatedIn"), "o": v("berlin"),
              "validFrom": "2024-01-01", "as": "loc" },
            { "op": "addToGraph", "eid": { "ref": "loc" }, "graph": v("g1") }
        ]}),
    )
    .unwrap();
    let rule = "{ ?p v:worksAt ?c. ?c v:locatedIn ?l } => { ?p v:worksIn ?l }.";
    let ms = |d: &str| tiramemsu::value::parse_date(d).unwrap() * 86_400_000;
    let r = db.call("reason", &json!({ "rules": rule })).unwrap();
    let c = &r["conclusions"][0];
    assert_eq!(r["conclusions"].as_array().unwrap().len(), 1);
    assert_eq!(c["o"], v("berlin"));
    assert_eq!(
        (c["validFrom"].as_i64(), c["validTo"].as_i64()),
        (Some(ms("2024-01-01")), Some(ms("2025-01-01")))
    );
    assert!(r["segments"].as_u64().unwrap() >= 1);
    let a = db
        .call("reason", &json!({ "rules": rule, "time": "atemporal" }))
        .unwrap();
    assert_eq!(a["conclusions"][0]["validFrom"], J::Null);
    // a window before the overlap concludes nothing
    let w = db
        .call(
            "reason",
            &json!({ "rules": rule, "window": { "from": "2020-01-01", "to": "2023-01-01" } }),
        )
        .unwrap();
    assert_eq!(w["conclusions"], json!([]));
    // only the location is in the graph: nothing to join
    let g = db
        .call(
            "reason",
            &json!({ "rules": rule, "scope": { "graphs": [v("g1")] }, "proofN3": true,
                     "limits": { "maxIterations": 100 }, "maxSegments": 8 }),
        )
        .unwrap();
    assert_eq!(g["conclusions"], json!([]));
    assert!(g["proofN3"].is_string());
    let seeds = db
        .call(
            "reason",
            &json!({ "rules": rule, "scope": { "seeds": { "nodes": [v("alice")], "hops": 2 } } }),
        )
        .unwrap();
    assert_eq!(seeds["conclusions"].as_array().unwrap().len(), 1);
    let code = |args: J| db.call("reason", &args).unwrap_err().code();
    assert_eq!(code(json!({ "rules": "{ ?x a v:Human } => " })), "N3Parse");
    assert_eq!(code(json!({})), "InvalidArgument");
    assert_eq!(
        code(json!({ "rules": rule, "ruleSet": "urn:x" })),
        "InvalidArgument"
    );
    assert_eq!(
        code(json!({ "rules": rule, "time": "sideways" })),
        "InvalidArgument"
    );
    assert_eq!(
        code(json!({ "rules": rule, "scope": "some" })),
        "InvalidArgument"
    );
    assert_eq!(
        code(json!({ "ruleSet": "urn:tiramemsu:v:nothing" })),
        "UnknownRuleSet"
    );
}

// `storeRules` stores, re-storing the same text returns the same statement,
// a stored rule set reasons and is described by `ruleSetInfo`.
// @tg: verifies:: [[openspec:n3-rule-sets#Rule-set surfaces]] {since: "2026-10-28", level: integration}
#[test]
fn store_rules_and_reason_with_a_rule_set() {
    let (_d, db) = open();
    let e1 = human(&db, "socrates");
    let store = |text: &str| {
        db.call(
            "transact",
            &json!({ "ops": [
                { "op": "storeRules", "ruleSet": "urn:tiramemsu:v:mortality", "n3": text, "as": "rs" },
                { "op": "assert", "s": { "ref": "rs" }, "p": v("note"), "o": "the classic" }
            ]}),
        )
    };
    let first = store(SOCRATES).unwrap();
    let rs = first["results"][0]["eid"].as_u64().unwrap();
    assert_eq!(first["refs"]["rs"], rs);
    // equal text: the same statement (the second op adds the same layer again)
    let again = store(SOCRATES).unwrap();
    assert_eq!(again["results"][0]["eid"], rs);
    assert_eq!(again["superseded"], json!([]));
    let r = db
        .call("reason", &json!({ "ruleSet": "urn:tiramemsu:v:mortality" }))
        .unwrap();
    assert_eq!(r["ruleSet"], v("mortality"));
    assert_eq!(r["ruleSetStatement"], rs);
    assert_eq!(
        r["conclusions"][0]["proof"],
        json!({ "ruleSet": v("mortality"), "rule": 0, "premises": [{ "eid": e1 }] })
    );
    let info = db
        .call("ruleSetInfo", &json!({ "ruleSet": v("mortality") }))
        .unwrap();
    assert_eq!(info["statement"], rs);
    assert_eq!(info["text"], SOCRATES);
    assert_eq!(info["rules"], 1);
    assert_eq!(info["monotonicity"], json!({ "kind": "monotonic" }));
    assert_eq!(info["predicates"], json!([RDF_TYPE]));
    let neg =
        "{ ?x a v:Employee. ?SCOPE log:notIncludes { ?x v:manager ?m } } => { ?x a v:Unmanaged }.";
    store(neg).unwrap();
    let info = db
        .call(
            "ruleSetInfo",
            &json!({ "ruleSet": "urn:tiramemsu:v:mortality" }),
        )
        .unwrap();
    assert_eq!(
        info["monotonicity"],
        json!({ "kind": "nonMonotonic", "rule": 0, "builtin": "log:notIncludes" })
    );
    // text that does not parse leaves no trace
    let before = live(&db);
    assert_eq!(store("{ ?x a").unwrap_err().code(), "N3Parse");
    assert_eq!(live(&db), before);
    assert_eq!(
        db.call(
            "transact",
            &json!({ "ops": [{ "op": "storeRules", "ruleSet": "urn:x" }] })
        )
        .unwrap_err()
        .code(),
        "InvalidArgument"
    );
    assert_eq!(
        db.call("ruleSetInfo", &json!({ "ruleSet": "urn:tiramemsu:v:none" }))
            .unwrap_err()
            .code(),
        "UnknownRuleSet"
    );
}

#[test]
fn materialize_rejects_malformed_arguments() {
    let (_d, db) = open();
    for args in [
        json!({}),
        json!({ "ruleSet": "urn:x", "dryRun": "yes" }),
        json!({ "ruleSet": "urn:x", "window": [1, 2] }),
        json!({ "ruleSet": "urn:x", "limits": { "maxDepth": 1 } }),
    ] {
        assert_eq!(
            db.call("materialize", &args).unwrap_err().code(),
            "InvalidArgument",
            "{args}"
        );
    }
}

// n3-materialization surfaces: the bridge writes one conclusion, a re-run finds
// it existing, and a dry run writes nothing.
// @tg: verifies:: [[openspec:n3-materialization#Materialization surfaces]] {since: "2026-10-28", level: integration}
#[test]
fn bridge_materialize() {
    let (_d, db) = open();
    human(&db, "socrates");
    db.call(
        "transact",
        &json!({ "ops": [{ "op": "storeRules", "ruleSet": "urn:tiramemsu:v:mortality", "n3": SOCRATES }] }),
    )
    .unwrap();
    let before = live(&db);
    let dry = db
        .call(
            "materialize",
            &json!({ "ruleSet": "urn:tiramemsu:v:mortality", "dryRun": true }),
        )
        .unwrap();
    assert_eq!(dry["dryRun"], true);
    assert_eq!(dry["derived"].as_array().unwrap().len(), 1);
    assert_eq!(live(&db), before);
    let r = db
        .call(
            "materialize",
            &json!({ "ruleSet": "urn:tiramemsu:v:mortality" }),
        )
        .unwrap();
    assert_eq!(r["ruleSet"], v("mortality"));
    assert_eq!(r["status"], "complete");
    assert_eq!(r["derived"].as_array().unwrap().len(), 1);
    assert!(r["report"]["t"].is_u64());
    let mortal = db.call("triples", &json!({ "o": v("Mortal") })).unwrap();
    assert_eq!(mortal.as_array().unwrap().len(), 1);
    let again = db
        .call(
            "materialize",
            &json!({ "ruleSet": "urn:tiramemsu:v:mortality" }),
        )
        .unwrap();
    assert_eq!(again["derived"], json!([]));
    assert_eq!(again["existing"].as_array().unwrap().len(), 1);
}
