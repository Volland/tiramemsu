//! Spec `temporal-views`: now, as-of, history and valid-at views, the triple-pattern
//! lookup, the event log and covering-index plans.

mod common;
use common::*;

use tm_core::*;

fn eids(rows: &[Triple]) -> Vec<Eid> {
    rows.iter().map(|t| t.eid).collect()
}

fn pad_to(db: &mut TestDb, t: u64) {
    while db.last_t() + 1 < t {
        db.tx(|_| Ok(()));
    }
}

host_test! {
    /// A view is a transaction-time selector plus a valid-time selector.
    fn view_selectors(db) {
        let mut e = None;
        db.tx(|tx| {
            e = Some(tx.assert(iri("alice"), iri("worksAt"), iri("acme"),
                Valid::between(day("2020-01-01"), day("2022-01-01")))?.eid());
            Ok(())
        });
        let now = ViewSpec::default();
        assert_eq!(now, ViewSpec::NOW);
        assert_eq!(eids(&db.triples(now, None, None, None)), vec![e.unwrap()]);
        let filtered = now.valid_at(day("2026-01-01"));
        assert!(db.triples(filtered, None, None, None).is_empty());
        assert_eq!(now, ViewSpec::NOW);
        assert_eq!(eids(&db.triples(now, None, None, None)), vec![e.unwrap()]);
        // creating a view does not touch the database
        if let Some(p) = db.probe() {
            let n = p.log.lock().unwrap().len();
            let _v = ViewSpec::as_of(TimeRef::Instant(5)).valid_at(3);
            assert_eq!(p.log.lock().unwrap().len(), n);
        }
    }
}

host_test! {
    /// Now view — retracted statement disappears; uncommitted writes are invisible.
    fn now_view(db) {
        let mut e1 = None;
        db.tx(|tx| {
            e1 = Some(tx.assert(iri("a"), iri("p"), iri("b"), Valid::ALWAYS)?.eid());
            Ok(())
        });
        db.tx(|tx| tx.retract(e1.unwrap()).map(|_| ()));
        assert!(!db.is_live(e1.unwrap()));
        // another connection writes without committing
        let raw = db.raw();
        raw.execute_batch("BEGIN IMMEDIATE").unwrap();
        raw.execute(
            "INSERT INTO triple(eid, s, p, o, t_add) VALUES (?1, 1, 2, 3, 1)",
            [Eid::new(500).oid().raw()],
        )
        .unwrap();
        assert!(db.now(None, None, None).is_empty());
        raw.execute_batch("ROLLBACK").unwrap();
    }
}

host_test! {
    /// As-of view by transaction.
    fn as_of_by_transaction(db) {
        pad_to(db, 3);
        let mut e1 = None;
        db.tx(|tx| {
            e1 = Some(tx.assert(iri("a"), iri("p"), iri("b"), Valid::ALWAYS)?.eid());
            Ok(())
        });
        pad_to(db, 6);
        let rep = db.tx(|tx| tx.retract(e1.unwrap()).map(|_| ()));
        assert_eq!(rep.t, TxId(6));
        let e1 = e1.unwrap();
        for t in [3, 4, 5] {
            let rows = db.as_of(t);
            let r = rows.iter().find(|r| r.eid == e1).expect("present");
            assert_eq!((r.t_ret, r.ret_kind), (None, None));
        }
        for t in [2, 6] {
            assert!(db.as_of(t).iter().all(|r| r.eid != e1));
        }
        assert!(db.as_of(0).is_empty());
        db.tx(|tx| tx.assert(iri("c"), iri("p"), iri("d"), Valid::ALWAYS).map(|_| ()));
        let now = db.now(None, None, None);
        assert_eq!(db.as_of(9), now);
    }
}

host_test! {
    /// As-of view by instant.
    fn as_of_by_instant(db) {
        db.clock.set(1_000);
        db.tx(|tx| tx.assert(iri("a"), iri("p"), Value::Int(1), Valid::ALWAYS).map(|_| ()));
        db.clock.set(2_000);
        db.tx(|tx| tx.assert(iri("a"), iri("p"), Value::Int(2), Valid::ALWAYS).map(|_| ()));
        let at = |db: &mut TestDb, ms| db.triples(ViewSpec::as_of(TimeRef::Instant(ms)), None, None, None);
        assert_eq!(at(db, 1_500), db.as_of(1));
        assert_eq!(at(db, 2_000), db.as_of(2));
        assert!(at(db, 999).is_empty());
    }
}

host_test! {
    /// History view.
    fn history_view(db) {
        let (mut e1, mut e2) = (None, None);
        db.tx(|tx| {
            e1 = Some(tx.assert(iri("a"), iri("p"), iri("b"), Valid::ALWAYS)?.eid());
            e2 = Some(tx.assert(iri("a"), iri("p"), iri("c"), Valid::ALWAYS)?.eid());
            Ok(())
        });
        pad_to(db, 6);
        db.tx(|tx| tx.retract(e1.unwrap()).map(|_| ()));
        let h = db.history_all();
        let r1 = h.iter().find(|r| r.eid == e1.unwrap()).unwrap();
        assert_eq!((r1.t_ret, r1.ret_kind), (Some(TxId(6)), Some(RetKind::Explicit)));
        let r2 = h.iter().find(|r| r.eid == e2.unwrap()).unwrap();
        assert_eq!(r2.t_ret, None);
        let mut e5 = None;
        db.tx(|tx| {
            let e = tx.assert(iri("x"), iri("p"), iri("y"), Valid::ALWAYS)?.eid();
            tx.retract(e)?;
            e5 = Some(e);
            Ok(())
        });
        let r = db.row(e5.unwrap());
        assert_eq!((r.t_add, r.t_ret), (TxId(7), Some(TxId(7))));
        assert!(!db.is_live(e5.unwrap()));
        assert!(db.as_of(7).iter().all(|t| t.eid != e5.unwrap()));
    }
}

host_test! {
    /// Valid-at filter.
    fn valid_at_filter(db) {
        let (mut e, mut u) = (None, None);
        db.tx(|tx| {
            e = Some(tx.assert(iri("a"), iri("p"), iri("b"), Valid::between(day("2025-01-01"), day("2026-03-01")))?.eid());
            u = Some(tx.assert(iri("a"), iri("p"), iri("c"), Valid::ALWAYS)?.eid());
            Ok(())
        });
        let at = |db: &mut TestDb, d: &str| eids(&db.triples(ViewSpec::NOW.valid_at(day(d)), None, None, None));
        for d in ["2025-01-01", "2026-02-28"] {
            assert!(at(db, d).contains(&e.unwrap()), "{d}");
        }
        for d in ["2024-12-31", "2026-03-01"] {
            assert!(!at(db, d).contains(&e.unwrap()), "{d}");
        }
        for d in ["1900-01-01", "2025-06-01", "2999-01-01"] {
            assert!(at(db, d).contains(&u.unwrap()));
        }
        // combined with as-of
        let mut w = None;
        db.tx(|tx| {
            w = Some(tx.assert(iri("alice"), iri("worksAt"), iri("acme"), Valid::from(day("2020-01-01")))?.eid());
            Ok(())
        });
        pad_to(db, 9);
        let rep = db.tx(|tx| tx.supersede(w.unwrap(), Patch { v_to: Some(Some(day("2026-03-01"))), ..Patch::default() }).map(|_| ()));
        assert_eq!(rep.t, TxId(9));
        let (alice, works) = (db.id(&iri("alice")), db.id(&iri("worksAt")));
        let past = db.triples(ViewSpec::as_of(TimeRef::Tx(8)).valid_at(day("2026-06-01")), Some(alice), Some(works), None);
        assert_eq!(eids(&past), vec![w.unwrap()]);
        let now = db.triples(ViewSpec::NOW.valid_at(day("2026-06-01")), Some(alice), Some(works), None);
        assert!(now.is_empty());
    }
}

host_test! {
    /// Triple-pattern lookup.
    fn triple_pattern_lookup(db) {
        let mut ids = Vec::new();
        db.tx(|tx| {
            ids.push(tx.assert(iri("alice"), iri("likes"), iri("tea"), Valid::ALWAYS)?.eid());
            ids.push(tx.assert(iri("alice"), iri("likes"), iri("coffee"), Valid::ALWAYS)?.eid());
            ids.push(tx.assert(iri("bob"), iri("likes"), iri("tea"), Valid::ALWAYS)?.eid());
            Ok(())
        });
        let (alice, likes, tea) = (db.id(&iri("alice")), db.id(&iri("likes")), db.id(&iri("tea")));
        let r = db.now(Some(alice), Some(likes), None);
        assert_eq!(eids(&r), vec![ids[0], ids[1]]);
        let full = &r[0];
        assert_eq!((full.s, full.p, full.o, full.t_add), (alice, likes, tea, TxId(1)));
        assert_eq!(eids(&db.now(None, None, Some(tea))), vec![ids[0], ids[2]]);
        let terms = db.count("term");
        assert_eq!(db.oid(&lit("twenty-byte string!!")), None);
        assert_eq!(db.count("term"), terms);
        db.tx(|tx| tx.retract(ids[2]).map(|_| ()));
        assert_eq!(eids(&db.history_all()), ids);
        // engine statements are included
        db.tx(|tx| tx.confirm(ids[0]).map(|_| ()));
        assert_eq!(db.now(Some(ids[0].oid()), None, None).len(), 1);
    }
}

host_test! {
    /// Event log since a transaction.
    fn event_log(db) {
        pad_to(db, 4);
        let (mut e1, mut e2) = (None, None);
        db.tx(|tx| {
            e1 = Some(tx.assert(iri("a"), iri("p"), iri("b"), Valid::ALWAYS)?.eid());
            e2 = Some(tx.assert(e1.unwrap(), iri("note"), lit("x"), Valid::ALWAYS)?.eid());
            Ok(())
        });
        db.tx(|tx| tx.retract(e1.unwrap()).map(|_| ()));
        let (e1, e2) = (e1.unwrap(), e2.unwrap());
        let ev = |t, eid, op, kind| Event { t: TxId(t), eid, op, kind };
        assert_eq!(db.events_since(3), vec![
            ev(4, e1, Op::Assert, None),
            ev(4, e2, Op::Assert, None),
            ev(5, e1, Op::Retract, Some(RetKind::Explicit)),
            ev(5, e2, Op::Retract, Some(RetKind::Cascade)),
        ]);
        assert!(db.events_since(5).is_empty());
        pad_to(db, 7);
        let mut e5 = None;
        db.tx(|tx| {
            let e = tx.assert(iri("x"), iri("p"), iri("y"), Valid::ALWAYS)?.eid();
            tx.retract(e)?;
            e5 = Some(e);
            Ok(())
        });
        assert_eq!(db.events_since(6), vec![
            ev(7, e5.unwrap(), Op::Assert, None),
            ev(7, e5.unwrap(), Op::Retract, Some(RetKind::Explicit)),
        ]);
        // failed, dry-run and speculative transactions produce no events
        let _ = db.try_tx(|tx| {
            tx.assert(iri("q"), iri("p"), iri("r"), Valid::ALWAYS)?;
            Err(Error::custom("x"))
        });
        let _ = db.try_tx_opts(TxOptions { dry_run: true, ..TxOptions::default() },
            |tx| tx.assert(iri("q"), iri("p"), iri("r"), Valid::ALWAYS).map(|_| ()));
        let _ = db.store().speculate(|tx| tx.assert(iri("q"), iri("p"), iri("r"), Valid::ALWAYS).map(|_| ()), |_| Ok(()));
        assert!(db.events_since(7).is_empty());
    }
}

fn plan(db: &mut TestDb, spec: ViewSpec, proj: &str, bound: &[(&str, ObjectId)]) -> String {
    let mut params = Params::new();
    let mut conds: Vec<String> = bound
        .iter()
        .map(|(c, v)| format!("a.{c} = {}", params.push(v.raw())))
        .collect();
    let time = scan_predicates(&spec, "a", &mut params);
    if !time.is_empty() {
        conds.push(time);
    }
    let sql = format!(
        "EXPLAIN QUERY PLAN SELECT {proj} FROM triple a WHERE {}",
        conds.join(" AND ")
    );
    let rows = db.quiet(|db| db.read(|e| e.rows(&sql, params.values())));
    rows.iter()
        .map(|r| r[3].as_str().unwrap_or("").to_string())
        .collect::<Vec<_>>()
        .join(" | ")
}

// @lat: [[tests#Storage Invariants#Views Use Covering Indexes]]
host_test! {
    fn view_scans_use_covering_indexes(db) {
        db.tx(|tx| {
            for i in 0..50 {
                let e = tx.assert(iri(&format!("s{i}")), iri("p"), Value::Int(i % 7), Valid::between(i, i + 100))?.eid();
                if i % 3 == 0 {
                    tx.retract(e)?;
                }
                tx.assert(iri(&format!("s{i}")), iri("q"), iri("o"), Valid::ALWAYS)?;
            }
            Ok(())
        });
        let (s, p, o) = (db.id(&iri("s1")), db.id(&iri("p")), db.id(&iri("o")));
        let now = plan(db, ViewSpec::NOW, "a.o, a.eid", &[("s", s), ("p", p)]);
        assert!(now.contains("COVERING INDEX live_spo"), "{now}");
        let asof = plan(db, ViewSpec::as_of(TimeRef::Tx(1)), "a.o, a.eid", &[("s", s), ("p", p)]);
        assert!(asof.contains("COVERING INDEX hist_spo"), "{asof}");
        let inst = plan(db, ViewSpec::as_of(TimeRef::Instant(T0)), "a.o, a.eid", &[("s", s), ("p", p)]);
        assert!(inst.contains("COVERING INDEX hist_spo"), "{inst}");
        let hist = plan(db, ViewSpec::history(), "a.o, a.eid", &[("s", s), ("p", p)]);
        assert!(hist.contains("COVERING INDEX hist_spo"), "{hist}");
        let valid = plan(db, ViewSpec::NOW.valid_at(50), "a.s, a.eid", &[("p", p)]);
        assert!(
            valid.contains("COVERING INDEX live_") || valid.contains("INDEX valid_p"),
            "{valid}"
        );
        let obj = plan(db, ViewSpec::NOW, "a.s, a.eid", &[("o", o)]);
        assert!(obj.contains("COVERING INDEX live_osp"), "{obj}");
        let obj_asof = plan(db, ViewSpec::as_of(TimeRef::Tx(1)), "a.s, a.eid", &[("o", o)]);
        assert!(obj_asof.contains("COVERING INDEX hist_osp"), "{obj_asof}");
        for pl in [&now, &asof, &inst, &hist, &valid, &obj, &obj_asof] {
            assert!(!pl.contains("SCAN a") || pl.contains("INDEX"), "{pl}");
        }
    }
}
