//! The storage contract as executable checks, run against every backend.
//!
//! Each function takes a fresh, empty backend and panics when the backend
//! breaks the contract. `tests/contract.rs` runs them on [`crate::mem::MemStore`];
//! other backend crates run them on theirs.

use std::sync::Arc;

use tm_core::vocab::v;
use tm_core::{ManualClock, RetKind, TxOptions, Valid, Value, ViewSpec};

use crate::engine::{Engine, EngineOptions};
use crate::store::{Backend, Pattern, Scope, StmtRow, TermRow};

fn row(eid: i64, s: i64, p: i64, o: i64) -> StmtRow {
    StmtRow {
        eid,
        s,
        p,
        o,
        t_add: 1,
        t_ret: None,
        v_from: None,
        v_to: None,
        ret_kind: None,
    }
}

fn seed(b: &dyn Backend) {
    let mut w = b.write().unwrap();
    for r in [
        row(13, 1, 2, 3),
        row(29, 1, 2, 4),
        row(45, 5, 2, 3),
        row(61, 1, 6, 3),
    ] {
        w.insert_statement(&r).unwrap();
    }
    w.commit().unwrap();
}

fn eids(rows: impl IntoIterator<Item = StmtRow>) -> Vec<i64> {
    let mut v: Vec<i64> = rows.into_iter().map(|r| r.eid).collect();
    v.sort_unstable();
    v
}

/// Every combination of bound positions returns exactly the matching rows.
pub fn scan_by_bound_positions(b: &dyn Backend) {
    seed(b);
    let r = b.read().unwrap();
    let scan = |s, p, o| {
        eids(
            r.scan(Pattern::new(s, p, o), Scope::Live)
                .unwrap()
                .map(|x| x.unwrap()),
        )
    };
    assert_eq!(scan(Some(1), Some(2), None), [13, 29]);
    assert_eq!(scan(None, Some(2), Some(3)), [13, 45]);
    assert_eq!(scan(Some(1), None, Some(3)), [13, 61]);
    assert_eq!(scan(Some(1), Some(2), Some(4)), [29]);
    assert_eq!(scan(Some(1), None, None), [13, 29, 61]);
    assert_eq!(scan(None, Some(6), None), [61]);
    assert_eq!(scan(None, None, Some(3)), [13, 45, 61]);
    assert_eq!(scan(None, None, None), [13, 29, 45, 61]);
    let full = r
        .scan(Pattern::new(Some(1), Some(2), Some(4)), Scope::Live)
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(full, row(29, 1, 2, 4));
}

/// `scan_eid` returns ascending eids for any bound positions.
pub fn scan_in_eid_order(b: &dyn Backend) {
    seed(b);
    let r = b.read().unwrap();
    for pat in [
        Pattern::new(None, Some(2), None),
        Pattern::new(None, None, Some(3)),
        Pattern::new(Some(1), None, None),
    ] {
        let got: Vec<i64> = r
            .scan_eid(pat, Scope::Live)
            .unwrap()
            .iter()
            .map(|x| x.eid)
            .collect();
        let mut sorted = got.clone();
        sorted.sort_unstable();
        assert_eq!(got, sorted);
    }
}

/// A retracted statement leaves the live scope and keeps its retraction in `All`.
pub fn live_and_all_scopes(b: &dyn Backend) {
    seed(b);
    let mut w = b.write().unwrap();
    assert!(w.retract(29, 2, RetKind::Explicit).unwrap());
    w.commit().unwrap();
    let r = b.read().unwrap();
    let pat = Pattern::new(Some(1), Some(2), None);
    assert_eq!(
        eids(r.scan(pat, Scope::Live).unwrap().map(|x| x.unwrap())),
        [13]
    );
    let all: Vec<StmtRow> = r.scan_eid(pat, Scope::All).unwrap();
    assert_eq!(eids(all.clone()), [13, 29]);
    let gone = all.iter().find(|x| x.eid == 29).unwrap();
    assert_eq!(
        (gone.t_ret, gone.kind()),
        (Some(2), Some(RetKind::Explicit))
    );
}

/// A second retraction returns false and changes nothing.
pub fn second_retraction_is_refused(b: &dyn Backend) {
    seed(b);
    let mut w = b.write().unwrap();
    assert!(w.retract(13, 2, RetKind::Explicit).unwrap());
    assert!(!w.retract(13, 3, RetKind::Cascade).unwrap());
    assert!(!w.retract(999, 3, RetKind::Cascade).unwrap());
    w.commit().unwrap();
    let r = b.read().unwrap().statement(13).unwrap().unwrap();
    assert_eq!((r.t_ret, r.kind()), (Some(2), Some(RetKind::Explicit)));
}

/// Inserting an existing eid, term id or transaction fails and changes nothing.
pub fn duplicate_key_is_refused(b: &dyn Backend) {
    seed(b);
    let mut w = b.write().unwrap();
    assert!(w.insert_statement(&row(13, 9, 9, 9)).is_err());
    let t = TermRow {
        tag: 0,
        lex: "x".into(),
        dt: None,
        lang: None,
        num: None,
    };
    w.insert_term(1, &t).unwrap();
    assert!(w
        .insert_term(
            1,
            &TermRow {
                lex: "y".into(),
                ..t.clone()
            }
        )
        .is_err());
    w.insert_tx(1, 100).unwrap();
    assert!(w.insert_tx(1, 200).is_err());
    w.commit().unwrap();
    let r = b.read().unwrap();
    assert_eq!(r.statement(13).unwrap().unwrap(), row(13, 1, 2, 3));
    assert_eq!(r.term(1).unwrap().unwrap().lex, "x");
    assert_eq!(r.tx_instant(1).unwrap(), Some(100));
}

fn engine(b: Arc<dyn Backend>) -> Engine {
    Engine::open(
        b,
        EngineOptions {
            clock: Arc::new(ManualClock::new(1_000)),
            ..Default::default()
        },
    )
    .unwrap()
}

/// Asserting the same fact twice yields one statement.
pub fn assert_on_the_store(b: Arc<dyn Backend>) {
    let mut e = engine(b);
    let rep = e
        .transact(TxOptions::default(), |tx| {
            let a = tx.assert(
                Value::iri(v("alice")),
                Value::iri(v("knows")),
                Value::iri(v("bob")),
                Valid::ALWAYS,
            )?;
            let b = tx.assert(
                Value::iri(v("alice")),
                Value::iri(v("knows")),
                Value::iri(v("bob")),
                Valid::ALWAYS,
            )?;
            assert!(a.is_new() && !b.is_new());
            assert_eq!(a.eid(), b.eid());
            Ok(())
        })
        .unwrap();
    assert_eq!(rep.asserted.len(), 1);
}

/// Retracting a fact retracts its layers, breadth first, in eid order.
pub fn cascade_on_the_store(b: Arc<dyn Backend>) {
    let mut e = engine(b);
    let iri = |s: &str| Value::iri(v(s));
    let rep = e
        .transact(TxOptions::default(), |tx| {
            let f = tx
                .assert(iri("alice"), iri("worksAt"), iri("acme"), Valid::ALWAYS)?
                .eid();
            let l1 = tx
                .assert(f, iri("source"), Value::str("chat"), Valid::ALWAYS)?
                .eid();
            tx.assert(f, iri("confidence"), Value::Double(0.8), Valid::ALWAYS)?;
            tx.assert(l1, iri("checkedBy"), iri("bob"), Valid::ALWAYS)?;
            Ok(())
        })
        .unwrap();
    let [f, l1, l2, l3] = rep.asserted[..] else {
        panic!("four statements")
    };
    let rep = e
        .transact(TxOptions::default(), |tx| tx.retract(f).map(|_| ()))
        .unwrap();
    assert_eq!(
        rep.retracted,
        vec![
            (f, RetKind::Explicit),
            (l1, RetKind::Cascade),
            (l2, RetKind::Cascade),
            (l3, RetKind::Cascade)
        ]
    );
}

/// A dry run's eids are never issued again.
pub fn dry_run_burns_ids(b: Arc<dyn Backend>) {
    let mut e = engine(b);
    let iri = |s: &str| Value::iri(v(s));
    let dry = e
        .transact(
            TxOptions {
                dry_run: true,
                ..Default::default()
            },
            |tx| {
                tx.assert(iri("a"), iri("p"), Value::Int(1), Valid::ALWAYS)
                    .map(|_| ())
            },
        )
        .unwrap();
    let real = e
        .transact(TxOptions::default(), |tx| {
            tx.assert(iri("a"), iri("p"), Value::Int(2), Valid::ALWAYS)
                .map(|_| ())
        })
        .unwrap();
    assert!(real.asserted[0] > dry.asserted[0]);
    assert_eq!(real.t, dry.t, "the dry run used no transaction number");
    let all = e
        .read(|r| crate::read::triples(r, &ViewSpec::history(), None, None, None))
        .unwrap();
    assert_eq!(all.len(), 1);
}

/// An open reader keeps its snapshot across a later commit.
pub fn reader_ignores_a_later_commit(b: Arc<dyn Backend>) {
    let mut e = engine(b.clone());
    let iri = |s: &str| Value::iri(v(s));
    let old = b.read().unwrap();
    e.transact(TxOptions::default(), |tx| {
        tx.assert(iri("a"), iri("p"), Value::Int(1), Valid::ALWAYS)
            .map(|_| ())
    })
    .unwrap();
    let pat = Pattern::ANY;
    assert_eq!(old.scan(pat, Scope::Live).unwrap().count(), 0);
    assert_eq!(b.read().unwrap().scan(pat, Scope::Live).unwrap().count(), 1);
}
