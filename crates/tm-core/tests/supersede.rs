//! Spec `supersede`: cascade-and-replay, the `sys:supersedes` link, patch rules,
//! liveness, bounds, schema checks, report and event log.

mod common;
use common::*;

use tm_core::vocab::*;
use tm_core::*;

struct Layers {
    e1: Eid,
    e2: Eid,
    e7: Eid,
    e8: Eid,
}

fn layers(db: &mut TestDb, valid: Valid) -> Layers {
    let mut out = None;
    db.tx(|tx| {
        let e1 = tx
            .assert(iri("alice"), iri("worksAt"), iri("acme"), valid)?
            .eid();
        let e2 = tx
            .assert(e1, iri("confidence"), Value::Double(0.8), Valid::ALWAYS)?
            .eid();
        let e7 = tx
            .assert(iri("belief9"), iri("supportedBy"), e1, Valid::ALWAYS)?
            .eid();
        let e8 = tx
            .assert(e7, iri("method"), lit("llm-extraction"), Valid::ALWAYS)?
            .eid();
        out = Some(Layers { e1, e2, e7, e8 });
        Ok(())
    });
    out.unwrap()
}

fn supersede(db: &mut TestDb, e: Eid, patch: Patch) -> (Eid, TxReport) {
    let mut new = None;
    let rep = db.tx(|tx| {
        new = Some(tx.supersede(e, patch)?);
        Ok(())
    });
    (new.unwrap(), rep)
}

fn from(ms: i64) -> Patch {
    Patch {
        v_from: Some(Some(ms)),
        ..Patch::default()
    }
}

// @lat: [[tests#Operations#Supersede Replays Layers]]
host_test! {
    fn supersede_replays_layers(db) {
        let l = layers(db, Valid::ALWAYS);
        let feb = day("2025-02-01");
        let (e10, rep) = supersede(db, l.e1, from(feb));
        let sigma: std::collections::HashMap<Eid, Eid> = rep.superseded.iter().copied().collect();
        assert_eq!(sigma[&l.e1], e10);
        let root = db.row(e10);
        let (alice, works, acme) = (db.id(&iri("alice")), db.id(&iri("worksAt")), db.id(&iri("acme")));
        assert_eq!((root.s, root.p, root.o, root.v_from, root.v_to), (alice, works, acme, Some(feb), None));
        let conf = db.row(sigma[&l.e2]);
        assert_eq!(conf.s, e10.oid());
        assert_eq!(db.decode(conf.o), Value::Double(0.8));
        let supp = db.row(sigma[&l.e7]);
        assert_eq!((supp.s, supp.o), (db.id(&iri("belief9")), e10.oid()));
        // deep layer
        let deep = db.row(sigma[&l.e8]);
        assert_eq!(deep.s, sigma[&l.e7].oid());
        assert_eq!(db.decode(deep.o), lit("llm-extraction"));
        for e in [l.e1, l.e2, l.e7, l.e8] {
            assert_eq!(db.row(e).ret_kind, Some(RetKind::Supersede));
            assert!(!db.is_live(e));
        }
        for new in sigma.values() {
            assert!(db.is_live(*new));
        }
        let link = db.id(&Value::iri(SYS_SUPERSEDES));
        let links = db.now(Some(e10.oid()), Some(link), None);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].o, l.e1.oid());
    }
}

host_test! {
    /// Supersede retracts and replays — non-root content unchanged; cycles.
    fn non_root_content_and_cycles(db) {
        let mut ids = None;
        let n = db.meta("next_stmt") as u64;
        db.tx(|tx| {
            let root = tx.assert(iri("alice"), iri("age"), Value::Int(30), Valid::ALWAYS)?.eid();
            let ann = tx.assert(root, iri("source"), lit("form"),
                Valid::between(day("2024-01-01"), day("2025-01-01")))?.eid();
            // a cycle hanging off the root: c1 -> c2 (forward) and c2 -> c1
            let c1 = tx.create(root, iri("rel"), Eid::new(n + 3), Valid::ALWAYS)?;
            let c2 = tx.create(c1, iri("rel"), iri("z"), Valid::ALWAYS)?;
            assert_eq!(c2, Eid::new(n + 3));
            ids = Some((root, ann, c1, c2));
            Ok(())
        });
        let (root, ann, c1, c2) = ids.unwrap();
        let (_, rep) = supersede(db, root, Patch::object(Value::Int(31)));
        let sigma: std::collections::HashMap<Eid, Eid> = rep.superseded.iter().copied().collect();
        let a = db.row(sigma[&ann]);
        assert_eq!((a.v_from, a.v_to), (Some(day("2024-01-01")), Some(day("2025-01-01"))));
        assert_eq!(a.p, db.id(&iri("source")));
        assert_eq!(db.decode(a.o), lit("form"));
        let (n1, n2) = (db.row(sigma[&c1]), db.row(sigma[&c2]));
        assert_eq!(n1.o, sigma[&c2].oid());
        assert_eq!(n2.s, sigma[&c1].oid());
        for r in [&n1, &n2] {
            assert!(r.s != c1.oid() && r.s != c2.oid() && r.o != c1.oid() && r.o != c2.oid());
        }
    }
}

host_test! {
    /// Supersede links the new root to the old one — chain of corrections.
    fn chain_of_corrections(db) {
        let l = layers(db, Valid::ALWAYS);
        let (e10, rep1) = supersede(db, l.e1, from(day("2025-02-01")));
        let link1 = *rep1.asserted.last().unwrap();
        let (e20, _) = supersede(db, e10, from(day("2025-03-01")));
        let link = db.id(&Value::iri(SYS_SUPERSEDES));
        let mut objs: Vec<ObjectId> = db.now(Some(e20.oid()), Some(link), None).iter().map(|t| t.o).collect();
        objs.sort();
        let mut want = vec![e10.oid(), l.e1.oid()];
        want.sort();
        assert_eq!(objs, want);
        assert_eq!(db.row(link1).ret_kind, Some(RetKind::Supersede));
    }
}

host_test! {
    /// Patch rules — close an open interval, clear a bound, change the object.
    fn patch_rules(db) {
        let mut e = None;
        db.tx(|tx| {
            e = Some(tx.assert(iri("alice"), iri("worksAt"), iri("acme"), Valid::from(day("2020-01-01")))?.eid());
            Ok(())
        });
        let (n, _) = supersede(db, e.unwrap(), Patch { v_to: Some(Some(day("2026-03-01"))), ..Patch::default() });
        let r = db.row(n);
        assert_eq!((r.v_from, r.v_to), (Some(day("2020-01-01")), Some(day("2026-03-01"))));
        let (m, _) = supersede(db, n, Patch { v_to: Some(None), ..Patch::default() });
        let r = db.row(m);
        assert_eq!((r.v_from, r.v_to), (Some(day("2020-01-01")), None));
        let mut t = None;
        db.tx(|tx| {
            t = Some(tx.assert(iri("alice"), iri("name"), lit("Alcie"), Valid::between(1, 2))?.eid());
            Ok(())
        });
        let (k, _) = supersede(db, t.unwrap(), Patch::object(lit("Alice")));
        let r = db.row(k);
        assert_eq!(db.decode(r.o), lit("Alice"));
        assert_eq!((r.v_from, r.v_to), (Some(1), Some(2)));
    }
}

host_test! {
    /// Patch rules — empty interval and no change are rejected without trace.
    fn invalid_patches(db) {
        let mut e = None;
        db.tx(|tx| {
            e = Some(tx.assert(iri("a"), iri("p"), lit("v"), Valid::from(day("2025-06-01")))?.eid());
            Ok(())
        });
        let e = e.unwrap();
        let before = db.snapshot();
        let r = db.try_tx(|tx| tx.supersede(e, Patch { v_to: Some(Some(day("2025-01-01"))), ..Patch::default() }).map(|_| ()));
        assert_err!(r, Error::InvalidPatch(m) if m.contains("empty"));
        let r = db.try_tx(|tx| tx.supersede(e, Patch::default()).map(|_| ()));
        assert_err!(r, Error::InvalidPatch(m) if m.contains("no change"));
        let r = db.try_tx(|tx| tx.supersede(e, Patch::object(lit("v"))).map(|_| ()));
        assert_err!(r, Error::InvalidPatch(_));
        assert_eq!(db.snapshot(), before);
    }
}

host_test! {
    /// Only live statements can be superseded.
    fn only_live_statements(db) {
        let l = layers(db, Valid::ALWAYS);
        let mut x = None;
        db.tx(|tx| {
            x = Some(tx.assert(iri("x"), iri("p"), iri("y"), Valid::ALWAYS)?.eid());
            Ok(())
        });
        db.tx(|tx| tx.retract(x.unwrap()).map(|_| ()));
        let r = db.try_tx(|tx| tx.supersede(x.unwrap(), Patch::object(iri("z"))).map(|_| ()));
        assert_err!(r, Error::NotLive(e) if e == x.unwrap());
        let r = db.try_tx(|tx| tx.supersede(Eid::new(999_999), Patch::object(iri("z"))).map(|_| ()));
        assert_err!(r, Error::NotLive(_));
        let before = db.snapshot();
        let r = db.try_tx(|tx| {
            tx.supersede(l.e1, Patch::object(iri("globex")))?;
            tx.supersede(l.e1, Patch::object(iri("initech")))?;
            Ok(())
        });
        assert_err!(r, Error::NotLive(e) if e == l.e1);
        assert_eq!(db.snapshot(), before);
        let r = db.try_tx(|tx| {
            tx.supersede(l.e1, Patch::object(iri("globex")))?;
            tx.supersede(l.e2, Patch::object(Value::Double(0.9)))?;
            Ok(())
        });
        assert_err!(r, Error::NotLive(e) if e == l.e2);
    }
}

host_test! {
    /// Supersede is bounded and schema-checked.
    fn bounded_and_schema_checked(db) {
        let mut root = None;
        db.tx(|tx| {
            let r = tx.create(iri("r"), iri("p"), iri("x"), Valid::ALWAYS)?;
            for i in 0..10 {
                tx.create(r, iri("note"), Value::Int(i), Valid::ALWAYS)?;
            }
            root = Some(r);
            Ok(())
        });
        let before = db.snapshot();
        let r = db.try_tx_opts(TxOptions { max_cascade: 5, ..TxOptions::default() },
            |tx| tx.supersede(root.unwrap(), Patch::object(iri("y"))).map(|_| ()));
        assert_err!(r, Error::CascadeLimitExceeded { limit: 5, .. });
        assert_eq!(db.snapshot(), before);

        let (mut age, mut bob) = (None, None);
        db.tx(|tx| {
            tx.assert(iri("age"), sys("valueType"), sys("INT"), Valid::ALWAYS)?;
            tx.assert(iri("email"), sys("unique"), Value::Bool(true), Valid::ALWAYS)?;
            age = Some(tx.assert(iri("alice"), iri("age"), Value::Int(30), Valid::ALWAYS)?.eid());
            tx.assert(iri("alice"), iri("email"), lit("a@x.org"), Valid::ALWAYS)?;
            bob = Some(tx.assert(iri("bob"), iri("email"), lit("b@x.org"), Valid::ALWAYS)?.eid());
            Ok(())
        });
        let r = db.try_tx(|tx| tx.supersede(age.unwrap(), Patch::object(lit("thirty"))).map(|_| ()));
        assert_err!(r, Error::ValueTypeMismatch { .. });
        let alice = db.id(&iri("alice"));
        let r = db.try_tx(|tx| tx.supersede(bob.unwrap(), Patch::object(lit("a@x.org"))).map(|_| ()));
        assert_err!(r, Error::UniqueViolation { existing, .. } if existing == alice);
        // cardinality-one replacement of other live objects overlapping the new root
        let (mut w1, mut w2) = (None, None);
        db.tx(|tx| {
            tx.assert(iri("worksAt"), sys("cardinality"), sys("one"), Valid::ALWAYS)?;
            w1 = Some(tx.assert(iri("carol"), iri("worksAt"), iri("acme"), Valid::between(0, 10))?.eid());
            w2 = Some(tx.assert(iri("carol"), iri("worksAt"), iri("globex"), Valid::between(20, 30))?.eid());
            Ok(())
        });
        let (_, rep) = supersede(db, w1.unwrap(), Patch { v_to: Some(Some(25)), ..Patch::default() });
        assert!(rep.retracted.contains(&(w2.unwrap(), RetKind::Cardinality)));
        // the new root is inserted even if equal content is live
        let mut dup = None;
        db.tx(|tx| {
            let a = tx.assert(iri("d"), iri("p"), iri("x"), Valid::ALWAYS)?.eid();
            tx.create(iri("d"), iri("p"), iri("y"), Valid::ALWAYS)?;
            dup = Some(a);
            Ok(())
        });
        let (n, _) = supersede(db, dup.unwrap(), Patch::object(iri("y")));
        let d = db.id(&iri("d"));
        let live: Vec<Eid> = db.now(Some(d), None, None).iter().map(|t| t.eid).collect();
        assert!(live.contains(&n) && live.len() == 2);
    }
}

host_test! {
    /// Supersede report and event log.
    fn report_and_event_log(db) {
        let mut ids = None;
        db.tx(|tx| {
            let e1 = tx.assert(iri("alice"), iri("worksAt"), iri("acme"), Valid::ALWAYS)?.eid();
            let e2 = tx.assert(e1, iri("confidence"), Value::Double(0.8), Valid::ALWAYS)?.eid();
            ids = Some((e1, e2));
            Ok(())
        });
        let (e1, e2) = ids.unwrap();
        let t_before = db.last_t();
        let (e10, rep) = supersede(db, e1, Patch::object(iri("globex")));
        let e11 = Eid::new(e10.n() + 1);
        let e12 = Eid::new(e10.n() + 2);
        assert_eq!(rep.superseded, vec![(e1, e10), (e2, e11)]);
        assert_eq!(rep.retracted, vec![(e1, RetKind::Supersede), (e2, RetKind::Supersede)]);
        assert_eq!(rep.asserted, vec![e10, e11, e12]);
        let ev = db.events_since(t_before);
        let t = rep.t;
        let want = vec![
            Event { t, eid: e10, op: Op::Assert, kind: None },
            Event { t, eid: e11, op: Op::Assert, kind: None },
            Event { t, eid: e12, op: Op::Assert, kind: None },
            Event { t, eid: e1, op: Op::Retract, kind: Some(RetKind::Supersede) },
            Event { t, eid: e2, op: Op::Retract, kind: Some(RetKind::Supersede) },
        ];
        assert_eq!(ev, want);
    }
}
