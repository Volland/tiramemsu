//! Spec `retraction-cascade`: recursive retraction, cycles, limits, kinds, dry runs.

mod common;
use common::*;

use tm_core::*;

/// e1 = (alice worksAt acme), e2 = (e1 confidence 0.8), e7 = (belief9 supportedBy e1),
/// e8 = (e7 method "llm-extraction").
struct Layers {
    e1: Eid,
    e2: Eid,
    e7: Eid,
    e8: Eid,
}

fn layers(db: &mut TestDb) -> Layers {
    let mut out = None;
    db.tx(|tx| {
        let e1 = tx
            .assert(iri("alice"), iri("worksAt"), iri("acme"), Valid::ALWAYS)?
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

fn pad_to(db: &mut TestDb, t: u64) {
    while db.last_t() + 1 < t {
        db.tx(|_| Ok(()));
    }
}

// @lat: [[tests#Cascade#Cascades Subject And Object]]
host_test! {
    fn annotation_and_reference_are_retracted(db) {
        let l = layers(db);
        let mut names = Vec::new();
        db.tx(|tx| {
            names.push(tx.assert(iri("alice"), iri("name"), lit("Alice"), Valid::ALWAYS)?.eid());
            names.push(tx.assert(iri("acme"), iri("name"), lit("Acme"), Valid::ALWAYS)?.eid());
            Ok(())
        });
        pad_to(db, 12);
        let rep = db.tx(|tx| tx.retract(l.e1).map(|_| ()));
        assert_eq!(rep.t, TxId(12));
        for e in [l.e1, l.e2, l.e7, l.e8] {
            assert_eq!(db.row(e).t_ret, Some(TxId(12)), "{e}");
        }
        assert_eq!(rep.retracted.len(), 4);
        for n in names {
            assert!(db.is_live(n));
        }
    }
}

host_test! {
    /// Cascade over subject and object positions — transaction metadata survives;
    /// already retracted statements are not walked.
    fn metadata_survives_and_dead_rows_are_not_walked(db) {
        let mut e1 = None;
        db.tx(|tx| {
            e1 = Some(tx.assert(iri("alice"), iri("worksAt"), iri("acme"), Valid::ALWAYS)?.eid());
            Ok(())
        });
        let e1 = e1.unwrap();
        let (mut conf, mut author) = (None, None);
        db.tx(|tx| {
            conf = Some(tx.confirm(e1)?);
            author = Some(tx.meta(sys("author"), iri("agent7"))?);
            Ok(())
        });
        db.tx(|tx| tx.retract(e1).map(|_| ()));
        assert!(!db.is_live(conf.unwrap()));
        assert!(db.is_live(author.unwrap()));

        let (mut a, mut b, mut c) = (None, None, None);
        db.tx(|tx| {
            let x = tx.assert(iri("z"), iri("p"), iri("q"), Valid::ALWAYS)?.eid();
            let y = tx.assert(x, iri("note"), lit("a"), Valid::ALWAYS)?.eid();
            let w = tx.assert(y, iri("note"), lit("b"), Valid::ALWAYS)?.eid();
            a = Some(x);
            b = Some(y);
            c = Some(w);
            Ok(())
        });
        // retract e2 (b) alone first: its annotation e3 (c) cascades with it,
        // seed an old non-conformant layer to test traversal of legacy history
        db.tx(|tx| tx.retract(b.unwrap()).map(|_| ()));
        let old = db.row(b.unwrap());
        let mut e3 = None;
        db.tx(|tx| {
            e3 = Some(tx.assert(iri("legacy"), iri("note"), lit("late"), Valid::ALWAYS)?.eid());
            Ok(())
        });
        db.legacy_object_reference(e3.unwrap(), b.unwrap());
        let _ = c;
        db.tx(|tx| tx.retract(a.unwrap()).map(|_| ()));
        assert_eq!(db.row(b.unwrap()), old);
        assert!(db.is_live(e3.unwrap()));
    }
}

host_test! {
    /// Cascade — recursive layers reach e8.
    fn recursive_layers(db) {
        let l = layers(db);
        let rep = db.tx(|tx| tx.retract(l.e1).map(|_| ()));
        assert!(rep.retracted.iter().any(|(e, _)| *e == l.e8));
        assert_eq!(db.row(l.e8).t_ret, Some(rep.t));
    }
}

fn cycle(db: &mut TestDb) -> (Eid, Eid) {
    let mut out = None;
    db.tx(|tx| {
        let e7 = tx.create(iri("b1"), iri("about"), iri("b2"), Valid::ALWAYS)?;
        let e8 = tx.create(e7, iri("about"), iri("b2"), Valid::ALWAYS)?;
        out = Some((e7, e8));
        Ok(())
    });
    let (e7, e8) = out.unwrap();
    db.legacy_object_reference(e7, e8);
    (e7, e8)
}

// @lat: [[tests#Cascade#Cascade Terminates On Cycles]]
host_test! {
    fn two_statements_referencing_each_other(db) {
        let (e7, e8) = cycle(db);
        let rep = db.tx(|tx| tx.retract(e7).map(|_| ()));
        assert_eq!(rep.retracted, vec![(e7, RetKind::Explicit), (e8, RetKind::Cascade)]);
        assert_eq!(db.row(e7).t_ret, Some(rep.t));
        assert_eq!(db.row(e8).t_ret, Some(rep.t));
    }
}

host_test! {
    /// Cascades terminate on cycles — Diamond.
    fn diamond(db) {
        let mut ids = None;
        db.tx(|tx| {
            let e1 = tx.assert(iri("a"), iri("p"), iri("b"), Valid::ALWAYS)?.eid();
            let e2 = tx.assert(e1, iri("p"), iri("x"), Valid::ALWAYS)?.eid();
            let e3 = tx.assert(e1, iri("q"), iri("y"), Valid::ALWAYS)?.eid();
            let e4 = tx.assert(e2, iri("r"), e3, Valid::ALWAYS)?.eid();
            ids = Some((e1, e2, e3, e4));
            Ok(())
        });
        let (e1, e2, e3, e4) = ids.unwrap();
        let rep = db.tx(|tx| tx.retract(e1).map(|_| ()));
        let got: Vec<Eid> = rep.retracted.iter().map(|(e, _)| *e).collect();
        assert_eq!(got, vec![e1, e2, e3, e4]);
    }
}

fn with_annotations(db: &mut TestDb, n: usize) -> Eid {
    let mut root = None;
    db.tx(|tx| {
        let r = tx.create(iri("root"), iri("p"), iri("x"), Valid::ALWAYS)?;
        for i in 0..n {
            tx.create(r, iri("note"), Value::Int(i as i64), Valid::ALWAYS)?;
        }
        root = Some(r);
        Ok(())
    });
    root.unwrap()
}

fn limit(n: usize) -> TxOptions {
    TxOptions {
        max_cascade: n,
        ..TxOptions::default()
    }
}

// @lat: [[tests#Cascade#Cascade Limit Aborts]]
host_test! {
    fn cascade_limit_exceeded(db) {
        let root = with_annotations(db, 5);
        let before = db.snapshot();
        let r = db.try_tx_opts(limit(5), |tx| tx.retract(root).map(|_| ()));
        assert_err!(r, Error::CascadeLimitExceeded { root: r0, limit: 5 } if r0 == root);
        assert_eq!(db.snapshot(), before);
    }
}

host_test! {
    /// Cascade size limit — exactly at the limit, and per root.
    fn cascade_limit_at_limit_and_per_root(db) {
        let root = with_annotations(db, 4);
        let rep = assert_ok(db.try_tx_opts(limit(5), |tx| tx.retract(root).map(|_| ())));
        assert_eq!(rep.retracted.len(), 5);
        let mut roots = Vec::new();
        db.tx(|tx| {
            for i in 0..3 {
                let r = tx.create(iri("m"), iri("unrelated"), Value::Int(i), Valid::ALWAYS)?;
                for j in 0..3 {
                    tx.create(r, iri("note"), Value::Int(j), Valid::ALWAYS)?;
                }
                roots.push(r);
            }
            Ok(())
        });
        let p = db.id(&iri("unrelated"));
        let rep = assert_ok(db.try_tx_opts(limit(4), |tx| tx.retract_matching(None, Some(p), None).map(|_| ())));
        assert_eq!(rep.retracted.len(), 12);
    }
}

host_test! {
    /// Retraction kinds — explicit with cascade, and cardinality replacement.
    fn retraction_kinds(db) {
        let (mut e1, mut e2) = (None, None);
        db.tx(|tx| {
            let a = tx.assert(iri("x"), iri("p"), iri("y"), Valid::ALWAYS)?.eid();
            e2 = Some(tx.assert(a, iri("note"), lit("n"), Valid::ALWAYS)?.eid());
            e1 = Some(a);
            Ok(())
        });
        db.tx(|tx| tx.retract(e1.unwrap()).map(|_| ()));
        assert_eq!(db.row(e1.unwrap()).ret_kind, Some(RetKind::Explicit));
        assert_eq!(db.row(e2.unwrap()).ret_kind, Some(RetKind::Cascade));

        let (mut a1, mut a2) = (None, None);
        db.tx(|tx| {
            tx.assert(iri("age"), sys("cardinality"), sys("one"), Valid::ALWAYS)?;
            let a = tx.assert(iri("alice"), iri("age"), Value::Int(30), Valid::ALWAYS)?.eid();
            a2 = Some(tx.assert(a, iri("source"), iri("form"), Valid::ALWAYS)?.eid());
            a1 = Some(a);
            Ok(())
        });
        let rep = db.tx(|tx| tx.assert(iri("alice"), iri("age"), Value::Int(31), Valid::ALWAYS).map(|_| ()));
        assert_eq!(
            rep.retracted,
            vec![(a1.unwrap(), RetKind::Cardinality), (a2.unwrap(), RetKind::Cardinality)]
        );
    }
}

// @lat: [[tests#Cascade#Dry Run Reports Without Commit]]
host_test! {
    fn dry_run_previews_a_cascade(db) {
        let mut root = None;
        let mut all = Vec::new();
        db.tx(|tx| {
            let r = tx.assert(iri("fact"), iri("p"), iri("x"), Valid::ALWAYS)?.eid();
            all.push(r);
            for i in 0..3 {
                all.push(tx.assert(r, iri("note"), Value::Int(i), Valid::ALWAYS)?.eid());
            }
            for i in 0..2 {
                all.push(tx.assert(iri(&format!("b{i}")), iri("cites"), r, Valid::ALWAYS)?.eid());
            }
            root = Some(r);
            Ok(())
        });
        let before = db.snapshot();
        let rep = assert_ok(db.try_tx_opts(
            TxOptions { dry_run: true, ..TxOptions::default() },
            |tx| tx.retract(root.unwrap()).map(|_| ()),
        ));
        assert_eq!(rep.retracted.len(), 6);
        assert_eq!(rep.retracted[0], (root.unwrap(), RetKind::Explicit));
        assert!(rep.retracted[1..].iter().all(|(_, k)| *k == RetKind::Cascade));
        let mut got: Vec<Eid> = rep.retracted.iter().map(|(e, _)| *e).collect();
        got.sort();
        all.sort();
        assert_eq!(got, all);
        for e in &all {
            assert!(db.is_live(*e));
        }
        // only the meta table may differ, and there only id counters
        let after = db.snapshot();
        for ((t, a), (_, b)) in before.iter().zip(after.iter()) {
            if t != "meta" {
                assert_eq!(a, b, "{t}");
            }
        }
        assert_eq!(db.count("tx"), 1);
        for k in ["last_t", "last_instant", "multi_version", "format_version"] {
            let old = before[0].1.iter().find(|r| r[0].as_str() == Some(k)).unwrap().clone();
            let new = after[0].1.iter().find(|r| r[0].as_str() == Some(k)).unwrap().clone();
            assert_eq!(old, new, "{k}");
        }
    }
}

host_test! {
    /// Retracted structures remain visible in the past.
    fn what_a_belief_relied_on(db) {
        let l = layers(db);
        pad_to(db, 20);
        let rep = db.tx(|tx| tx.retract(l.e1).map(|_| ()));
        assert_eq!(rep.t, TxId(20));
        let past = db.as_of(19);
        for e in [l.e1, l.e2, l.e7] {
            let row = past.iter().find(|t| t.eid == e).expect("visible at 19");
            let hist = db.row(e);
            assert_eq!((row.s, row.p, row.o), (hist.s, hist.p, hist.o));
        }
    }
}
