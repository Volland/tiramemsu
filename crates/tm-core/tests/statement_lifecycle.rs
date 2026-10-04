//! Spec `statement-lifecycle`: assert, create, retract, retract_matching, confirm,
//! new nodes, valid time, self-reference and the reserved namespace.

mod common;
use common::*;

use tm_core::*;

fn assert1(db: &mut TestDb, s: &str, p: &str, o: Value, valid: Valid) -> Asserted {
    let mut r = None;
    db.tx(|tx| {
        r = Some(tx.assert(iri(s), iri(p), o, valid)?);
        Ok(())
    });
    r.unwrap()
}

fn works(db: &mut TestDb, valid: Valid) -> Asserted {
    assert1(db, "alice", "worksAt", iri("acme"), valid)
}

// @lat: [[tests#Operations#Assert Is Idempotent]]
host_test! {
    fn same_fact_twice(db) {
        let first = works(db, Valid::ALWAYS);
        assert!(first.is_new());
        let rows = db.count("triple");
        let second = works(db, Valid::ALWAYS);
        assert_eq!(second, Asserted::Existing(first.eid()));
        assert_eq!(db.count("triple"), rows);
    }
}

host_test! {
    /// Assert is idempotent over overlapping valid time — boundary scenarios.
    fn overlap_boundaries(db) {
        let y = |s: &str| day(s);
        let e = works(db, Valid::between(y("2020-01-01"), y("2022-01-01"))).eid();
        let r = works(db, Valid::between(y("2021-01-01"), y("2023-01-01")));
        assert_eq!(r, Asserted::Existing(e));
        let row = db.row(e);
        assert_eq!((row.v_from, row.v_to), (Some(y("2020-01-01")), Some(y("2022-01-01"))));
        // touching intervals do not overlap
        let t = works(db, Valid::between(y("2022-01-01"), y("2024-01-01")));
        assert!(t.is_new());
        assert!(db.is_live(e) && db.is_live(t.eid()));
    }
}

host_test! {
    /// Unbounded interval overlaps everything; half-open ends; retracted match ignored.
    fn unbounded_half_open_and_retracted(db) {
        let e = assert1(db, "bob", "worksAt", iri("acme"), Valid::ALWAYS).eid();
        let r = assert1(db, "bob", "worksAt", iri("acme"), Valid::between(day("1990-01-01"), day("1991-01-01")));
        assert_eq!(r, Asserted::Existing(e));
        let a = assert1(db, "carol", "worksAt", iri("acme"), Valid::from(day("2024-01-01")));
        let b = assert1(db, "carol", "worksAt", iri("acme"), Valid::until(day("2024-01-01")));
        assert!(a.is_new() && b.is_new());
        db.tx(|tx| tx.retract(e).map(|_| ()));
        let again = assert1(db, "bob", "worksAt", iri("acme"), Valid::ALWAYS);
        assert!(again.is_new());
        assert_ne!(again.eid(), e);
    }
}

// @lat: [[tests#Operations#Non-Overlapping Episodes Coexist]]
host_test! {
    fn non_overlapping_episodes_coexist(db) {
        let a = works(db, Valid::between(day("2020-01-01"), day("2022-06-01")));
        let b = works(db, Valid::from(day("2024-01-01")));
        assert!(a.is_new() && b.is_new());
        assert_ne!(a.eid(), b.eid());
        let s = db.id(&iri("alice"));
        let live = db.now(Some(s), None, None);
        assert_eq!(live.len(), 2);
    }
}

// @lat: [[tests#Operations#Create Makes Parallel Edges]]
host_test! {
    fn create_makes_parallel_edges(db) {
        let mut ids = Vec::new();
        db.tx(|tx| {
            ids.push(tx.create(iri("a"), iri("called"), iri("b"), Valid::ALWAYS)?);
            ids.push(tx.create(iri("a"), iri("called"), iri("b"), Valid::ALWAYS)?);
            Ok(())
        });
        assert_ne!(ids[0], ids[1]);
        assert!(db.is_live(ids[0]) && db.is_live(ids[1]));
        // create after assert
        let e1 = assert1(db, "c", "called", iri("d"), Valid::ALWAYS).eid();
        let mut e2 = None;
        db.tx(|tx| {
            e2 = Some(tx.create(iri("c"), iri("called"), iri("d"), Valid::ALWAYS)?);
            Ok(())
        });
        assert_ne!(e1, e2.unwrap());
        assert!(db.is_live(e1) && db.is_live(e2.unwrap()));
    }
}

host_test! {
    /// Statement content and positions.
    fn statement_positions(db) {
        let before = db.snapshot();
        let r = db.try_tx(|tx| tx.assert(Value::Int(5), iri("p"), iri("o"), Valid::ALWAYS).map(|_| ()));
        assert_err!(r, Error::InvalidTerm { position: Position::Subject, .. });
        assert_eq!(db.snapshot(), before);
        let r = db.try_tx(|tx| {
            let n = tx.new_node()?;
            tx.assert(iri("s"), n, iri("o"), Valid::ALWAYS).map(|_| ())
        });
        assert_err!(r, Error::InvalidTerm { position: Position::Predicate, .. });
        let r = db.try_tx(|tx| {
            let unknown = ObjectId::from_unsigned(Tag::Iri, 999_999);
            tx.assert(iri("s"), unknown, iri("o"), Valid::ALWAYS).map(|_| ())
        });
        assert_err!(r, Error::InvalidTerm { position: Position::Predicate, .. });
        // statement and transaction subjects
        let e1 = works(db, Valid::ALWAYS).eid();
        db.tx(|_| Ok(()));
        let rep = db.tx(|tx| {
            tx.assert(e1, iri("confidence"), Value::Double(0.8), Valid::ALWAYS)?;
            tx.assert(TxId(3), iri("note"), lit("ok"), Valid::ALWAYS)?;
            Ok(())
        });
        assert_eq!(rep.asserted.len(), 2);
    }
}

host_test! {
    /// Ordinary statement endpoints must be live, including within a transaction.
    fn references_checked_for_liveness(db) {
        let e1 = works(db, Valid::ALWAYS).eid();
        db.tx(|tx| tx.retract(e1).map(|_| ()));
        let before = db.snapshot();
        let result = db.try_tx(|tx| tx.assert(e1, iri("note"), lit("was wrong"), Valid::ALWAYS).map(|_| ()));
        assert_err!(result, Error::NotLive(e) if e == e1);
        assert_eq!(db.snapshot(), before);
        db.tx(|tx| {
            let root = tx.create(iri("b1"), iri("about"), iri("b2"), Valid::ALWAYS)?;
            tx.create(root, iri("note"), lit("same transaction"), Valid::ALWAYS)?;
            Ok(())
        });
    }
}

host_test! {
    /// Valid time is a half-open interval.
    fn valid_time_intervals(db) {
        let before = db.snapshot();
        let d = day("2025-01-01");
        let r = db.try_tx(|tx| tx.assert(iri("a"), iri("p"), iri("b"), Valid::between(d, d)).map(|_| ()));
        assert_err!(r, Error::InvalidInterval { .. });
        assert_eq!(db.snapshot(), before);
        let r = db.try_tx(|tx| {
            tx.create(iri("a"), iri("p"), iri("b"), Valid::between(day("2026-01-01"), d)).map(|_| ())
        });
        assert_err!(r, Error::InvalidInterval { .. });
        let a = assert1(db, "a", "p", iri("b"), Valid::from(d)).eid();
        let b = assert1(db, "a", "p", iri("c"), Valid::until(d)).eid();
        let (ra, rb) = (db.row(a), db.row(b));
        assert_eq!((ra.v_from, ra.v_to), (Some(d), None));
        assert_eq!((rb.v_from, rb.v_to), (None, Some(d)));
        // end instant is excluded
        let e = assert1(db, "x", "p", iri("y"), Valid::between(d, day("2026-03-01"))).eid();
        let valid = |db: &mut TestDb, ms: i64| {
            db.triples(ViewSpec::NOW.valid_at(ms), None, None, None).iter().any(|t| t.eid == e)
        };
        assert!(valid(db, d));
        assert!(valid(db, day("2026-03-01") - 1));
        assert!(!valid(db, day("2026-03-01")));
    }
}

host_test! {
    /// Assert can confirm an existing match; confirm records corroboration.
    fn confirmations(db) {
        let e1 = works(db, Valid::ALWAYS).eid();
        for _ in 0..7 {
            db.tx(|_| Ok(()));
        }
        let mut r = None;
        let rep = db.tx(|tx| {
            r = Some(tx.assert_with(iri("alice"), iri("worksAt"), iri("acme"),
                AssertOpts { valid: Valid::ALWAYS, on_existing: OnExisting::Confirm })?);
            Ok(())
        });
        assert_eq!(rep.t, TxId(9));
        assert_eq!(r.unwrap(), Asserted::Existing(e1));
        let cb = db.id(&sys("confirmedBy"));
        let conf = db.now(Some(e1.oid()), Some(cb), None);
        assert_eq!(conf.len(), 1);
        assert_eq!(conf[0].o, TxId(9).oid());
        // confirm policy on a new statement writes no confirmation
        let mut n = None;
        db.tx(|tx| {
            n = Some(tx.assert_with(iri("dave"), iri("worksAt"), iri("acme"),
                AssertOpts { valid: Valid::ALWAYS, on_existing: OnExisting::Confirm })?);
            Ok(())
        });
        assert!(n.unwrap().is_new());
        assert!(db.now(Some(n.unwrap().eid().oid()), Some(cb), None).is_empty());
        // confirmation by two transactions; twice in one transaction is the same eid
        let row_before = db.row(e1);
        let (mut c1, mut c2) = (None, None);
        let t_a = db.tx(|tx| {
            c1 = Some(tx.confirm(e1)?);
            assert_eq!(tx.confirm(e1)?, c1.unwrap());
            Ok(())
        });
        let t_b = db.tx(|tx| {
            c2 = Some(tx.confirm(e1)?);
            Ok(())
        });
        assert_ne!(c1, c2);
        let objs: Vec<ObjectId> = db.now(Some(e1.oid()), Some(cb), None).iter().map(|t| t.o).collect();
        assert!(objs.contains(&t_a.t.oid()) && objs.contains(&t_b.t.oid()));
        assert_eq!(db.row(e1), row_before);
        // confirm a retracted statement
        db.tx(|tx| tx.retract(e1).map(|_| ()));
        let before = db.snapshot();
        let r = db.try_tx(|tx| tx.confirm(e1).map(|_| ()));
        assert_err!(r, Error::NotLive(e) if e == e1);
        assert_eq!(db.snapshot(), before);
    }
}

host_test! {
    /// New nodes.
    fn new_nodes(db) {
        let rows = db.count("triple");
        let (mut a, mut b) = (None, None);
        db.tx(|tx| {
            a = Some(tx.new_node()?);
            b = Some(tx.new_node()?);
            Ok(())
        });
        assert_ne!(a, b);
        assert_eq!(a.unwrap().tag().unwrap(), Tag::Node);
        assert_eq!(db.count("triple"), rows);
        let n = a.unwrap();
        let mut name = None;
        let mut age = None;
        db.tx(|tx| {
            name = Some(tx.assert(n, iri("name"), lit("x"), Valid::ALWAYS)?.eid());
            age = Some(tx.assert(n, iri("age"), Value::Int(3), Valid::ALWAYS)?.eid());
            Ok(())
        });
        db.tx(|tx| tx.retract(name.unwrap()).map(|_| ()));
        assert!(db.is_live(age.unwrap()));
    }
}

host_test! {
    /// No direct self-reference.
    fn self_reference(db) {
        db.tx(|tx| tx.create(iri("a"), iri("p"), iri("b"), Valid::ALWAYS).map(|_| ()));
        let before = db.snapshot();
        let next = Eid::new(db.meta("next_stmt") as u64);
        let r = db.try_tx(|tx| tx.assert(iri("a"), iri("about"), next, Valid::ALWAYS).map(|_| ()));
        assert_err!(r, Error::SelfReference(e) if e == next);
        assert_eq!(db.snapshot(), before);
        // A distinct future endpoint is also rejected, without allocating a row.
        let future = Eid::new(db.meta("next_stmt") as u64 + 1);
        let r = db.try_tx(|tx| tx.create(iri("m"), iri("r"), future, Valid::ALWAYS).map(|_| ()));
        assert_err!(r, Error::NotLive(e) if e == future);
        assert_eq!(db.snapshot(), before);
    }
}

host_test! {
    /// Reserved sys namespace.
    fn reserved_namespace(db) {
        let e1 = works(db, Valid::ALWAYS).eid();
        db.tx(|_| Ok(()));
        let cases: Vec<(Value, Value, &str)> = vec![
            (sys("confirmedBy"), Value::Tx(TxId(2)), "urn:tiramemsu:sys:confirmedBy"),
            (sys("foo"), Value::Int(1), "urn:tiramemsu:sys:foo"),
            (sys("supersedes"), Value::Int(1), "urn:tiramemsu:sys:supersedes"),
            (sys("subject"), Value::Int(1), "urn:tiramemsu:sys:subject"),
            (Value::iri("urn:tiramemsu:tm:txAdded"), Value::Int(5), "urn:tiramemsu:tm:txAdded"),
        ];
        for (p, o, name) in cases {
            let r = db.try_tx(|tx| tx.assert(e1, p, o, Valid::ALWAYS).map(|_| ()));
            assert_err!(r, Error::ReservedNamespace(i) if i == name);
        }
        let r = db.try_tx(|tx| tx.assert(iri("alice"), sys("reason"), lit("x"), Valid::ALWAYS).map(|_| ()));
        assert_err!(r, Error::ReservedNamespace(i) if i == "urn:tiramemsu:sys:reason");
        let rep = db.tx(|tx| {
            tx.assert(iri("email"), sys("unique"), Value::Bool(true), Valid::ALWAYS)?;
            tx.assert(sys("db"), sys("vocab"), Value::iri("https://example.org/"), Valid::ALWAYS)?;
            tx.assert(iri("x"), iri("y"), sys("db"), Valid::ALWAYS)?;
            Ok(())
        });
        assert_eq!(rep.asserted.len(), 3);
        // sys: statements can be retracted
        let flag = rep.asserted[0];
        db.tx(|tx| {
            assert!(tx.retract(flag)?);
            Ok(())
        });
    }
}

host_test! {
    /// Eids are never reused.
    fn retract_then_reassert(db) {
        let e1 = works(db, Valid::ALWAYS).eid();
        db.tx(|tx| tx.retract(e1).map(|_| ()));
        let e2 = works(db, Valid::ALWAYS).eid();
        assert_ne!(e1, e2);
        assert!(db.row(e1).t_ret.is_some());
    }
}

// @lat: [[tests#Operations#Retract Is Once]]
host_test! {
    fn second_retraction_is_a_no_op(db) {
        let e1 = works(db, Valid::ALWAYS).eid();
        for _ in 0..2 {
            db.tx(|_| Ok(()));
        }
        let rep = db.tx(|tx| {
            assert!(tx.retract(e1)?);
            Ok(())
        });
        assert_eq!(rep.t, TxId(4));
        let row = db.row(e1);
        assert_eq!((row.t_ret, row.ret_kind), (Some(TxId(4)), Some(RetKind::Explicit)));
        db.tx(|_| Ok(()));
        let rep = db.tx(|tx| {
            assert!(!tx.retract(e1)?);
            Ok(())
        });
        assert_eq!(rep.t, TxId(6));
        assert!(rep.retracted.is_empty());
        assert_eq!(db.row(e1), row);
    }
}

host_test! {
    /// Retract sets the retraction exactly once — twice in one tx, unknown eid,
    /// assert and retract in the same transaction.
    fn retract_edge_cases(db) {
        let e1 = works(db, Valid::ALWAYS).eid();
        db.tx(|tx| {
            assert!(tx.retract(e1)?);
            assert!(!tx.retract(e1)?);
            assert!(!tx.retract(Eid::new(9_999_999))?);
            Ok(())
        });
        for _ in 0..4 {
            db.tx(|_| Ok(()));
        }
        let mut e5 = None;
        let rep = db.tx(|tx| {
            let e = tx.assert(iri("x"), iri("p"), iri("y"), Valid::ALWAYS)?.eid();
            assert!(tx.retract(e)?);
            e5 = Some(e);
            Ok(())
        });
        assert_eq!(rep.t, TxId(7));
        let row = db.row(e5.unwrap());
        assert_eq!((row.t_add, row.t_ret), (TxId(7), Some(TxId(7))));
        assert!(!db.is_live(e5.unwrap()));
        for t in 0..=8 {
            assert!(db.as_of(t).iter().all(|r| r.eid != e5.unwrap()));
        }
    }
}

host_test! {
    /// Retract by pattern.
    fn retract_by_pattern(db) {
        let mut ids = Vec::new();
        db.tx(|tx| {
            ids.push(tx.assert(iri("alice"), iri("likes"), iri("tea"), Valid::ALWAYS)?.eid());
            ids.push(tx.assert(iri("alice"), iri("likes"), iri("coffee"), Valid::ALWAYS)?.eid());
            ids.push(tx.assert(iri("alice"), iri("worksAt"), iri("acme"), Valid::between(0, 10))?.eid());
            ids.push(tx.assert(iri("alice"), iri("worksAt"), iri("acme"), Valid::between(20, 30))?.eid());
            Ok(())
        });
        let (alice, likes, works_at, acme) =
            (db.id(&iri("alice")), db.id(&iri("likes")), db.id(&iri("worksAt")), db.id(&iri("acme")));
        let mut got = Vec::new();
        let rep = db.tx(|tx| {
            got = tx.retract_matching(Some(alice), Some(likes), None)?;
            Ok(())
        });
        assert_eq!(got, ids[..2].to_vec());
        assert_eq!(rep.retracted, vec![(ids[0], RetKind::Explicit), (ids[1], RetKind::Explicit)]);
        assert!(db.is_live(ids[2]));
        // valid time does not filter
        db.tx(|tx| {
            got = tx.retract_matching(Some(alice), Some(works_at), Some(acme))?;
            Ok(())
        });
        assert_eq!(got, ids[2..].to_vec());
        // nothing matches
        let bob = db.store().speculate(|tx| tx.encode(iri("nobody")).map(|_| ()), |_| Ok(()));
        assert_ok(bob);
        db.tx(|tx| {
            let nobody = tx.encode(iri("nobody"))?;
            assert!(tx.retract_matching(Some(nobody), None, None)?.is_empty());
            Ok(())
        });
    }
}

host_test! {
    /// Retract by pattern — match reached through an earlier cascade.
    fn retract_matching_through_cascade(db) {
        let (mut e1, mut e2) = (None, None);
        db.tx(|tx| {
            let a = tx.assert(iri("alice"), iri("note"), lit("y"), Valid::ALWAYS)?.eid();
            e2 = Some(tx.assert(a, iri("note"), lit("x"), Valid::ALWAYS)?.eid());
            e1 = Some(a);
            Ok(())
        });
        let note = db.id(&iri("note"));
        let mut got = Vec::new();
        db.tx(|tx| {
            got = tx.retract_matching(None, Some(note), None)?;
            Ok(())
        });
        assert_eq!(got, vec![e1.unwrap(), e2.unwrap()]);
        assert_eq!(db.row(e1.unwrap()).ret_kind, Some(RetKind::Explicit));
        assert_eq!(db.row(e2.unwrap()).ret_kind, Some(RetKind::Cascade));
    }
}
