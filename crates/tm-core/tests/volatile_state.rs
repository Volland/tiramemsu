//! Spec `volatile-state`: the volatile side table.

mod common;
use common::*;

use tm_core::read;
use tm_core::*;

fn vol(db: &mut TestDb) -> Vec<Vec<SqlValue>> {
    db.rows("SELECT s, key, value, updated_at FROM volatile ORDER BY s, key")
}

fn values(db: &mut TestDb, spec: ViewSpec, s: &str, key: &str) -> Vec<ObjectId> {
    let (s, k) = (db.id(&iri(s)), db.id(&iri(key)));
    db.read(|e| read::values(e, &spec, s, k))
}

host_test! {
    /// Volatile values are upserted per subject and key — overwrite, failure, clear.
    fn upserts_and_clears(db) {
        let v1 = Value::literal("2026-09-01T10:00:00Z", Some(vocab::XSD_DATETIME), None);
        let v2 = Value::literal("2026-09-02T10:00:00Z", Some(vocab::XSD_DATETIME), None);
        db.tx(|_| Ok(()));
        db.tx(|_| Ok(()));
        db.tx(|tx| tx.set_volatile(iri("alice"), iri("lastSeen"), v1.clone()));
        db.clock.set(T0 + 50);
        let rep = db.tx(|tx| tx.set_volatile(iri("alice"), iri("lastSeen"), v2.clone()));
        assert_eq!(rep.t, TxId(4));
        let rows = vol(db);
        assert_eq!(rows.len(), 1);
        let v2id = db.id(&v2);
        assert_eq!(rows[0][2], SqlValue::Integer(v2id.raw()));
        assert_eq!(rows[0][3], SqlValue::Integer(rep.instant));
        // failed transaction discards the write
        let before = vol(db);
        let r = db.try_tx(|tx| {
            tx.set_volatile(iri("alice"), iri("lastSeen"), Value::Int(1))?;
            tx.set_volatile(iri("bob"), iri("lastSeen"), Value::Int(1))?;
            Err(Error::custom("fail"))
        });
        assert!(r.is_err());
        assert_eq!(vol(db), before);
        // clear twice
        db.tx(|tx| tx.set_volatile(iri("alice"), iri("score"), Value::Int(3)));
        db.tx(|tx| tx.clear_volatile(iri("alice"), iri("score")));
        db.tx(|tx| tx.clear_volatile(iri("alice"), iri("score")));
        assert_eq!(vol(db).len(), 1);
        // kinds
        let r = db.try_tx(|tx| tx.set_volatile(iri("alice"), lit("key"), Value::Int(1)));
        assert_err!(r, Error::InvalidTerm { position: Position::Key, .. });
        let r = db.try_tx(|tx| tx.set_volatile(Value::Int(4), iri("k"), Value::Int(1)));
        assert_err!(r, Error::InvalidTerm { position: Position::Subject, .. });
    }
}

host_test! {
    /// Volatile values are not statements.
    fn not_statements(db) {
        let t0 = db.last_t();
        let rep = db.tx(|tx| tx.set_volatile(iri("alice"), iri("lastSeen"), Value::Int(9)));
        assert!(rep.asserted.is_empty() && rep.retracted.is_empty());
        assert_eq!(db.count("tx") as u64, t0 + 1);
        let (a, k) = (db.id(&iri("alice")), db.id(&iri("lastSeen")));
        assert!(db.now(Some(a), Some(k), None).is_empty());
        assert!(db.history_all().is_empty());
        assert!(db.events_since(t0).is_empty());
    }
}

host_test! {
    /// Volatile values are visible only in the now view.
    fn visible_only_now(db) {
        db.tx(|tx| {
            tx.set_volatile(iri("alice"), iri("lastSeen"), Value::Int(9))?;
            tx.assert(iri("alice"), iri("status"), lit("away"), Valid::ALWAYS)?;
            tx.set_volatile(iri("alice"), iri("status"), lit("busy"))?;
            Ok(())
        });
        let nine = db.id(&Value::Int(9));
        assert_eq!(values(db, ViewSpec::NOW, "alice", "lastSeen"), vec![nine]);
        assert_eq!(values(db, ViewSpec::NOW.valid_at(5), "alice", "lastSeen"), vec![nine]);
        let t = db.last_t();
        assert!(values(db, ViewSpec::as_of(TimeRef::Tx(t)), "alice", "lastSeen").is_empty());
        assert!(values(db, ViewSpec::history(), "alice", "lastSeen").is_empty());
        let away = db.id(&lit("away"));
        assert_eq!(values(db, ViewSpec::NOW, "alice", "status"), vec![away]);
        // speculation sees its own volatile writes; committed value unchanged
        let (a, k) = (db.id(&iri("alice")), db.id(&iri("lastSeen")));
        let seen = assert_ok(db.store().speculate(
            |tx| tx.set_volatile(iri("alice"), iri("lastSeen"), Value::Int(5)),
            |e| read::values(e, &ViewSpec::NOW, a, k),
        ));
        assert_eq!(seen, vec![ObjectId::from_signed(Tag::Int, 5)]);
        assert_eq!(values(db, ViewSpec::NOW, "alice", "lastSeen"), vec![nine]);
    }
}
