//! Spec `never-forget`: the invariant triggers, seen from a raw SQLite connection.
//!
//! The source audit (no `DELETE FROM triple|term|tx`, no `UPDATE term|tx` in engine
//! code) also runs as a CI step (`.github/workflows/ci.yml`).

mod common;
use common::*;

use tm_core::*;

fn seed(db: &mut TestDb) -> (Eid, Eid, ObjectId) {
    let (mut e1, mut e2, mut term) = (None, None, None);
    db.tx(|tx| {
        e1 = Some(
            tx.assert(
                iri("alice"),
                iri("name"),
                lit("a long enough name"),
                Valid::ALWAYS,
            )?
            .eid(),
        );
        e2 = Some(
            tx.assert(iri("bob"), iri("name"), lit("Bob"), Valid::ALWAYS)?
                .eid(),
        );
        term = Some(tx.encode(lit("a long enough name"))?);
        Ok(())
    });
    (e1.unwrap(), e2.unwrap(), term.unwrap())
}

fn err_msg(r: rusqlite::Result<usize>) -> String {
    r.expect_err("must fail").to_string()
}

// @lat: [[tests#Storage Invariants#Triggers Block Deletion]]
host_test! {
    fn raw_delete_of_one_statement(db) {
        let (e1, _, _) = seed(db);
        let raw = db.raw();
        let before = db.row(e1);
        let m = err_msg(raw.execute("DELETE FROM triple WHERE eid = ?1", [e1.oid().raw()]));
        assert!(m.contains("tiramemsu: triples are never deleted"), "{m}");
        assert_eq!(db.row(e1), before);
        let n = db.count("triple");
        assert!(raw.execute("DELETE FROM triple", []).is_err());
        assert_eq!(db.count("triple"), n);
    }
}

host_test! {
    /// Rows are never replaced.
    fn rows_are_never_replaced(db) {
        let (e1, _, term) = seed(db);
        let raw = db.raw();
        let before = db.snapshot();
        let m = err_msg(raw.execute(
            "INSERT OR REPLACE INTO triple(eid, s, p, o, t_add) VALUES (?1, 1, 2, 3, 1)",
            [e1.oid().raw()],
        ));
        assert!(m.contains("tiramemsu: eids are never reused"), "{m}");
        assert!(raw
            .execute("REPLACE INTO term(id, tag, lex) VALUES (?1, 10, 'x')", [term.unsigned_payload() as i64])
            .is_err());
        assert!(raw.execute("REPLACE INTO tx(t, instant) VALUES (1, 5)", []).is_err());
        assert_eq!(db.snapshot(), before);
        // new rows are unaffected
        db.tx(|tx| tx.create(iri("c"), iri("p"), iri("d"), Valid::ALWAYS).map(|_| ()));
    }
}

host_test! {
    /// A statement is retracted at most once and its content never changes.
    fn retracted_at_most_once(db) {
        let (e1, e2, _) = seed(db);
        let raw = db.raw();
        let id1 = e1.oid().raw();
        assert_eq!(raw.execute("UPDATE triple SET t_ret = 9, ret_kind = 0 WHERE eid = ?1", [id1]).unwrap(), 1);
        let m = err_msg(raw.execute("UPDATE triple SET t_ret = 12 WHERE eid = ?1", [id1]));
        assert!(m.contains("tiramemsu: only a single retraction is allowed"), "{m}");
        assert_eq!(db.row(e1).t_ret, Some(TxId(9)));
        assert!(raw.execute("UPDATE triple SET t_ret = NULL WHERE eid = ?1", [id1]).is_err());
        let id2 = e2.oid().raw();
        let before = db.row(e2);
        for col in ["o = 77", "s = 77", "p = 77", "eid = 77", "t_add = 77", "v_from = 77", "v_to = 77"] {
            for sql in [
                format!("UPDATE triple SET {col} WHERE eid = ?1"),
                format!("UPDATE triple SET {col}, t_ret = 5, ret_kind = 0 WHERE eid = ?1"),
            ] {
                assert!(raw.execute(&sql, [id2]).is_err(), "{sql}");
            }
            assert!(raw.execute(&format!("UPDATE triple SET {col} WHERE eid = ?1"), [id1]).is_err());
        }
        assert!(raw.execute("UPDATE triple SET ret_kind = 1 WHERE eid = ?1", [id2]).is_err());
        assert_eq!(db.row(e2), before);
    }
}

host_test! {
    /// Terms and transactions are never deleted or changed.
    fn terms_and_transactions(db) {
        let (_, _, term) = seed(db);
        let raw = db.raw();
        let id = term.unsigned_payload() as i64;
        let m = err_msg(raw.execute("DELETE FROM term WHERE id = ?1", [id]));
        assert!(m.contains("tiramemsu: terms are never deleted"), "{m}");
        let m = err_msg(raw.execute("UPDATE term SET lex = 'changed' WHERE id = ?1", [id]));
        assert!(m.contains("tiramemsu: terms are immutable"), "{m}");
        let m = err_msg(raw.execute("DELETE FROM tx WHERE t = 1", []));
        assert!(m.contains("tiramemsu: transactions are never deleted"), "{m}");
        let m = err_msg(raw.execute("UPDATE tx SET instant = 1 WHERE t = 1", []));
        assert!(m.contains("tiramemsu: transactions are immutable"), "{m}");
    }
}

host_test! {
    /// The engine never issues deletions or rewrites — savepoint rollback works.
    fn savepoint_rollback_is_not_blocked(db) {
        seed(db);
        let before = db.data_snapshot();
        let seen = assert_ok(db.store().speculate(
            |tx| {
                tx.assert(iri("x"), iri("y"), lit("a speculative long string"), Valid::ALWAYS)?;
                tx.create(iri("x"), iri("y"), iri("z"), Valid::ALWAYS)?;
                Ok(())
            },
            |e| e.query_i64("SELECT count(*) FROM triple", &[]),
        ));
        assert_eq!(seen, Some(4));
        assert_eq!(db.data_snapshot(), before);
    }
}

fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs" || x == "sql") {
            out.push(p);
        }
    }
}

// The engine never issues deletions or rewrites — Source audit.
#[test]
fn source_audit() {
    let crates = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/.."));
    let mut files = Vec::new();
    for c in std::fs::read_dir(crates).unwrap() {
        let src = c.unwrap().path().join("src");
        if src.is_dir() {
            walk(&src, &mut files);
        }
    }
    assert!(files.len() > 10);
    for f in files {
        let text = std::fs::read_to_string(&f).unwrap().to_ascii_uppercase();
        for bad in [
            "DELETE FROM TRIPLE",
            "DELETE FROM TERM",
            "DELETE FROM TX",
            "UPDATE TERM",
            "UPDATE TX",
        ] {
            assert!(!text.contains(bad), "{} contains {bad}", f.display());
        }
    }
}

host_test! {
    /// Erasure never deletes rows.
    fn retracted_values_stay_in_history(db) {
        let (e1, _, term) = seed(db);
        let t = db.last_t();
        db.tx(|tx| tx.retract(e1).map(|_| ()));
        assert_eq!(db.scalar(&format!("SELECT count(*) FROM term WHERE id = {}", term.unsigned_payload())), 1);
        assert!(db.history_all().iter().any(|r| r.eid == e1));
        assert!(db.as_of(t).iter().any(|r| r.eid == e1));
        assert!(db.rows("SELECT name FROM sqlite_schema WHERE name = 'seal_key'").is_empty());
    }
}

host_test! {
    /// Volatile and metadata tables are outside the invariant.
    fn volatile_and_meta_are_outside(db) {
        db.tx(|tx| tx.set_volatile(iri("a"), iri("k"), Value::Int(1)));
        db.tx(|tx| tx.set_volatile(iri("a"), iri("k"), Value::Int(2)));
        let raw = db.raw();
        raw.execute("DELETE FROM volatile", []).unwrap();
        raw.execute("UPDATE meta SET value = value + 1 WHERE key = 'next_stmt'", []).unwrap();
    }
}
