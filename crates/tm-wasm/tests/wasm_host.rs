//! The WASM host in WebAssembly: SQLite compiled for `wasm32-unknown-unknown`
//! (`sqlite-wasm-rs`) on the memory VFS, run under Node.js by wasm-bindgen-test.
//! Every capability and journal result here is what that runtime actually does.
//!
//! Run: `cargo test -p tm-wasm --target wasm32-unknown-unknown --tests` with the matching
//! `wasm-bindgen-test-runner` on `PATH` (see `.cargo/config.toml`).

#![cfg(all(target_family = "wasm", target_os = "unknown"))]

mod common;

use std::sync::atomic::{AtomicU32, Ordering};

use common::*;
use tiramemsu::*;
use tm_wasm::{Journal, Storage, WasmHost};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_test::*;

/// A fresh memory-VFS file name per test (the VFS is shared by the instance).
fn fresh(stem: &str) -> String {
    static N: AtomicU32 = AtomicU32::new(0);
    format!("{stem}-{}.db", N.fetch_add(1, Ordering::Relaxed))
}

fn memory() -> WasmHost {
    WasmHost::new(Storage::Memory, Journal::Rollback)
}

fn open(host: &WasmHost, path: &str, opts: OpenOptions) -> Db {
    Db::open_with_host(host.clone(), path, opts).unwrap()
}

// @lat: [[tests#WASM SQLite Host#WebAssembly capabilities]]
#[wasm_bindgen_test]
fn runtime_capabilities_are_probed() {
    let info = tm_wasm::runtime_info();
    console_log!("sqlite {} {:?}", info.sqlite_version, info.capabilities);
    let caps = info.capabilities;
    // functions, eponymous virtual tables and FTS5 work in this runtime ...
    assert!(caps.functions && caps.vtab && caps.fts5);
    // ... STAT4 is not compiled in, and there are no WAL readers
    assert_eq!(
        caps.stat4,
        info.compile_options.iter().any(|o| o == "ENABLE_STAT4")
    );
    assert!(!caps.stat4);
    assert!(!caps.reader_pool);
    assert!(info.compile_options.iter().any(|o| o == "THREADSAFE=0"));
    let db = open(&memory(), &fresh("caps"), engine_options());
    assert_eq!(db.capabilities(), caps);
    // statistics still work without STAT4: ANALYZE fills sqlite_stat1
    write_fixture(&db);
    db.optimize().unwrap();
    drop(db);
    let path = fresh("stats");
    let db = open(&memory(), &path, core_options());
    write_fixture(&db);
    drop(db);
    // reopening a file with statements and no STAT4 samples runs a full ANALYZE
    let db = open(&memory(), &path, core_options());
    let stat1 = db.read_sql("SELECT count(*) FROM sqlite_stat1").unwrap();
    assert!(stat1[0][0].as_i64().unwrap() > 0);
}

// @lat: [[tests#WASM SQLite Host#WebAssembly core-only runtime]]
#[wasm_bindgen_test]
fn core_only_runtime_refuses_the_query_engine() {
    let limited = memory().limit_capabilities(Capabilities {
        reader_pool: true,
        functions: false,
        vtab: false,
        stat4: true,
        fts5: true,
    });
    let path = fresh("core");
    let err = Db::open_with_host(limited.clone(), &path, engine_options()).unwrap_err();
    assert!(matches!(err, Error::MissingCapability { .. }), "{err:?}");
    let db = open(&limited, &path, core_options());
    write_fixture(&db);
    check_fixture(&db);
}

// @lat: [[tests#WASM SQLite Host#WebAssembly operator support]]
#[wasm_bindgen_test]
fn query_engine_runs_in_webassembly() {
    let db = open(&memory(), &fresh("engine"), engine_options());
    write_fixture(&db);
    check_operators(&db);
}

// @lat: [[tests#WASM SQLite Host#WAL refused on the memory VFS]]
#[wasm_bindgen_test]
fn wal_on_the_memory_vfs_fails_explicitly() {
    let host = WasmHost::new(Storage::Memory, Journal::Wal);
    let err = Db::open_with_host(host, fresh("wal"), core_options()).unwrap_err();
    match &err {
        // the memory VFS has no shared memory: SQLite keeps a rollback journal
        Error::MissingCapability { capability } => {
            assert!(capability.contains("journal_mode=wal"), "{capability}");
            assert!(capability.contains("memory storage"), "{capability}");
            assert!(capability.contains("journal_mode=delete"), "{capability}");
        }
        other => panic!("expected MissingCapability, got {other:?}"),
    }
    // the OPFS VFS is not installed under Node.js, and there is no file system
    let err = Db::open_with_host(
        WasmHost::new(Storage::Opfs, Journal::Rollback),
        fresh("opfs"),
        core_options(),
    )
    .unwrap_err();
    assert!(matches!(err, Error::MissingCapability { .. }), "{err:?}");
    let err = Db::open_with_host(
        WasmHost::new(Storage::File, Journal::Rollback),
        fresh("file"),
        core_options(),
    )
    .unwrap_err();
    assert!(matches!(err, Error::Unsupported { .. }), "{err:?}");
}

// @lat: [[tests#WASM SQLite Host#WebAssembly byte export and import]]
#[wasm_bindgen_test]
fn export_and_import_preserve_the_file() {
    let host = memory();
    let path = fresh("export");
    let db = open(&host, &path, core_options());
    write_fixture(&db);
    let before = dump(&db);
    let bytes = host.export_file(&path).unwrap();
    drop(db);
    assert!(bytes.starts_with(b"SQLite format 3\0"));
    let copy = fresh("copy");
    host.import_file(&copy, &bytes).unwrap();
    assert!(host.import_file(&copy, &bytes).is_err());
    let db = open(&host, &copy, engine_options());
    assert_eq!(dump(&db), before);
    continue_history(&db, 3);
}

// @lat: [[tests#WASM SQLite Host#WebAssembly interrupted write]]
#[wasm_bindgen_test]
fn interrupted_write_recovers_the_last_commit() {
    let host = memory();
    let path = fresh("crash");
    let db = open(&host, &path, core_options());
    write_fixture(&db);
    let committed = dump(&db);
    // the worker stops in the middle of a write: what storage holds at that
    // moment is all a restarted worker gets
    let mut at_stop = None;
    let _ = db.transact(TxOptions::default(), |tx| {
        tx.assert(v("lost"), v("state"), lit("uncommitted"), Valid::ALWAYS)?;
        at_stop = Some(host.export_file(&path)?);
        Err(Error::custom("worker stopped"))
    });
    drop(db);
    let restarted = fresh("restarted");
    host.import_file(&restarted, &at_stop.unwrap()).unwrap();
    let db = open(&host, &restarted, core_options());
    assert_eq!(dump(&db), committed);
    assert!(db.now().encode(&v("lost")).unwrap().is_none());
    continue_history(&db, 3);
}

// @lat: [[tests#WASM SQLite Host#WebAssembly restart]]
#[wasm_bindgen_test]
fn restart_sees_the_commit_and_continues_numbering() {
    let host = memory();
    let path = fresh("restart");
    let db = open(&host, &path, engine_options());
    let r = db
        .transact(TxOptions::default(), |tx| {
            tx.assert(v("worker"), v("state"), lit("started"), Valid::ALWAYS)
                .map(|_| ())
        })
        .unwrap();
    assert_eq!(r.t.t(), 1);
    drop(db);
    let db = open(&host, &path, engine_options());
    let worker = db.now().encode(&v("worker")).unwrap().expect("committed");
    assert_eq!(db.now().triples(Some(worker), None, None).unwrap().len(), 1);
    let r = db
        .transact(TxOptions::default(), |tx| {
            tx.assert(v("worker"), v("state"), lit("restarted"), Valid::ALWAYS)
                .map(|_| ())
        })
        .unwrap();
    assert_eq!(r.t.t(), 2);
}

/// Node's `fs` and `process.env`, through `process.getBuiltinModule`, so the test
/// works whatever module format the runner emits.
fn node(name: &str) -> Option<JsValue> {
    let process = js_sys::Reflect::get(&js_sys::global(), &"process".into()).ok()?;
    if process.is_undefined() {
        return None;
    }
    match name {
        "env" => js_sys::Reflect::get(&process, &"env".into()).ok(),
        module => {
            let get = js_sys::Reflect::get(&process, &"getBuiltinModule".into()).ok()?;
            get.dyn_into::<js_sys::Function>()
                .ok()?
                .call1(&process, &module.into())
                .ok()
        }
    }
}

fn env(key: &str) -> Option<String> {
    js_sys::Reflect::get(&node("env")?, &key.into())
        .ok()?
        .as_string()
}

fn call(target: &JsValue, method: &str, args: &[JsValue]) -> JsValue {
    let f: js_sys::Function = js_sys::Reflect::get(target, &method.into())
        .unwrap()
        .dyn_into()
        .unwrap();
    f.apply(target, &args.iter().cloned().collect::<js_sys::Array>())
        .unwrap()
}

/// Step 2 of `scripts/wasm-interop.sh`: imports the native file, checks it
/// against the native dump, adds a transaction and exports the result for the
/// native side to open. Skipped unless `TM_WASM_INTEROP_DIR` is set.
// @lat: [[tests#WASM SQLite Host#WebAssembly side of the handoff]]
#[wasm_bindgen_test]
fn interop_wasm_side() {
    let Some(dir) = env("TM_WASM_INTEROP_DIR") else {
        console_log!("TM_WASM_INTEROP_DIR not set: cross-target handoff skipped");
        return;
    };
    let fs = node("fs").expect("node fs");
    let read = |name: &str| -> Vec<u8> {
        let buf = call(&fs, "readFileSync", &[format!("{dir}/{name}").into()]);
        js_sys::Uint8Array::new(&buf).to_vec()
    };
    let host = memory();
    let path = fresh("from-native");
    host.import_file(&path, &read("native.db")).unwrap();
    let db = open(&host, &path, engine_options());
    let native_dump = String::from_utf8(read("native.dump")).unwrap();
    assert_eq!(dump(&db).join("\n"), native_dump, "native file in WASM");
    check_fixture(&db);
    continue_history(&db, 3);
    let out = dump(&db).join("\n");
    let bytes = host.export_file(&path).unwrap();
    drop(db);
    let arr = js_sys::Uint8Array::from(bytes.as_slice());
    call(
        &fs,
        "writeFileSync",
        &[format!("{dir}/wasm.db").into(), arr.into()],
    );
    call(
        &fs,
        "writeFileSync",
        &[format!("{dir}/wasm.dump").into(), out.into()],
    );
}
