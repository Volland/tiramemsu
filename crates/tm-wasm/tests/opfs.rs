//! OPFS storage in a real browser: a dedicated Web Worker in headless Chrome,
//! driven by wasm-bindgen-test over WebDriver. Opt-in (feature `opfs-tests`),
//! since it needs Chrome and a matching chromedriver:
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p tm-wasm --target wasm32-unknown-unknown --features opfs-tests --test opfs
//! ```

#![cfg(all(target_family = "wasm", target_os = "unknown", feature = "opfs-tests"))]

mod common;

use common::*;
use tiramemsu::*;
use tm_wasm::{install_opfs, Journal, OpfsOptions, Storage, WasmHost};
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_dedicated_worker);

async fn installed() {
    install_opfs(&OpfsOptions {
        directory: ".tiramemsu-test".to_string(),
        capacity: 12,
        clear_on_init: true,
    })
    .await
    .unwrap();
}

fn opfs(journal: Journal) -> WasmHost {
    WasmHost::new(Storage::Opfs, journal)
}

// @lat: [[tests#WASM SQLite Host#OPFS in a worker]]
#[wasm_bindgen_test]
async fn opfs_keeps_commits_and_refuses_wal() {
    installed().await;
    // the sync-access-handle pool has no shared memory either: WAL is refused
    let err = Db::open_with_host(opfs(Journal::Wal), "/wal.db", core_options()).unwrap_err();
    match &err {
        Error::MissingCapability { capability } => {
            assert!(capability.contains("opfs storage"), "{capability}");
            assert!(capability.contains("journal_mode=wal"), "{capability}");
        }
        other => panic!("expected MissingCapability, got {other:?}"),
    }

    let host = opfs(Journal::Rollback);
    let db = Db::open_with_host(host.clone(), "/memory.db", engine_options()).unwrap();
    write_fixture(&db);
    check_operators(&db);
    let committed = dump(&db);

    // a worker stopped mid-write: what OPFS holds at that moment
    let mut at_stop = None;
    let _ = db.transact(TxOptions::default(), |tx| {
        tx.assert(v("lost"), v("state"), lit("uncommitted"), Valid::ALWAYS)?;
        at_stop = Some(host.export_file("/memory.db")?);
        Err(Error::custom("worker stopped"))
    });
    drop(db);

    // reopening reads the file back from OPFS: committed state, numbering continues
    let db = Db::open_with_host(host.clone(), "/memory.db", engine_options()).unwrap();
    assert_eq!(dump(&db), committed);
    continue_history(&db, 3);
    drop(db);
    host.import_file("/stopped.db", &at_stop.unwrap()).unwrap();
    let db = Db::open_with_host(host, "/stopped.db", core_options()).unwrap();
    assert_eq!(dump(&db), committed);
    assert!(db.now().encode(&v("lost")).unwrap().is_none());
}
