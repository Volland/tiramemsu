//! Spec `sql-executor`: the executor boundary, host capabilities, the minimal tier.

mod common;
use common::*;

use tm_core::*;

fn src_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            src_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs" || x == "sql") {
            out.push(p);
        }
    }
}

// Requirement: Required executor operations — The core has no SQLite binding.
// (CI additionally runs `cargo tree -p tm-core -e normal`.)
#[test]
fn core_has_no_sqlite_binding() {
    let manifest =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml")).unwrap();
    let deps = manifest
        .split("[dependencies]")
        .nth(1)
        .unwrap()
        .split("\n[")
        .next()
        .unwrap();
    assert!(!deps.contains("rusqlite"), "{deps}");
    assert!(!deps.contains("libsqlite3-sys"), "{deps}");
    assert!(!deps.contains("sqlite"), "{deps}");
}

// Requirement: The rusqlite host — No rusqlite type crosses into the core.
#[test]
fn no_rusqlite_type_in_core() {
    let mut files = Vec::new();
    src_files(
        std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src")),
        &mut files,
    );
    assert!(!files.is_empty());
    for f in files {
        let text = std::fs::read_to_string(&f).unwrap();
        for needle in ["rusqlite::", "use rusqlite", "extern crate rusqlite"] {
            assert!(!text.contains(needle), "{} uses {needle}", f.display());
        }
    }
}

host_test! {
    /// Reads within a write transaction see earlier writes.
    fn reads_within_write_see_earlier_writes(db) {
        let exec = db.store().executor();
        exec.begin_immediate().unwrap();
        exec.execute(
            "INSERT INTO term(id, tag, lex) VALUES (?1, 0, ?2)",
            &[SqlValue::Integer(99), SqlValue::from("urn:x:fresh")],
        )
        .unwrap();
        let id = exec
            .query_i64("SELECT id FROM term WHERE lex = ?1", &[SqlValue::from("urn:x:fresh")])
            .unwrap();
        assert_eq!(id, Some(99));
        exec.rollback().unwrap();
        // the same through the engine: intern, then look up, before commit
        db.tx(|tx| {
            let id = tx.encode(lit("a string longer than seven bytes"))?;
            assert_eq!(tx.lookup(&lit("a string longer than seven bytes"))?, Some(id));
            Ok(())
        });
    }
}

host_test! {
    /// Savepoint rollback keeps the outer transaction.
    fn savepoint_rollback_keeps_outer(db) {
        let exec = db.store().executor();
        exec.begin_immediate().unwrap();
        exec.execute("INSERT INTO volatile VALUES (1, 2, 3, 4)", &[]).unwrap();
        exec.savepoint("sp").unwrap();
        exec.execute("INSERT INTO volatile VALUES (5, 6, 7, 8)", &[]).unwrap();
        exec.rollback_to("sp").unwrap();
        exec.release("sp").unwrap();
        exec.execute("INSERT INTO volatile VALUES (9, 10, 11, 12)", &[]).unwrap();
        exec.commit().unwrap();
        let rows = db.rows("SELECT s FROM volatile ORDER BY s");
        assert_eq!(rows, vec![vec![SqlValue::Integer(1)], vec![SqlValue::Integer(9)]]);
    }
}

fn open_second(db: &TestDb) -> Box<dyn Executor> {
    let opts = HostOptions::default();
    match db.kind {
        HostKind::Rusqlite => tm_rusqlite::RusqliteHost::new().open_writer(&db.path, &opts),
        HostKind::Minimal => MinimalHost::new().open_writer(&db.path, &opts),
    }
    .unwrap()
}

host_test! {
    /// Stable read snapshot.
    fn stable_read_snapshot(db) {
        db.tx(|tx| tx.assert(iri("a"), iri("p"), iri("b"), Valid::ALWAYS).map(|_| ()));
        let mut other = open_second(db);
        let exec = db.store().executor();
        exec.begin_read().unwrap();
        let before = exec.query_i64("SELECT count(*) FROM tx", &[]).unwrap();
        assert_eq!(before, Some(1));
        other.begin_immediate().unwrap();
        other
            .execute("INSERT INTO tx(t, instant) VALUES (2, 5)", &[])
            .unwrap();
        other.commit().unwrap();
        let after = exec.query_i64("SELECT count(*) FROM tx", &[]).unwrap();
        assert_eq!(after, before);
        exec.commit().unwrap();
        let now = exec.query_i64("SELECT count(*) FROM tx", &[]).unwrap();
        assert_eq!(now, Some(2));
    }
}

// Requirement: Capabilities are declared by the host — Declaration is visible.
#[test]
fn declaration_is_visible() {
    let db = TestDb::new(HostKind::Minimal);
    let mut db = db;
    assert_eq!(db.store().capabilities(), Capabilities::default());
    let mut db2 = TestDb::new(HostKind::Rusqlite);
    assert_eq!(
        db2.store().capabilities(),
        tm_rusqlite::RusqliteHost::new().capabilities()
    );
}

// Requirement: Capabilities are declared by the host — Host without STAT4.
#[test]
fn host_without_stat4_opens_and_keeps_stat1() {
    let mut db = TestDb::new(HostKind::Minimal);
    assert!(!db.store().capabilities().stat4);
    db.tx(|tx| {
        for i in 0..20 {
            tx.assert(
                iri(&format!("s{i}")),
                iri("p"),
                Value::Int(i),
                Valid::ALWAYS,
            )?;
        }
        Ok(())
    });
    assert_ok(db.store().optimize());
    assert!(db.count("sqlite_stat1") > 0);
}

// Requirement: The rusqlite host — rusqlite host declares every capability.
#[test]
fn rusqlite_host_declares_every_capability() {
    let caps = tm_rusqlite::RusqliteHost::new().capabilities();
    assert!(caps.reader_pool && caps.functions && caps.vtab && caps.stat4 && caps.fts5);
    let opts = tm_rusqlite::compile_options();
    assert!(opts.iter().any(|o| o == "ENABLE_STAT4"), "{opts:?}");
    assert!(opts.iter().any(|o| o == "ENABLE_FTS5"), "{opts:?}");
}

// Requirement: A failed transaction leaves no trace on any host — Failed body.
// @lat: [[tests#Storage Invariants#Failed Transactions Leave No Trace]]
host_test! {
    fn failed_body_leaves_no_trace(db) {
        db.tx(|tx| tx.assert(iri("a"), iri("p"), iri("b"), Valid::ALWAYS).map(|_| ()));
        let before = db.snapshot();
        let r = db.try_tx(|tx| {
            tx.assert(iri("x"), iri("q"), lit("a long string value here"), Valid::ALWAYS)?;
            tx.set_volatile(iri("x"), iri("seen"), Value::Int(3))?;
            tx.new_node()?;
            Err(Error::custom("abort"))
        });
        assert_err!(r, Error::Custom(_));
        assert_eq!(db.snapshot(), before);
        let rep = db.tx(|_| Ok(()));
        assert_eq!(rep.t, TxId(2));
    }
}

// Requirement: A failed transaction leaves no trace on any host — Host error.
#[test]
fn host_busy_error_leaves_no_trace() {
    let mut db = TestDb::new(HostKind::Minimal);
    db.tx(|tx| {
        tx.assert(iri("a"), iri("p"), iri("b"), Valid::ALWAYS)
            .map(|_| ())
    });
    let before = db.snapshot();
    db.probe()
        .unwrap()
        .inject("INSERT INTO triple", 1, SqlError::BUSY);
    let r = db.try_tx(|tx| {
        tx.assert(
            iri("x"),
            iri("q"),
            lit("a long string value here"),
            Valid::ALWAYS,
        )?;
        tx.assert(iri("y"), iri("q"), iri("z"), Valid::ALWAYS)?;
        Ok(())
    });
    assert_err!(r, Error::Sqlite(e) if e.code == SqlError::BUSY);
    assert_eq!(db.snapshot(), before);
    let rep = db.tx(|tx| {
        tx.assert(iri("y"), iri("q"), iri("z"), Valid::ALWAYS)
            .map(|_| ())
    });
    assert_eq!(rep.t, TxId(2));
}

// Requirement: The core runs on the required operations alone — No optional
// feature in core SQL. (Every minimal-host test also checks its log on drop.)
// @lat: [[tests#Storage Invariants#Core Runs On A Minimal Host]]
#[test]
fn core_sql_uses_no_optional_feature() {
    let mut db = TestDb::new(HostKind::Minimal);
    db.tx(|tx| {
        let e = tx
            .assert(
                iri("a"),
                iri("p"),
                lit("some long literal text"),
                Valid::ALWAYS,
            )?
            .eid();
        tx.assert(e, iri("conf"), Value::Double(0.5), Valid::ALWAYS)?;
        tx.create(iri("a"), iri("p"), iri("b"), Valid::from(5))?;
        tx.assert(iri("age"), sys("cardinality"), sys("one"), Valid::ALWAYS)?;
        tx.assert(iri("a"), iri("age"), Value::Int(3), Valid::ALWAYS)?;
        tx.assert(iri("a"), iri("age"), Value::Int(4), Valid::ALWAYS)?;
        tx.supersede(e, Patch::object(lit("other")))?;
        tx.set_volatile(iri("a"), iri("seen"), Value::Int(1))?;
        Ok(())
    });
    let _ = db.now(None, None, None);
    let _ = db.triples(ViewSpec::as_of(TimeRef::Instant(T0)), None, None, None);
    let _ = db.events_since(0);
    let log = db.probe().unwrap().log.lock().unwrap().clone();
    assert!(log.len() > 20);
    for sql in &log {
        check_core_sql(sql).unwrap();
    }
    assert!(check_core_sql("SELECT * FROM rarray(?1)").is_err());
    assert!(check_core_sql("SELECT my_udf(o) FROM triple").is_err());
}
