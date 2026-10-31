//! ELPI through the bridge (feature `elpi`): `elpi`, `elpiCheck`,
//! `elpiConclude`, `elpiProgramInfo`, `storeElpiProgram`, `enforceElpi` and
//! `materializeElpi`; without the feature every one fails with `Unsupported`.

use serde_json::{json, Value as J};
use tiramemsu_json::Database;

fn open() -> (tempfile::TempDir, Database) {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(dir.path().join("m.db").to_str().unwrap(), &J::Null).unwrap();
    (dir, db)
}

#[cfg(feature = "elpi")]
fn v(s: &str) -> J {
    json!({ "iri": format!("urn:tiramemsu:v:{s}") })
}

#[cfg(feature = "elpi")]
const GRAND: &str = "tm.conclude (triple X (iri \"urn:tiramemsu:v:grandparent\") Z) [E1, E2] :-\n  tm.triple X (iri \"urn:tiramemsu:v:parent\") Y E1,\n  tm.triple Y (iri \"urn:tiramemsu:v:parent\") Z E2.\n";

// @tg: verifies:: [[openspec:elpi-query-calls#ELPI bridge operations#Bridge round trip]] {level: integration}
#[test]
#[cfg(feature = "elpi")]
fn bridge_round_trip() {
    let (_d, db) = open();
    db.call(
        "transact",
        &json!({ "ops": [
            { "op": "assert", "s": v("a"), "p": v("parent"), "o": v("b") },
            { "op": "assert", "s": v("b"), "p": v("parent"), "o": v("c") },
            { "op": "storeElpiProgram", "program": "urn:tiramemsu:v:g", "text": GRAND },
        ]}),
    )
    .unwrap();
    let q = db
        .call(
            "elpi",
            &json!({ "stored": "urn:tiramemsu:v:g",
                     "goal": "tm.triple (iri \"urn:tiramemsu:v:a\") _ Y _" }),
        )
        .unwrap();
    assert_eq!(q["rows"], json!([{ "Y": v("b") }]));
    let c = db
        .call("elpiConclude", &json!({ "stored": "urn:tiramemsu:v:g" }))
        .unwrap();
    assert_eq!(c["conclusions"][0]["o"], v("c"));
    assert_eq!(c["conclusions"][0]["support"].as_array().unwrap().len(), 2);
    let info = db
        .call(
            "elpiProgramInfo",
            &json!({ "program": "urn:tiramemsu:v:g" }),
        )
        .unwrap();
    assert_eq!(info["enforced"], json!(false));
    let m = db
        .call("materializeElpi", &json!({ "stored": "urn:tiramemsu:v:g" }))
        .unwrap();
    assert_eq!(m["derived"].as_array().unwrap().len(), 1);
    let chk = db
        .call(
            "elpiCheck",
            &json!({ "program": "tm.violation warn \"w\" \"any parent\" [E] :- tm.triple _ (iri \"urn:tiramemsu:v:parent\") _ E." }),
        )
        .unwrap();
    assert_eq!(chk["violations"].as_array().unwrap().len(), 2);
    let e = db
        .call(
            "transact",
            &json!({ "ops": [ { "op": "enforceElpi", "program": "urn:tiramemsu:v:g" } ] }),
        )
        .unwrap();
    assert!(e["results"][0]["eid"].as_u64().is_some(), "{e}");
}

// @tg: verifies:: [[openspec:elpi-query-calls#ELPI bridge operations#Bridge without the feature]] {level: integration}
#[test]
#[cfg(not(feature = "elpi"))]
fn bridge_without_the_feature() {
    let (_d, db) = open();
    for op in [
        "elpi",
        "elpiCheck",
        "elpiConclude",
        "elpiProgramInfo",
        "materializeElpi",
    ] {
        let e = db
            .call(op, &json!({ "program": "p.", "goal": "p" }))
            .unwrap_err();
        assert_eq!(e.code(), "Unsupported", "{op}");
    }
}
