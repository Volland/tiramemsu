//! Spec `storage-format`: file creation, the format-1 schema, counters, statistics,
//! format versions, migrations and foreign files.

mod common;
use common::*;

use std::collections::BTreeSet;

use tm_core::storage::{self, migrate::Migration};
use tm_core::*;
use tm_rusqlite::RusqliteHost;

fn names(conn: &rusqlite::Connection, ty: &str) -> BTreeSet<String> {
    let mut st = conn
        .prepare("SELECT name FROM sqlite_schema WHERE type = ?1 AND name NOT LIKE 'sqlite_%'")
        .unwrap();
    st.query_map([ty], |r| r.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn set(v: &[&str]) -> BTreeSet<String> {
    v.iter().map(|s| s.to_string()).collect()
}

host_test! {
    /// Opening creates a new database file — Fresh file gets the full schema.
    fn fresh_file_gets_full_schema(db) {
        assert!(db.path.exists());
        let raw = db.raw();
        assert_eq!(
            names(&raw, "table"),
            set(&["meta", "term", "tx", "triple", "volatile", "pred_multi"])
        );
        assert_eq!(
            names(&raw, "index"),
            set(&[
                "term_key", "term_num", "tx_instant", "live_spo", "live_pos", "live_osp",
                "hist_spo", "hist_pos", "hist_osp", "valid_p", "log_add", "log_ret",
            ])
        );
        assert_eq!(names(&raw, "view"), set(&["event"]));
        let triggers = names(&raw, "trigger");
        for t in [
            "triple_no_delete", "triple_retract_once", "term_no_delete", "term_no_update",
            "tx_no_delete", "tx_no_update", "triple_no_replace", "term_no_replace", "tx_no_replace",
        ] {
            assert!(triggers.contains(t), "{t}");
        }
        assert_eq!(triggers.len(), 9);
    }
}

host_test! {
    /// Opening creates a new database file — Fresh database is empty.
    fn fresh_database_is_empty(db) {
        for t in ["tx", "triple", "term", "volatile"] {
            assert_eq!(db.count(t), 0, "{t}");
        }
        assert!(db.now(None, None, None).is_empty());
        assert!(db.history_all().is_empty());
        for at in [TimeRef::Tx(0), TimeRef::Tx(5), TimeRef::Instant(i64::MAX)] {
            assert!(db.triples(ViewSpec::as_of(at), None, None, None).is_empty());
        }
    }
}

// Opening creates a new database file — Interrupted creation leaves no
// half-initialised file (fault injected after the first DDL statement).
#[test]
fn interrupted_creation_leaves_no_half_initialised_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("x.db");
    let host = MinimalHost::new();
    host.probe.inject("CREATE ", 1, SqlError::FULL);
    let r = Store::open(&host, &path, StoreOptions::default());
    assert_err!(r, Error::Sqlite(e) if e.code == SqlError::FULL);
    {
        let raw = rusqlite::Connection::open(&path).unwrap();
        let n: i64 = raw
            .query_row("SELECT count(*) FROM sqlite_schema", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
    }
    let mut store = Store::open(&host, &path, StoreOptions::default()).unwrap();
    let n = store
        .executor()
        .query_i64(
            "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
            &[],
        )
        .unwrap();
    assert_eq!(n, Some(6));
}

host_test! {
    /// Tables are STRICT and the journal is WAL.
    fn strict_tables_and_wal(db) {
        let mode = db.rows("PRAGMA journal_mode");
        assert_eq!(mode[0][0], SqlValue::from("wal"));
        let raw = db.raw();
        let err = raw
            .execute(
                "INSERT INTO triple(eid, s, p, o, t_add) VALUES (19, 'text', 1, 2, 1)",
                [],
            )
            .unwrap_err();
        assert!(err.to_string().contains("cannot store TEXT"), "{err}");
        let sync = db.rows("PRAGMA synchronous");
        assert_eq!(sync[0][0], SqlValue::Integer(1));
    }
}

host_test! {
    /// Schema matches format version 1 exactly (DDL text, partial indexes, key order).
    fn schema_matches_format_1(db) {
        let raw = db.raw();
        let mut st = raw
            .prepare("SELECT sql FROM sqlite_schema WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%'")
            .unwrap();
        let stored: BTreeSet<String> = st
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let expected: BTreeSet<String> = storage::split_statements(storage::DDL_V1)
            .into_iter()
            .map(|s| s.trim_end_matches(';').trim().to_string())
            .collect();
        assert_eq!(stored, expected);
        // partial indexes
        let partial = |name: &str| -> String {
            raw.query_row("SELECT sql FROM sqlite_schema WHERE name = ?1", [name], |r| r.get(0))
                .unwrap()
        };
        for i in ["live_spo", "live_pos", "live_osp", "valid_p"] {
            assert!(partial(i).ends_with("WHERE t_ret IS NULL"), "{i}");
        }
        assert!(partial("log_ret").ends_with("WHERE t_ret IS NOT NULL"));
        assert!(partial("term_num").ends_with("WHERE num IS NOT NULL"));
        // history indexes: t_add descending after the three positions
        for (i, cols) in [("hist_spo", ["s", "p", "o"]), ("hist_pos", ["p", "o", "s"]), ("hist_osp", ["o", "s", "p"])] {
            let mut st = raw.prepare(&format!("PRAGMA index_xinfo({i})")).unwrap();
            let info: Vec<(Option<String>, i64)> = st
                .query_map([], |r| Ok((r.get::<_, Option<String>>(2)?, r.get::<_, i64>(3)?)))
                .unwrap()
                .map(Result::unwrap)
                .collect();
            assert_eq!(info[0].0.as_deref(), Some(cols[0]));
            assert_eq!(info[1].0.as_deref(), Some(cols[1]));
            assert_eq!(info[2].0.as_deref(), Some(cols[2]));
            assert_eq!(info[3], (Some("t_add".to_string()), 1));
        }
        // volatile: WITHOUT ROWID STRICT with primary key (s, key)
        assert!(partial("volatile").ends_with("WITHOUT ROWID, STRICT"));
        let mut st = raw.prepare("PRAGMA table_info(volatile)").unwrap();
        let pk: Vec<(String, i64)> = st
            .query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, i64>(5)?)))
            .unwrap()
            .map(Result::unwrap)
            .filter(|(_, k)| *k > 0)
            .collect();
        assert_eq!(pk, vec![("s".to_string(), 1), ("key".to_string(), 2)]);
    }
}

host_test! {
    /// Schema matches format version 1 exactly — Live indexes keep t_ret in their key.
    fn live_indexes_keep_t_ret(db) {
        let raw = db.raw();
        for (i, cols) in [
            ("live_spo", ["s", "p", "o"]),
            ("live_pos", ["p", "o", "s"]),
            ("live_osp", ["o", "s", "p"]),
        ] {
            let mut st = raw.prepare(&format!("PRAGMA index_info({i})")).unwrap();
            let got: Vec<String> = st
                .query_map([], |r| r.get::<_, String>(2))
                .unwrap()
                .map(Result::unwrap)
                .collect();
            let mut want: Vec<String> = cols.iter().map(|s| s.to_string()).collect();
            want.extend(["t_ret", "v_from", "v_to"].map(String::from));
            assert_eq!(got, want, "{i}");
        }
        let plan = db.rows(
            "EXPLAIN QUERY PLAN SELECT o, eid FROM triple WHERE s = 1 AND p = 2 AND t_ret IS NULL",
        );
        let detail = format!("{plan:?}");
        assert!(detail.contains("COVERING INDEX live_spo"), "{detail}");
    }
}

fn reserved_absent(db: &mut TestDb) {
    let rows = db.rows("SELECT name FROM sqlite_schema");
    for r in rows {
        let n = r[0].as_str().unwrap();
        assert!(
            n != "seal_key" && n != "term_fts" && !n.starts_with("vec_"),
            "{n}"
        );
    }
}

host_test! {
    /// Format 1 reserves names for later milestones.
    fn reserved_names_are_absent(db) {
        reserved_absent(db);
        db.tx(|tx| tx.assert(iri("a"), iri("p"), iri("b"), Valid::ALWAYS).map(|_| ()));
        db.reopen();
        reserved_absent(db);
    }
}

host_test! {
    /// Engine metadata counters — Fresh counters.
    fn fresh_counters(db) {
        let rows = db.rows("SELECT key, value FROM meta ORDER BY key");
        let got: Vec<(String, i64)> = rows
            .into_iter()
            .map(|r| (r[0].as_str().unwrap().to_string(), r[1].as_i64().unwrap()))
            .collect();
        let want: Vec<(String, i64)> = [
            ("format_version", 1), ("last_instant", 0), ("last_t", 0), ("multi_version", 0),
            ("next_bnode", 1), ("next_node", 1), ("next_stmt", 1), ("next_term", 1),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), *v))
        .collect();
        assert_eq!(got, want);
    }
}

host_test! {
    /// Engine metadata counters — Counters advance with allocation.
    fn counters_advance(db) {
        let (s0, n0) = (db.meta("next_stmt"), db.meta("next_node"));
        let rep = db.tx(|tx| {
            let n = tx.new_node()?;
            tx.assert(n, iri("p"), Value::Int(1), Valid::ALWAYS)?;
            tx.assert(n, iri("p"), Value::Int(2), Valid::ALWAYS)?;
            tx.assert(n, iri("p"), Value::Int(3), Valid::ALWAYS)?;
            Ok(())
        });
        assert!(db.meta("next_stmt") >= s0 + 3);
        assert!(db.meta("next_node") > n0);
        assert_eq!(db.meta("last_t") as u64, rep.t.0);
        assert_eq!(db.meta("last_instant"), rep.instant);
    }
}

host_test! {
    /// Engine metadata counters — Ids are not derived from existing rows.
    fn ids_are_not_derived_from_rows(db) {
        db.tx(|tx| tx.assert(iri("a"), iri("p"), iri("b"), Valid::ALWAYS).map(|_| ()));
        assert_ok(db.store().speculate(
            |tx| {
                for i in 0..5 {
                    tx.create(iri("a"), iri("p"), Value::Int(i), Valid::ALWAYS)?;
                }
                Ok(())
            },
            |_| Ok(()),
        ));
        let next = db.meta("next_stmt");
        let max_stored = db.scalar("SELECT max(eid) FROM triple") >> 4;
        assert!(next > max_stored + 1);
        let rep = db.tx(|tx| tx.create(iri("a"), iri("p"), iri("c"), Valid::ALWAYS).map(|_| ()));
        assert_eq!(rep.asserted[0].n() as i64, next);
    }
}

// Newer format versions are refused — File from a newer build.
#[test]
fn newer_format_version_is_refused() {
    let mut db = TestDb::new(HostKind::Rusqlite);
    db.tx(|tx| {
        tx.assert(iri("a"), iri("p"), iri("b"), Valid::ALWAYS)
            .map(|_| ())
    });
    db.close();
    {
        let raw = db.raw();
        raw.execute("UPDATE meta SET value = 2 WHERE key = 'format_version'", [])
            .unwrap();
    }
    let bytes = std::fs::read(&db.path).unwrap();
    let r = Store::open(&RusqliteHost::new(), &db.path, StoreOptions::default());
    assert_err!(
        r,
        Error::FormatVersion {
            found: 2,
            supported: 1
        }
    );
    let r = Store::open(&MinimalHost::new(), &db.path, StoreOptions::default());
    assert_err!(
        r,
        Error::FormatVersion {
            found: 2,
            supported: 1
        }
    );
    assert_eq!(std::fs::read(&db.path).unwrap(), bytes);
}

fn add_v2_table(exec: &mut dyn Executor) -> Result<()> {
    exec.execute_batch("CREATE TABLE extra_v2 (x INTEGER) STRICT")
}

fn failing_step(exec: &mut dyn Executor) -> Result<()> {
    exec.execute_batch("CREATE TABLE extra_fail (x INTEGER) STRICT")?;
    Err(Error::custom("migration failed"))
}

fn contents(raw: &rusqlite::Connection) -> Vec<String> {
    let mut out = Vec::new();
    for t in ["triple", "term", "tx"] {
        let mut st = raw
            .prepare(&format!("SELECT * FROM {t} ORDER BY 1"))
            .unwrap();
        let n = st.column_count();
        let rows = st
            .query_map([], |r| {
                Ok((0..n)
                    .map(|i| format!("{:?}", r.get_ref(i).unwrap()))
                    .collect::<Vec<_>>()
                    .join(","))
            })
            .unwrap();
        out.extend(rows.map(Result::unwrap));
    }
    out
}

// Older format versions are migrated forward atomically — both scenarios.
#[test]
fn older_versions_migrate_forward_atomically() {
    for kind in [HostKind::Rusqlite, HostKind::Minimal] {
        let mut db = TestDb::new(kind);
        db.tx(|tx| {
            tx.assert(
                iri("a"),
                iri("p"),
                lit("a long enough string"),
                Valid::ALWAYS,
            )?;
            Ok(())
        });
        db.close();
        let before = contents(&db.raw());
        let host: Box<dyn Host> = match kind {
            HostKind::Rusqlite => Box::new(RusqliteHost::new()),
            HostKind::Minimal => Box::new(MinimalHost::new()),
        };
        // failed migration rolls back
        let bad = [Migration {
            from: 1,
            apply: failing_step,
        }];
        let r = storage::open_with(&*host, &db.path, &HostOptions::default(), &bad, 2);
        assert!(r.is_err());
        drop(r);
        {
            let raw = db.raw();
            let v: i64 = raw
                .query_row(
                    "SELECT value FROM meta WHERE key = 'format_version'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(v, 1);
            assert!(!names(&raw, "table").contains("extra_fail"));
            assert_eq!(contents(&raw), before);
        }
        // successful migration
        let good = [Migration {
            from: 1,
            apply: add_v2_table,
        }];
        let exec = storage::open_with(&*host, &db.path, &HostOptions::default(), &good, 2).unwrap();
        drop(exec);
        let raw = db.raw();
        let v: i64 = raw
            .query_row(
                "SELECT value FROM meta WHERE key = 'format_version'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(v, 2);
        assert!(names(&raw, "table").contains("extra_v2"));
        assert_eq!(contents(&raw), before);
    }
}

// Foreign files are refused — Unrelated SQLite database / Empty SQLite file.
#[test]
fn foreign_and_empty_files() {
    let dir = tempfile::tempdir().unwrap();
    let foreign = dir.path().join("customers.db");
    {
        let raw = rusqlite::Connection::open(&foreign).unwrap();
        raw.execute_batch("CREATE TABLE customers (id INTEGER PRIMARY KEY, name TEXT)")
            .unwrap();
    }
    for host in [&RusqliteHost::new() as &dyn Host, &MinimalHost::new()] {
        let r = Store::open(host, &foreign, StoreOptions::default());
        assert_err!(r, Error::ForeignFile(p) if p == foreign);
    }
    let raw = rusqlite::Connection::open(&foreign).unwrap();
    assert_eq!(names(&raw, "table"), set(&["customers"]));
    assert!(names(&raw, "index").is_empty() && names(&raw, "trigger").is_empty());

    let empty = dir.path().join("empty.db");
    std::fs::File::create(&empty).unwrap();
    let mut s = Store::open(&RusqliteHost::new(), &empty, StoreOptions::default()).unwrap();
    assert_eq!(
        s.executor()
            .query_i64("SELECT value FROM meta WHERE key = 'format_version'", &[])
            .unwrap(),
        Some(1)
    );
    let no_tables = dir.path().join("notables.db");
    {
        let raw = rusqlite::Connection::open(&no_tables).unwrap();
        raw.execute_batch("CREATE TABLE t(x); DROP TABLE t;")
            .unwrap();
    }
    let mut s = Store::open(&MinimalHost::new(), &no_tables, StoreOptions::default()).unwrap();
    assert_eq!(
        s.executor().query_i64("SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%'", &[]).unwrap(),
        Some(6)
    );
}

fn skewed_load(db: &mut TestDb, txs: usize) {
    for k in 0..txs {
        db.tx(|tx| {
            for i in 0..40 {
                let s = iri(&format!("n{k}_{i}"));
                tx.assert(s.clone(), iri("type"), iri("Big"), Valid::ALWAYS)?;
                if i % 20 == 0 {
                    tx.assert(s.clone(), iri("rare"), Value::Int(i), Valid::ALWAYS)?;
                }
                tx.assert(s, iri("score"), Value::Int(i), Valid::ALWAYS)?;
            }
            Ok(())
        });
    }
}

host_test! {
    /// Planner statistics — Statistics exist without an explicit optimize.
    fn statistics_exist_without_optimize(db) {
        skewed_load(db, 5);
        db.reopen();
        let idx: BTreeSet<String> = db
            .rows("SELECT idx FROM sqlite_stat1 WHERE tbl = 'triple'")
            .into_iter()
            .filter_map(|r| r[0].as_str().map(str::to_string))
            .collect();
        assert!(idx.contains("live_spo") && idx.contains("hist_pos"), "{idx:?}");
    }
}

// Planner statistics — Periodic optimize.
#[test]
fn periodic_optimize() {
    for kind in [HostKind::Rusqlite, HostKind::Minimal] {
        let mut db = TestDb::with_optimize_every(kind, 10);
        if let Some(p) = db.probe() {
            p.clear_log();
        }
        for i in 0..25 {
            db.tx(|tx| {
                tx.assert(iri("a"), iri("p"), Value::Int(i), Valid::ALWAYS)
                    .map(|_| ())
            });
        }
        assert_eq!(db.store().optimize_runs(), 2);
        if let Some(p) = db.probe() {
            assert_eq!(p.count("PRAGMA optimize"), 2);
        }
        // dry runs and speculation do not count
        for _ in 0..10 {
            let r = db.try_tx_opts(
                TxOptions {
                    dry_run: true,
                    ..TxOptions::default()
                },
                |tx| {
                    tx.assert(iri("x"), iri("p"), iri("y"), Valid::ALWAYS)
                        .map(|_| ())
                },
            );
            assert_ok(r);
        }
        assert_eq!(db.store().optimize_runs(), 2);
        // a bulk load optimises right after its commit
        db.tx(|tx| {
            for i in 0..10 {
                tx.create(iri("bulk"), iri("p"), Value::Int(i), Valid::ALWAYS)?;
            }
            Ok(())
        });
        assert_eq!(db.store().optimize_runs(), 3);
    }
}

fn lookups(db: &mut TestDb) -> Vec<Vec<Triple>> {
    let p = db.id(&iri("score"));
    let big = db.id(&iri("Big"));
    let s = db.id(&iri("n1_1"));
    let mut out = Vec::new();
    for spec in [
        ViewSpec::NOW,
        ViewSpec::history(),
        ViewSpec::as_of(TimeRef::Tx(3)),
        ViewSpec::NOW.valid_at(5),
    ] {
        out.push(db.triples(spec, None, Some(p), None));
        out.push(db.triples(spec, None, None, Some(big)));
        out.push(db.triples(spec, Some(s), None, None));
    }
    out
}

host_test! {
    /// Planner statistics — Stale statistics never change results.
    fn stale_statistics_never_change_results(db) {
        skewed_load(db, 3);
        for k in 0..4 {
            db.tx(|tx| {
                let p = tx.encode(iri("score"))?;
                let s = tx.encode(iri(&format!("n{k}_1")))?;
                tx.retract_matching(Some(s), Some(p), None)?;
                tx.assert(s, p, Value::Int(100 + k), Valid::ALWAYS)?;
                Ok(())
            });
        }
        let before = lookups(db);
        assert_ok(db.store().optimize());
        assert_eq!(lookups(db), before);
    }
}

host_test! {
    /// Planner statistics — Full analysis on request.
    fn full_analysis_on_request(db) {
        skewed_load(db, 1);
        db.tx(|tx| {
            let p = tx.encode(iri("score"))?;
            let three = tx.encode(Value::Int(3))?;
            tx.retract_matching(None, Some(p), Some(three))?;
            Ok(())
        });
        assert_ok(db.store().optimize());
        let idx: BTreeSet<String> = db
            .rows("SELECT idx FROM sqlite_stat1 WHERE tbl = 'triple'")
            .into_iter()
            .filter_map(|r| r[0].as_str().map(str::to_string))
            .collect();
        for i in [
            "live_spo", "live_pos", "live_osp", "hist_spo", "hist_pos", "hist_osp", "valid_p",
            "log_add", "log_ret",
        ] {
            assert!(idx.contains(i), "{i} missing from {idx:?}");
        }
    }
}

fn pred_multi(db: &mut TestDb) -> BTreeSet<i64> {
    db.rows("SELECT p FROM pred_multi")
        .into_iter()
        .filter_map(|r| r[0].as_i64())
        .collect()
}

host_test! {
    /// Multi-eid predicates are recorded — all three scenarios, plus rollback,
    /// speculation and dry runs leaving `pred_multi` unchanged.
    fn multi_eid_predicates_are_recorded(db) {
        db.tx(|tx| {
            tx.assert(iri("alice"), iri("name"), lit("Alice"), Valid::ALWAYS)?;
            tx.assert(iri("alice"), iri("name"), lit("Alice"), Valid::from(5))?;
            Ok(())
        });
        assert!(pred_multi(db).is_empty());
        let v0 = db.meta("multi_version");
        db.tx(|tx| {
            tx.assert(iri("alice"), iri("called"), iri("bob"), Valid::ALWAYS)?;
            tx.create(iri("alice"), iri("called"), iri("bob"), Valid::ALWAYS)?;
            tx.assert(iri("alice"), iri("worksAt"), iri("acme"), Valid::between(0, 10))?;
            tx.assert(iri("alice"), iri("worksAt"), iri("acme"), Valid::between(20, 30))?;
            Ok(())
        });
        let (called, works) = (db.id(&iri("called")), db.id(&iri("worksAt")));
        assert_eq!(pred_multi(db), BTreeSet::from([called.raw(), works.raw()]));
        assert_eq!(db.meta("multi_version"), v0 + 2);
        let mut e = None;
        db.tx(|tx| {
            e = Some(tx.assert(iri("alice"), iri("likes"), iri("tea"), Valid::ALWAYS)?.eid());
            Ok(())
        });
        db.tx(|tx| tx.retract(e.unwrap()).map(|_| ()));
        let snap = db.snapshot();
        // rollback, speculation and dry run leave pred_multi unchanged
        let r = db.try_tx(|tx| {
            tx.assert(iri("alice"), iri("likes"), iri("tea"), Valid::ALWAYS)?;
            Err(Error::custom("no"))
        });
        assert!(r.is_err());
        assert_ok(db.store().speculate(
            |tx| tx.assert(iri("alice"), iri("likes"), iri("tea"), Valid::ALWAYS).map(|_| ()),
            |_| Ok(()),
        ));
        let dry = db.try_tx_opts(TxOptions { dry_run: true, ..TxOptions::default() }, |tx| {
            tx.assert(iri("alice"), iri("likes"), iri("tea"), Valid::ALWAYS).map(|_| ())
        });
        assert_ok(dry);
        assert_eq!(db.data_snapshot(), snap.into_iter().filter(|(t, _)| t != "meta").collect::<Vec<_>>());
        assert_eq!(db.meta("multi_version"), v0 + 2);
        db.tx(|tx| tx.assert(iri("alice"), iri("likes"), iri("tea"), Valid::ALWAYS).map(|_| ()));
        let likes = db.id(&iri("likes"));
        assert!(pred_multi(db).contains(&likes.raw()));
        assert_eq!(db.meta("multi_version"), v0 + 3);
    }
}
