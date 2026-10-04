//! Regression coverage for the paper's four engine conformance gaps.
mod common;
use common::*;
use tm_core::*;

// @lat: [[tests#Conformance Repairs#Statement Endpoints Must Be Live]]
host_test! {
    fn statement_endpoints_must_be_live(db) {
        let root = db
            .tx(|tx| {
                tx.assert(iri("a"), iri("p"), iri("b"), Valid::ALWAYS)
                    .map(|_| ())
            })
            .asserted[0];
        db.tx(|tx| tx.retract(root).map(|_| ()));
        let before = db.snapshot();
        for e in [root, Eid::new(999999)] {
            assert!(db
                .try_tx(|tx| tx
                    .assert(e, iri("note"), lit("x"), Valid::ALWAYS)
                    .map(|_| ()))
                .is_err());
            assert!(db
                .try_tx(|tx| tx
                    .create(iri("a"), iri("about"), e, Valid::ALWAYS)
                    .map(|_| ()))
                .is_err());
            assert_eq!(db.snapshot(), before);
        }
    }
}

// @lat: [[tests#Conformance Repairs#Dropped Membership Layers Are Not Replayed]]
host_test! {
    fn dropped_membership_layers_are_not_replayed(db) {
        let mut ids = None;
        db.tx(|tx| {
            let root = tx
                .assert(iri("a"), iri("p"), iri("b"), Valid::ALWAYS)?
                .eid();
            let (m, _) = tx.add_to_graph(root, iri("session"), AssertOpts::default())?;
            let layer = tx
                .assert(m, iri("author"), iri("writer"), Valid::ALWAYS)?
                .eid();
            let deep = tx
                .assert(layer, iri("confidence"), Value::Int(80), Valid::ALWAYS)?
                .eid();
            ids = Some((root, m, layer, deep));
            Ok(())
        });
        let (root, m, layer, deep) = ids.unwrap();
        let report = db.tx(|tx| tx.supersede(root, Patch::object(iri("c"))).map(|_| ()));
        for e in [m, layer, deep] {
            assert!(!db.is_live(e));
            assert!(!report.superseded.iter().any(|(old, _)| *old == e));
        }
        let allocated: std::collections::HashSet<_> = db
            .rows("SELECT eid FROM triple")
            .into_iter()
            .map(|r| r[0].as_i64().unwrap())
            .collect();
        for row in db.now(None, None, None) {
            for id in [row.s, row.o] {
                if Eid::from_oid(id).is_some() {
                    assert!(allocated.contains(&id.raw()));
                }
            }
        }
    }
}

// @lat: [[tests#Conformance Repairs#Direct SQL Cannot Backdate Statements]]
host_test! {
    fn direct_sql_cannot_backdate_statements(db) {
        let root = db
            .tx(|tx| {
                tx.assert(iri("a"), iri("p"), iri("b"), Valid::ALWAYS)
                    .map(|_| ())
            })
            .asserted[0];
        let before = db.snapshot();
        let raw = db.raw();
        assert!(raw
            .execute(
                "INSERT INTO triple(eid,s,p,o,t_add) SELECT ?1,s,p,o,t_add FROM triple WHERE eid=?2",
                rusqlite::params![Eid::new(999999).oid().raw(), root.oid().raw()]
            )
            .is_err());
        assert!(raw
            .execute(
                "UPDATE triple SET t_ret=t_add WHERE eid=?1",
                [root.oid().raw()]
            )
            .is_err());
        assert!(raw
            .execute(
                "UPDATE triple SET t_ret=999999 WHERE eid=?1",
                [root.oid().raw()]
            )
            .is_err());
        assert_eq!(db.snapshot(), before);
        // Even while a fresh date is open, past dates remain protected.
        raw.execute_batch("BEGIN IMMEDIATE; INSERT INTO tx(t,instant) VALUES (2,2000000);")
            .unwrap();
        assert!(raw
            .execute("UPDATE triple SET t_ret=1 WHERE eid=?1", [root.oid().raw()])
            .is_err());
        let fresh = Eid::new(999999).oid().raw();
        assert!(raw
            .execute(
                "INSERT INTO triple(eid,s,p,o,t_add) SELECT ?1,s,p,o,1 FROM triple WHERE eid=?2",
                rusqlite::params![fresh, root.oid().raw()]
            )
            .is_err());
        raw.execute(
            "INSERT INTO triple(eid,s,p,o,t_add) SELECT ?1,s,p,o,2 FROM triple WHERE eid=?2",
            rusqlite::params![fresh, root.oid().raw()],
        )
        .unwrap();
        raw.execute("UPDATE triple SET t_ret=2 WHERE eid=?1", [fresh])
            .unwrap();
        raw.execute_batch("ROLLBACK").unwrap();
        assert_eq!(db.snapshot(), before);
        db.tx(|tx| {
            let e = tx
                .assert(iri("transient"), iri("p"), iri("b"), Valid::ALWAYS)?
                .eid();
            tx.retract(e)?;
            Ok(())
        });
    }
}

// @lat: [[tests#Conformance Repairs#Correction Remaps Patched References]]
host_test! {
    fn correction_remaps_patched_references(db) {
        let mut ids = None;
        db.tx(|tx| {
            let root = tx
                .assert(iri("a"), iri("p"), iri("b"), Valid::ALWAYS)?
                .eid();
            let layer = tx.assert(root, iri("note"), lit("x"), Valid::ALWAYS)?.eid();
            ids = Some((root, layer));
            Ok(())
        });
        let (root, layer) = ids.unwrap();
        let report = db.tx(|tx| {
            tx.supersede(root, Patch::object(Value::Stmt(layer)))
                .map(|_| ())
        });
        let sigma: std::collections::HashMap<_, _> = report.superseded.iter().copied().collect();
        assert_eq!(db.row(sigma[&root]).o, sigma[&layer].oid());
        assert_eq!(db.row(sigma[&layer]).s, sigma[&root].oid());
        // A correction can create a cycle, but it remains closed and allocated.
        assert!(db.is_live(sigma[&root]) && db.is_live(sigma[&layer]));
    }
}

// @lat: [[tests#Conformance Repairs#Invalid Correction Endpoints Roll Back]]
host_test! {
    fn invalid_correction_endpoints_roll_back(db) {
        let mut ids = None;
        db.tx(|tx| {
            let root = tx
                .assert(iri("a"), iri("p"), iri("b"), Valid::ALWAYS)?
                .eid();
            let (m, _) = tx.add_to_graph(root, iri("session"), AssertOpts::default())?;
            ids = Some((root, m));
            Ok(())
        });
        let (root, m) = ids.unwrap();
        let before = db.snapshot();
        for target in [m, root, Eid::new(999999)] {
            assert!(db
                .try_tx(|tx| tx
                    .supersede(root, Patch::object(Value::Stmt(target)))
                    .map(|_| ()))
                .is_err());
            assert_eq!(db.snapshot(), before);
        }
        // A membership selected for dropping cannot itself be the correction root.
        assert!(db
            .try_tx(|tx| tx
                .supersede(m, Patch::object(iri("other-session")))
                .map(|_| ()))
            .is_err());
        assert_eq!(db.snapshot(), before);
    }
}

// @lat: [[tests#Conformance Repairs#Format Three Migrates Date Guards]]
host_test! {
    fn format_two_migrates_date_guards(db) {
        let root = db
            .tx(|tx| {
                tx.assert(iri("a"), iri("p"), iri("b"), Valid::ALWAYS)
                    .map(|_| ())
            })
            .asserted[0];
        let original = db.row(root);
        db.close();
        {
            let raw = db.raw();
            raw.execute_batch("DROP TRIGGER triple_add_date; DROP TRIGGER triple_ret_date; UPDATE meta SET value=3 WHERE key='format_version';").unwrap();
        }
        db.reopen();
        assert_eq!(db.row(root), original);
        assert_eq!(db.meta("format_version"), storage::FORMAT_VERSION);
        let raw = db.raw();
        assert!(raw
            .execute(
                "UPDATE triple SET t_ret=t_add WHERE eid=?1",
                [root.oid().raw()]
            )
            .is_err());
        assert!(raw
            .execute(
                "INSERT INTO triple(eid,s,p,o,t_add) SELECT ?1,s,p,o,t_add FROM triple WHERE eid=?2",
                rusqlite::params![Eid::new(999999).oid().raw(), root.oid().raw()]
            )
            .is_err());
    }
}

// @lat: [[tests#Conformance Repairs#Cardinality Cannot Retract A New Endpoint]]
host_test! {
    fn cardinality_cannot_retract_a_new_endpoint(db) {
        let mut ids = None;
        db.tx(|tx| {
            tx.assert(iri("choice"), sys("cardinality"), sys("one"), Valid::ALWAYS)?;
            let root = tx
                .assert(iri("a"), iri("choice"), iri("b"), Valid::ALWAYS)?
                .eid();
            let layer = tx.assert(root, iri("note"), lit("x"), Valid::ALWAYS)?.eid();
            ids = Some((root, layer));
            Ok(())
        });
        let (root, layer) = ids.unwrap();
        let before = db.snapshot();
        let result = db.try_tx(|tx| {
            tx.assert(iri("a"), iri("choice"), layer, Valid::ALWAYS)
                .map(|_| ())
        });
        assert_err!(result, Error::NotLive(e) if e==layer);
        assert_eq!(db.snapshot(), before);
        assert!(db.is_live(root) && db.is_live(layer));
    }
}
