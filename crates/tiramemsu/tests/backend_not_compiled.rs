//! A facade built without the `redb` feature refuses redb files.
#![cfg(not(feature = "redb"))]

use tiramemsu::*;

// @tg: verifies:: [[openspec:backend-selection#Backend compiled out#Default Rust build opens a redb file]] {since: "2026-10-27", level: integration}
#[test]
fn default_rust_build_opens_a_redb_file() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("m.redb");
    // a redb file starts with redb's magic number
    std::fs::write(
        &p,
        [
            b'r', b'e', b'd', b'b', 0x1A, 0x0A, 0xA9, 0x0D, 0x0A, 0, 0, 0,
        ],
    )
    .unwrap();
    let r = Db::open(&p, OpenOptions::default());
    assert!(
        matches!(&r, Err(Error::BackendNotCompiled { backend }) if backend == "redb"),
        "{r:?}"
    );
    let r = Db::create(
        d.path().join("n.redb"),
        Backend::Redb,
        OpenOptions::default(),
    );
    assert!(matches!(r, Err(Error::BackendNotCompiled { .. })), "{r:?}");
}
