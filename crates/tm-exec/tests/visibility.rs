//! Spec `view-scoped-scans`: transaction-time and valid-time visibility.

mod common;

use common::*;
use tiramemsu::ir::builder::IrBuilder;
use tiramemsu::ir::{Op, Semantics, View};
use tiramemsu::*;

fn b() -> IrBuilder {
    IrBuilder::sparql()
}

fn seen(t: &TestDb, view: View, e: Eid) -> bool {
    let bb = b().at(view);
    let q = bb.query(Op::Triple(bb.t("?s", "?p", "?o").with_eid("?r")));
    run(&t.db.now(), &q)
        .column("r")
        .contains(&Some(Value::Stmt(e)))
}

// "Retracted at t" / "Added at t"
#[test]
fn transaction_time_boundaries() {
    let t = TestDb::new();
    t.advance_to(4);
    let mut e = None;
    t.tx(|tx| {
        e = Some(tx.assert(v("a"), v("p"), v("b"), Valid::ALWAYS)?.eid());
        Ok(())
    });
    let e = e.unwrap();
    assert_eq!(t.last_t(), 5);
    t.advance_to(6);
    let mut live = None;
    t.tx(|tx| {
        live = Some(tx.assert(v("c"), v("p"), v("d"), Valid::ALWAYS)?.eid());
        Ok(())
    });
    let live = live.unwrap();
    assert_eq!(t.last_t(), 7);
    t.advance_to(8);
    t.tx(|tx| tx.retract(e).map(|_| ()));
    assert_eq!(t.last_t(), 9);
    for (tx, vis) in [(4, false), (5, true), (8, true), (9, false)] {
        assert_eq!(
            seen(&t, View::as_of_tx(tx), e),
            vis,
            "retracted stmt at tx {tx}"
        );
    }
    assert!(!seen(&t, View::NOW, e));
    assert!(
        seen(&t, View::history(), e),
        "history includes retracted rows"
    );
    assert!(seen(&t, View::as_of_tx(7), live) && seen(&t, View::NOW, live));
    assert!(!seen(&t, View::as_of_tx(6), live));
}

// "Cascaded rows follow the same rule"
#[test]
fn cascaded_annotations() {
    let f = layers();
    let e1 = f.eid("e1");
    f.t.advance_to(8);
    f.t.tx(|tx| tx.retract(e1).map(|_| ()));
    assert_eq!(f.t.last_t(), 9);
    let pair = |view: View| {
        let bb = b().at(view);
        bb.query(Op::join(vec![
            Op::Triple(bb.t("?a", "v:worksAt", "?c").with_eid("?r")),
            bb.triple("?r", "v:confidence", "?conf"),
        ]))
    };
    assert_eq!(run(&f.db().now(), &pair(View::as_of_tx(8))).len(), 1);
    assert_eq!(run(&f.db().now(), &pair(View::NOW)).len(), 0);
}

const D: i64 = 1_772_323_200_000; // 2026-03-01T00:00:00Z

fn valid_seen(t: &TestDb, d: i64) -> Vec<String> {
    let bb = b().at(View::NOW.valid_at(d));
    let q = bb.query(bb.triple("v:alice", "v:worksAt", "?c"));
    rows(&run(&t.db.now(), &q))
        .into_iter()
        .map(|r| r[0].clone())
        .collect()
}

// "Half-open upper bound" / "Inclusive lower bound" / "Unbounded statement" /
// "Unfiltered includes ended facts"
#[test]
fn valid_time_boundaries() {
    let t = TestDb::new();
    t.tx(|tx| {
        tx.assert(
            v("alice"),
            v("worksAt"),
            v("acme"),
            Valid::between(D - 365 * 86_400_000, D),
        )?;
        tx.assert(v("alice"), v("worksAt"), v("forever"), Valid::ALWAYS)?;
        tx.assert(v("alice"), v("worksAt"), v("later"), Valid::from(D + 1_000))?;
        Ok(())
    });
    assert!(valid_seen(&t, D - 1).contains(&"acme".to_string()));
    assert!(!valid_seen(&t, D).contains(&"acme".to_string()));
    assert!(valid_seen(&t, D + 1_000).contains(&"later".to_string()));
    assert!(!valid_seen(&t, D + 999).contains(&"later".to_string()));
    for d in [D - 10_000_000_000, D, D + 5_000] {
        assert!(valid_seen(&t, d).contains(&"forever".to_string()));
    }
    // unfiltered: every live statement, including the ended and the future one
    let q = b().query(b().triple("v:alice", "v:worksAt", "?c"));
    assert_eq!(run(&t.db.now(), &q).len(), 3);
}

// "Episodes under valid-at"
#[test]
fn episodes_under_valid_at() {
    let t = TestDb::new();
    let day = 86_400_000i64;
    let (y2020, y2022, y2024) = (18_262 * day, 19_358 * day, 19_723 * day);
    t.tx(|tx| {
        tx.assert(
            v("alice"),
            v("worksAt"),
            v("acme"),
            Valid::between(y2020, y2022),
        )?;
        tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::from(y2024))?;
        Ok(())
    });
    let count = |d: i64| valid_seen(&t, d).len();
    assert_eq!(count(y2020 + 150 * day), 1);
    assert_eq!(count(y2022 + 100 * day), 0);
    assert_eq!(count(y2024 + 500 * day), 1);
    // bag semantics see the episode identity
    let bb = b().at(View::NOW.valid_at(y2020 + 150 * day));
    let q = IrQuery::new(
        Op::Triple(bb.t("v:alice", "v:worksAt", "?c").with_eid("?r")),
        Semantics::sparql(),
    );
    assert_eq!(run(&t.db.now(), &q).len(), 1);
}
