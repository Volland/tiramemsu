//! Datalog operations in a bridge built without the feature `datalog`.
#![cfg(not(feature = "datalog"))]

use serde_json::{json, Value as J};
use tiramemsu_json::Database;

// @tg: verifies:: [[openspec:datalog-programs#Datalog read surfaces#Bridge without the feature]] {level: integration}
#[test]
fn bridge_without_the_feature() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(dir.path().join("m.db").to_str().unwrap(), &J::Null).unwrap();
    for op in ["datalog", "datalogReason", "programInfo", "explainDatalog", "check"] {
        let e = db.call(op, &json!({ "program": "p(X) :- v:q(X, X)." })).unwrap_err();
        assert_eq!(e.code(), "Unsupported", "{op}");
    }
}
