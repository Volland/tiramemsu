//! The WASM binding's logic on a native target (`"storage": "file"`): the
//! configuration, the JSON bridge on the WASM host, refused calls and the file
//! export. `js.rs` drives the same surface through wasm-bindgen in WebAssembly.

#![cfg(not(target_family = "wasm"))]

use serde_json::{json, Value as J};
use tiramemsu_wasm::{import_file, runtime_info, Config, Worker};

fn v(s: &str) -> J {
    json!({ "iri": format!("urn:tiramemsu:v:{s}") })
}

fn config(dir: &tempfile::TempDir, extra: J) -> Config {
    let mut c = json!({
        "storage": "file",
        "path": dir.path().join("worker.db").to_str().unwrap(),
        "journal": "rollback",
    });
    for (k, val) in extra.as_object().unwrap() {
        c[k] = val.clone();
    }
    Config::from_json(&c).unwrap()
}

fn code(e: &str) -> String {
    serde_json::from_str::<J>(e).unwrap()["code"]
        .as_str()
        .unwrap()
        .to_string()
}

// @lat: [[tests#WASM SQLite Host#Worker binding]]
#[test]
fn worker_serves_the_json_bridge_on_the_wasm_host() {
    let dir = tempfile::tempdir().unwrap();
    let w = Worker::open(&config(&dir, json!({}))).unwrap();
    let r = w
        .call(
            "transact",
            &json!({ "ops": [
                { "op": "assert", "s": v("alice"), "p": v("knows"), "o": v("bob") },
                { "op": "assert", "s": v("bob"), "p": v("knows"), "o": v("carol") },
            ]}),
        )
        .unwrap();
    assert_eq!(r["t"], 1);
    let rows = w
        .call(
            "sparql",
            &json!({ "view": {"kind": "now"}, "text": "SELECT ?o WHERE { v:alice v:knows+ ?o }" }),
        )
        .unwrap();
    assert_eq!(rows["rows"].as_array().unwrap().len(), 2);
    let caps = w.capabilities();
    assert_eq!(caps["storage"], "file");
    assert_eq!(caps["journal"], "rollback");
    assert_eq!(caps["readerPool"], false);
    assert_eq!(caps["queryEngine"], true);
    assert_eq!(runtime_info()["capabilities"]["vtab"], true);

    // the committed file goes out as bytes and comes back as another database
    let bytes = w.export_file().unwrap();
    let copy = json!({
        "storage": "file",
        "path": dir.path().join("copy.db").to_str().unwrap(),
        "journal": "rollback",
    });
    let copy = Config::from_json(&copy).unwrap();
    import_file(&copy, &bytes).unwrap();
    let w2 = Worker::open(&copy).unwrap();
    let r = w2
        .call_text(
            "transact",
            r#"{"ops":[{"op":"assert","s":{"iri":"urn:x:a"},"p":{"iri":"urn:x:p"},"o":"after import"}]}"#,
        )
        .unwrap();
    assert!(r.contains(r#""t":2"#), "{r}");
}

// @lat: [[tests#WASM SQLite Host#Worker refusals]]
#[test]
fn worker_refuses_what_cannot_run_in_webassembly() {
    let dir = tempfile::tempdir().unwrap();
    let w = Worker::open(&config(&dir, json!({}))).unwrap();
    for (op, args) in [
        ("importBegin", "{}"),
        ("cancel", r#"{"key":"k"}"#),
        (
            "triples",
            r#"{"view":{"kind":"now"},"budget":{"maxRows":10}}"#,
        ),
    ] {
        let e = w.call_text(op, args).unwrap_err();
        assert_eq!(code(&e), "Unsupported", "{op}: {e}");
    }
    // a worker opened without the engine refuses queries (the facade's own error)
    let core = tempfile::tempdir().unwrap();
    let w = Worker::open(&config(&core, json!({"queryEngine": false}))).unwrap();
    assert_eq!(w.capabilities()["queryEngine"], false);
    let e = w
        .call_text("sparql", r#"{"view":{"kind":"now"},"text":"ASK {}"}"#)
        .unwrap_err();
    assert_eq!(code(&e), "Unsupported", "{e}");
    // an unsupported journal mode is an open error, never another mode
    let e = Worker::open(
        &Config::from_json(&json!({"storage": "file", "path": ":memory:", "journal": "wal"}))
            .unwrap(),
    )
    .unwrap_err();
    assert_eq!(e.code(), "MissingCapability");
}

// @lat: [[tests#WASM SQLite Host#Worker configuration]]
#[test]
fn configuration_is_validated() {
    for bad in [
        json!({"storage": "disk", "path": "a.db", "journal": "rollback"}),
        json!({"storage": "memory", "path": "", "journal": "rollback"}),
        json!({"storage": "memory", "path": "a.db"}),
        json!({"storage": "memory", "path": "a.db", "journal": "rollback", "extra": 1}),
        json!({"storage": "memory", "path": "a.db", "journal": "rollback", "options": {"readers": 2}}),
        json!({"storage": "memory", "path": "a.db", "journal": "rollback", "queryEngine": "yes"}),
    ] {
        let e = Config::from_json(&bad).unwrap_err();
        assert_eq!(e.code(), "InvalidArgument", "{bad}");
    }
    // the memory and OPFS storages exist only in WebAssembly
    let e = Worker::open(
        &Config::from_json(&json!({"storage": "memory", "path": "a.db", "journal": "rollback"}))
            .unwrap(),
    )
    .unwrap_err();
    assert_eq!(e.code(), "Unsupported");
}
