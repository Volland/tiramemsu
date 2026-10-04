//! Retained repair probes: success means each documented gap is prevented.
use tm_core::{
    read, AssertOpts, Eid, Patch, Store, StoreOptions, TxOptions, Valid, Value, ViewSpec,
};
use tm_rusqlite::RusqliteHost;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("conformance.db");
    let mut store = Store::open(&RusqliteHost::new(), &path, StoreOptions::default())?;
    let iri = |name: &str| Value::iri(format!("urn:paper:{name}"));
    let mut root = None;
    store.transact(TxOptions::default(), |tx| {
        root = Some(
            tx.assert(iri("alice"), iri("worksAt"), iri("acme"), Valid::ALWAYS)?
                .eid(),
        );
        Ok(())
    })?;
    let root = root.unwrap();
    store.transact(TxOptions::default(), |tx| tx.retract(root).map(|_| ()))?;
    let result = store.transact(TxOptions::default(), |tx| {
        tx.assert(root, iri("confidence"), Value::Int(80), Valid::ALWAYS)?;
        Ok(())
    });
    assert!(matches!(result, Err(tm_core::Error::NotLive(e)) if e == root));
    println!("retracted-reference: PREVENTED");

    let missing = Eid::new(999_999);
    let result = store.transact(TxOptions::default(), |tx| {
        tx.assert(missing, iri("source"), Value::str("missing"), Valid::ALWAYS)?;
        Ok(())
    });
    assert!(matches!(result, Err(tm_core::Error::NotLive(e)) if e == missing));
    println!("unallocated-reference: PREVENTED");

    let mut fact = None;
    let mut member_layer = None;
    store.transact(TxOptions::default(), |tx| {
        let f = tx
            .assert(iri("bob"), iri("worksAt"), iri("acme"), Valid::ALWAYS)?
            .eid();
        let (m, _) = tx.add_to_graph(f, iri("session"), AssertOpts::default())?;
        member_layer = Some(
            tx.assert(m, iri("addedBy"), iri("author"), Valid::ALWAYS)?
                .eid(),
        );
        fact = Some(f);
        Ok(())
    })?;
    let report = store.transact(TxOptions::default(), |tx| {
        tx.supersede(fact.unwrap(), Patch::object(iri("globex")))?;
        Ok(())
    })?;
    assert!(!report
        .superseded
        .iter()
        .any(|(old, _)| *old == member_layer.unwrap()));
    let rows = store.read(|ex| read::triples(ex, &ViewSpec::NOW, None, None, None))?;
    let all = store.read(|ex| read::triples(ex, &ViewSpec::history(), None, None, None))?;
    for row in &rows {
        for id in [row.s, row.o] {
            if Eid::from_oid(id).is_some() {
                assert!(all.iter().any(|r| r.eid.oid() == id));
            }
        }
    }
    println!("dropped-membership-layer: PREVENTED");

    let conn = rusqlite::Connection::open(&path)?;
    conn.pragma_update(None, "recursive_triggers", "ON")?;
    let first: i64 = conn.query_row("SELECT min(t) FROM tx", [], |r| r.get(0))?;
    let live_eid: i64 = conn.query_row(
        "SELECT eid FROM triple WHERE t_ret IS NULL ORDER BY eid LIMIT 1",
        [],
        |r| r.get(0),
    )?;
    // A separate fresh statement at an old committed time changes the old view.
    let fresh = Eid::new(8_000_000).oid().raw();
    let count = |c: &rusqlite::Connection| -> rusqlite::Result<i64> {
        c.query_row(
            "SELECT count(*) FROM triple WHERE t_add<=?1 AND (t_ret IS NULL OR t_ret>?1)",
            [first],
            |r| r.get(0),
        )
    };
    let before = count(&conn)?;
    assert!(conn
        .execute(
            "INSERT INTO triple(eid,s,p,o,t_add) SELECT ?1,s,p,o,?2 FROM triple WHERE eid=?3",
            rusqlite::params![fresh, first, live_eid]
        )
        .is_err());
    assert_eq!(count(&conn)?, before);
    assert!(conn
        .execute(
            "UPDATE triple SET t_ret=?1 WHERE eid=?2",
            rusqlite::params![first, live_eid]
        )
        .is_err());
    assert_eq!(count(&conn)?, before);
    println!("backdated-direct-sql: PREVENTED (insert and retract; triggers enabled)");
    Ok(())
}
