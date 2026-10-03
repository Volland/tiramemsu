//! The JavaScript surface in WebAssembly (wasm-bindgen-test under Node.js, the
//! memory VFS): open, calls, thrown errors, export and import, close.

#![cfg(all(target_family = "wasm", target_os = "unknown"))]

use tiramemsu_wasm::js::{import_file, runtime_info, JsDatabase};
use wasm_bindgen::JsValue;
use wasm_bindgen_test::*;

fn config(path: &str, journal: &str) -> String {
    format!(r#"{{"storage":"memory","path":"{path}","journal":"{journal}"}}"#)
}

/// The message of a thrown `Error`.
fn message(e: wasm_bindgen::JsError) -> String {
    let v = JsValue::from(e);
    js_sys::Reflect::get(&v, &"message".into())
        .unwrap()
        .as_string()
        .unwrap()
}

const ASSERT: &str = r#"{"ops":[
  {"op":"assert","s":{"iri":"urn:tiramemsu:v:alice"},"p":{"iri":"urn:tiramemsu:v:knows"},"o":{"iri":"urn:tiramemsu:v:bob"}},
  {"op":"assert","s":{"iri":"urn:tiramemsu:v:bob"},"p":{"iri":"urn:tiramemsu:v:knows"},"o":{"iri":"urn:tiramemsu:v:carol"}}
]}"#;

// @lat: [[tests#WASM SQLite Host#JavaScript API]]
#[wasm_bindgen_test]
async fn database_open_call_export_import_close() {
    let info: serde_json::Value = serde_json::from_str(&runtime_info()).unwrap();
    assert_eq!(info["capabilities"]["vtab"], true);
    assert_eq!(info["capabilities"]["stat4"], false);

    let mut db = JsDatabase::open(config("js.db", "rollback")).await.unwrap();
    let r = db.call("transact", ASSERT).unwrap();
    assert!(r.contains(r#""t":1"#), "{r}");
    let rows = db
        .call(
            "sparql",
            r#"{"view":{"kind":"now"},"text":"SELECT ?o WHERE { v:alice v:knows+ ?o }"}"#,
        )
        .unwrap();
    let rows: serde_json::Value = serde_json::from_str(&rows).unwrap();
    assert_eq!(rows["rows"].as_array().unwrap().len(), 2);
    let caps: serde_json::Value = serde_json::from_str(&db.capabilities().unwrap()).unwrap();
    assert_eq!(caps["storage"], "memory");
    assert_eq!(caps["fts5"], true);

    // refused calls and bridge errors are thrown as tiramemsu:{code,message}
    let e = message(db.call("importBegin", "{}").unwrap_err());
    assert!(
        e.starts_with("tiramemsu:") && e.contains("Unsupported"),
        "{e}"
    );
    let e = message(db.call("sparql", r#"{"text":"SELEC"}"#).unwrap_err());
    assert!(e.contains("Parse"), "{e}");

    let bytes = db.export_file().unwrap();
    db.close();
    assert!(message(db.call("info", "{}").unwrap_err()).contains("closed"));

    import_file(config("js-copy.db", "rollback"), bytes)
        .await
        .unwrap();
    let copy = JsDatabase::open(config("js-copy.db", "rollback"))
        .await
        .unwrap();
    let r = copy.call("transact", ASSERT).unwrap();
    assert!(r.contains(r#""t":2"#), "{r}");
}

// @lat: [[tests#WASM SQLite Host#JavaScript journal errors]]
#[wasm_bindgen_test]
async fn wal_and_opfs_fail_at_open() {
    let e = message(
        JsDatabase::open(config("js-wal.db", "wal"))
            .await
            .err()
            .unwrap(),
    );
    assert!(
        e.contains("MissingCapability") && e.contains("journal_mode=wal"),
        "{e}"
    );
    // OPFS needs a dedicated Web Worker; Node.js has none
    let e = message(
        JsDatabase::open(r#"{"storage":"opfs","path":"o.db","journal":"rollback"}"#.to_string())
            .await
            .err()
            .unwrap(),
    );
    assert!(e.contains("MissingCapability") && e.contains("opfs"), "{e}");
}
