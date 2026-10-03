//! The WASM host's logic on a native target (`Storage::File`): capability probe,
//! core-only tier, operator registration, explicit journal failures, file
//! interchange with `tm-rusqlite` in both directions, crash recovery in a child
//! process and restart. The same scenarios run in WebAssembly under Node.js in
//! `wasm_host.rs`, on the memory VFS.

#![cfg(not(target_family = "wasm"))]

mod common;

use std::path::{Path, PathBuf};

use common::*;
use tiramemsu::*;
use tm_wasm::{Journal, Storage, WasmHost};

fn tmp() -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("memory.db");
    (d, p)
}

fn host(journal: Journal) -> WasmHost {
    WasmHost::new(Storage::File, journal)
}

fn open_wasm(path: &Path, opts: OpenOptions) -> Db {
    Db::open_with_host(host(Journal::Rollback), path, opts).unwrap()
}

fn journal_mode(db: &Db) -> SqlValue {
    db.read_sql("PRAGMA journal_mode")
        .unwrap()
        .remove(0)
        .remove(0)
}

fn core_only() -> Capabilities {
    Capabilities {
        reader_pool: true,
        functions: false,
        vtab: false,
        stat4: true,
        fts5: true,
    }
}

// @lat: [[tests#WASM SQLite Host#Probed capabilities]]
#[test]
fn probe_reports_what_the_runtime_does() {
    let info = tm_wasm::runtime_info();
    let caps = info.capabilities;
    assert!(caps.functions && caps.vtab && caps.fts5);
    assert!(!caps.reader_pool, "the WASM host never declares readers");
    assert_eq!(
        caps.stat4,
        info.compile_options.iter().any(|o| o == "ENABLE_STAT4")
    );
    assert!(!info.sqlite_version.is_empty());
    let (_d, p) = tmp();
    let db = open_wasm(&p, engine_options());
    assert_eq!(db.capabilities(), caps);
    assert_eq!(db.reader_count(), 0);
}

// @lat: [[tests#WASM SQLite Host#Core-only runtime]]
#[test]
fn core_only_runtime_refuses_the_query_engine() {
    let (_d, p) = tmp();
    let limited = host(Journal::Rollback).limit_capabilities(core_only());
    let caps = Host::capabilities(&limited);
    assert!(!caps.functions && !caps.vtab && !caps.reader_pool);
    // the engine is refused before the file is touched
    let err = Db::open_with_host(limited.clone(), &p, engine_options()).unwrap_err();
    assert!(
        matches!(&err, Error::MissingCapability { .. }),
        "expected MissingCapability, got {err:?}"
    );
    assert!(!p.exists());
    // the tm-core tier works: transactions, views, history, bundles
    let db = Db::open_with_host(limited.clone(), &p, core_options()).unwrap();
    let reports = write_fixture(&db);
    check_fixture(&db);
    let bundle = db.now().bundle(reports[0].asserted[1]).unwrap();
    assert!(bundle.to_ntriples().contains("Alice"));
    drop(db);
    // no registration hooks on a connection without functions or vtab
    let mut store = tm_core::Store::open(&limited, &p, tm_core::StoreOptions::default()).unwrap();
    assert!(store.executor().registry().is_none());
}

// @lat: [[tests#WASM SQLite Host#Native operator support]]
#[test]
fn query_engine_registers_every_operator_on_the_connection() {
    let (_d, p) = tmp();
    let db = open_wasm(&p, engine_options());
    write_fixture(&db);
    check_operators(&db);
}

// @lat: [[tests#WASM SQLite Host#Unsupported journal]]
#[test]
fn unsupported_journal_fails_explicitly() {
    // a database the runtime cannot put in WAL mode (an in-memory one keeps
    // journal_mode=memory): the open fails and names what the runtime kept
    let err = Db::open_with_host(host(Journal::Wal), ":memory:", core_options()).unwrap_err();
    match &err {
        Error::MissingCapability { capability } => {
            assert!(capability.contains("journal_mode=wal"), "{capability}");
            assert!(capability.contains("journal_mode=memory"), "{capability}");
        }
        other => panic!("expected MissingCapability, got {other:?}"),
    }
    // the memory and OPFS VFSes exist only in WebAssembly
    for storage in [Storage::Memory, Storage::Opfs] {
        let (_d, p) = tmp();
        let err = Db::open_with_host(
            WasmHost::new(storage, Journal::Rollback),
            &p,
            core_options(),
        )
        .unwrap_err();
        assert!(
            matches!(&err, Error::Unsupported { .. }),
            "expected Unsupported, got {err:?}"
        );
        assert!(!p.exists(), "no database is created");
    }
    // where the runtime can, each policy holds the mode it names, also after the
    // engine's own switch to WAL at open
    let (_d, p) = tmp();
    let db = Db::open_with_host(host(Journal::Wal), &p, core_options()).unwrap();
    assert_eq!(journal_mode(&db), SqlValue::from("wal"));
    drop(db);
    let (_d2, p2) = tmp();
    let db = open_wasm(&p2, core_options());
    write_fixture(&db);
    assert_eq!(journal_mode(&db), SqlValue::from("delete"));
    drop(db);
    let bytes = std::fs::read(&p2).unwrap();
    assert_eq!(&bytes[18..20], &[1, 1], "rollback-journal file header");
}

// @lat: [[tests#WASM SQLite Host#Native interchange]]
#[test]
fn wasm_host_file_opens_natively_and_back() {
    let (_d, p) = tmp();
    let db = open_wasm(&p, core_options());
    let reports = write_fixture(&db);
    let before = dump(&db);
    drop(db);

    // the native host opens the file the WASM host wrote
    let native = Db::open(&p, OpenOptions::default()).unwrap();
    assert_eq!(dump(&native), before, "terms, statements, txs and counters");
    check_fixture(&native);
    assert_eq!(journal_mode(&native), SqlValue::from("wal"));
    continue_history(&native, reports.last().unwrap().t.t());
    let after_native = dump(&native);
    drop(native);
    // closing checkpoints the WAL: the main file alone carries every commit,
    // which is all a browser import gets
    assert!(!PathBuf::from(format!("{}-wal", p.display())).exists());

    // and the WASM host opens the native WAL file again
    let db = open_wasm(&p, engine_options());
    assert_eq!(dump(&db), after_native);
    assert_eq!(journal_mode(&db), SqlValue::from("delete"));
    continue_history(&db, 4);
}

// @lat: [[tests#WASM SQLite Host#Byte export and import]]
#[test]
fn export_and_import_preserve_the_file() {
    let (d, p) = tmp();
    let h = host(Journal::Rollback);
    let db = Db::open_with_host(h.clone(), &p, core_options()).unwrap();
    write_fixture(&db);
    let before = dump(&db);
    let bytes = h.export_file(p.to_str().unwrap()).unwrap();
    drop(db);
    let copy = d.path().join("copy.db");
    h.import_file(copy.to_str().unwrap(), &bytes).unwrap();
    // importing over an existing file and importing garbage are refused
    assert!(h.import_file(copy.to_str().unwrap(), &bytes).is_err());
    assert!(h
        .import_file(d.path().join("x.db").to_str().unwrap(), b"not sqlite")
        .is_err());
    let native = Db::open(&copy, OpenOptions::default()).unwrap();
    assert_eq!(dump(&native), before);
}

/// Child half of the crash test: commits the fixture, then dies inside a large
/// uncommitted transaction (`abort` runs no destructor and no rollback).
fn crash_child(path: &Path) -> ! {
    let db = open_wasm(path, core_options());
    write_fixture(&db);
    let _ = db.transact(TxOptions::default(), |tx| {
        let filler = "x".repeat(1024);
        for i in 0..20_000 {
            tx.assert(
                v(&format!("s{i}")),
                v("filler"),
                lit(&format!("{i} {filler}")),
                Valid::ALWAYS,
            )?;
        }
        // the page cache spilled into the file: a hot journal exists now
        assert!(Path::new(&format!("{}-journal", path.display())).exists());
        std::process::abort();
    });
    unreachable!("the transaction aborts the process");
}

// @lat: [[tests#WASM SQLite Host#Interrupted write]]
#[test]
fn interrupted_write_recovers_the_last_commit() {
    if let Ok(path) = std::env::var("TM_WASM_CRASH_CHILD") {
        crash_child(Path::new(&path));
    }
    let (_d, p) = tmp();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "interrupted_write_recovers_the_last_commit",
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("TM_WASM_CRASH_CHILD", &p)
        .status()
        .unwrap();
    assert!(!status.success(), "the child must die mid-write");
    let journal = PathBuf::from(format!("{}-journal", p.display()));
    assert!(journal.exists(), "the child left a hot journal");

    // reopening rolls the hot journal back: exactly the committed fixture
    let db = open_wasm(&p, core_options());
    let ok = db.read_sql("PRAGMA integrity_check").unwrap();
    assert_eq!(ok[0][0], SqlValue::from("ok"));
    check_fixture(&db);
    let filler = db.now().encode(&v("filler")).unwrap();
    assert!(filler.is_none(), "no term of the lost transaction survives");
    assert!(!journal.exists());
    continue_history(&db, 3);
    drop(db);
    // the native host agrees
    let native = Db::open(&p, OpenOptions::default()).unwrap();
    check_fixture_after_continue(&native);
}

fn check_fixture_after_continue(db: &Db) {
    let txs = db.read_sql("SELECT count(*) FROM tx").unwrap();
    assert_eq!(txs[0][0], SqlValue::Integer(4));
}

// @lat: [[tests#WASM SQLite Host#Restart]]
#[test]
fn restart_sees_the_commit_and_continues_numbering() {
    let (_d, p) = tmp();
    let db = open_wasm(&p, engine_options());
    let r = db
        .transact(TxOptions::default(), |tx| {
            tx.assert(v("worker"), v("state"), lit("started"), Valid::ALWAYS)
                .map(|_| ())
        })
        .unwrap();
    assert_eq!(r.t.t(), 1);
    drop(db);
    let db = open_wasm(&p, engine_options());
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

/// Writes the native half of the cross-target handoff into
/// `$TM_WASM_INTEROP_DIR/native.db` (`scripts/wasm-interop.sh`).
// @lat: [[tests#WASM SQLite Host#Cross-target handoff]]
#[test]
#[ignore = "run by scripts/wasm-interop.sh"]
fn interop_native_side() {
    let dir = PathBuf::from(std::env::var("TM_WASM_INTEROP_DIR").expect("TM_WASM_INTEROP_DIR"));
    match std::env::var("TM_WASM_INTEROP_STEP").as_deref() {
        // step 1: the native host writes the fixture for the WASM build to import
        Ok("write") => {
            let p = dir.join("native.db");
            let _ = std::fs::remove_file(&p);
            let db = Db::open(&p, OpenOptions::default()).unwrap();
            write_fixture(&db);
            std::fs::write(dir.join("native.dump"), dump(&db).join("\n")).unwrap();
        }
        // step 3: the file the WASM build exported (fixture + one more tx) opens natively
        Ok("check") => {
            let p = dir.join("wasm.db");
            let db = Db::open(&p, OpenOptions::default()).unwrap();
            let expected = std::fs::read_to_string(dir.join("wasm.dump")).unwrap();
            assert_eq!(dump(&db).join("\n"), expected);
            check_fixture_after_continue(&db);
            continue_history(&db, 4);
        }
        other => panic!("TM_WASM_INTEROP_STEP must be write or check, got {other:?}"),
    }
}
