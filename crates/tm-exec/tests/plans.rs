//! Spec `view-scoped-scans` "Index-friendly scans", plus plan-family and stale
//! statistics scenarios of `sql-execution` "Join order from statistics".

mod common;

use common::skewed::*;
use common::*;
use tiramemsu::ir::builder::IrBuilder;
use tiramemsu::ir::{Expr, Op, TriplePattern, View};
use tiramemsu::*;

fn b() -> IrBuilder {
    IrBuilder::sparql()
}

/// A store where every predicate used below has retracted rows (churn).
fn churned() -> (TestDb, Eid) {
    let t = TestDb::new();
    let mut e_alice = None;
    t.tx(|tx| {
        for i in 0..200 {
            tx.assert(
                v(&format!("s{i}")),
                v("worksAt"),
                v(&format!("o{}", i % 20)),
                Valid::ALWAYS,
            )?;
            tx.assert(
                v(&format!("s{i}")),
                v("name"),
                s(&format!("n{i}")),
                Valid::ALWAYS,
            )?;
        }
        e_alice = Some(
            tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::from(1_000))?
                .eid(),
        );
        tx.assert(v("bob"), v("worksAt"), v("acme"), Valid::ALWAYS)?;
        Ok(())
    });
    t.tx(|tx| {
        // retract some rows of each predicate and of the object acme
        for i in 0..150 {
            tx.retract_matching(Some(t.id(&v(&format!("s{i}")))), None, None)?;
        }
        tx.retract_matching(Some(t.id(&v("bob"))), None, None)?;
        Ok(())
    });
    t.db.optimize().unwrap();
    (t, e_alice.unwrap())
}

fn plan_of(t: &TestDb, q: &IrQuery) -> String {
    explain(&t.db.now(), q).query_plan.join("\n")
}

// Same leaf as M0's test of `tests#Storage Invariants#Views Use Covering Indexes`, which
// keeps the single `@lat` reference; this one checks the IR-generated shapes.
#[test]
fn views_use_covering_indexes() {
    let (t, e1) = churned();
    let alice = || "v:alice";
    let one = |bb: &IrBuilder, s: &str, p: &str, o: &str| bb.query(bb.triple(s, p, o));
    // Now: subject and predicate bound
    let p = plan_of(&t, &one(&b(), alice(), "v:worksAt", "?o"));
    assert!(p.contains("USING COVERING INDEX live_spo"), "{p}");
    // Now: object bound only
    let p = plan_of(&t, &one(&b(), "?s", "?p", "v:acme"));
    assert!(p.contains("USING COVERING INDEX live_osp"), "{p}");
    // Now: predicate only
    let p = plan_of(&t, &one(&b(), "?s", "v:name", "?o"));
    assert!(p.contains("USING COVERING INDEX live_pos"), "{p}");
    // AsOf: subject and predicate bound
    let asof = b().at(View::as_of_tx(1));
    let p = plan_of(&t, &one(&asof, alice(), "v:worksAt", "?o"));
    assert!(p.contains("USING COVERING INDEX hist_spo"), "{p}");
    // History: predicate bound
    let hist = b().at(View::history());
    let p = plan_of(&t, &one(&hist, "?s", "v:worksAt", "?o"));
    assert!(p.contains("USING COVERING INDEX hist_pos"), "{p}");
    // {Now, At(d)}: predicate bound
    let valid = b().at(View::NOW.valid_at(2_000));
    let p = plan_of(&t, &one(&valid, "?s", "v:worksAt", "?o"));
    assert!(p.contains("live_pos") || p.contains("valid_p"), "{p}");
    // eid equal to a constant: the integer primary key
    let q = b().query(
        Op::Triple(b().t("?s", "?p", "?o").with_eid("?r"))
            .filter(Expr::eq(Expr::var("r"), Expr::val(Value::Stmt(e1)))),
    );
    let p = plan_of(&t, &q);
    assert!(p.contains("USING INTEGER PRIMARY KEY"), "{p}");
    // tm:txAdded constant: log_add
    let q = b().query(b().triple("?r", "tm:txAdded", Value::Tx(TxId(2))));
    let p = plan_of(&t, &q);
    assert!(p.contains("log_add"), "{p}");
    // no case scans a pattern alias that has a bound position
    for q in [
        one(&b(), alice(), "v:worksAt", "?o"),
        one(&b(), "?s", "?p", "v:acme"),
        one(&hist, "?s", "v:worksAt", "?o"),
        one(&asof, alice(), "v:worksAt", "?o"),
    ] {
        let p = plan_of(&t, &q);
        assert!(!p.contains("SCAN t0"), "{p}");
    }
    let _ = TriplePattern::new("?a", "?b", "?c", View::NOW);
}

// view-scoped-scans "Churned predicate in a join uses the live index" /
// "Churn-free predicate may use either family"
#[test]
fn plan_family_in_joins() {
    let t = skewed();
    let join = |p: &str| {
        b().query(Op::join(vec![
            b().triple("?x", p, "?y"),
            b().triple("?x", "v:rare", "v:Special"),
        ]))
    };
    // churned predicate: a covering live_* index, never hist_*
    let q = b().query(Op::join(vec![
        b().triple("?x", "v:status", "?s"),
        b().triple("?x", "v:type", "v:Person"),
    ]));
    let ex = explain(&t.db.now(), &q);
    let status_alias = ex
        .query_plan
        .iter()
        .find(|l| l.contains("t0"))
        .cloned()
        .expect("status alias in plan");
    assert!(
        status_alias.contains("COVERING INDEX live_"),
        "{:?}",
        ex.query_plan
    );
    // churn-free predicate: either family, never a full scan
    let ex = explain(&t.db.now(), &join("v:knows"));
    for l in ex
        .query_plan
        .iter()
        .filter(|l| l.contains("triple") || l.contains(" t"))
    {
        assert!(!l.starts_with("SCAN"), "{:?}", ex.query_plan);
        assert!(
            l.contains("COVERING INDEX live_") || l.contains("COVERING INDEX hist_"),
            "{l}"
        );
    }
    // results equal the results under a churned copy
    let copy = skewed();
    copy.tx(|tx| {
        let ids: Vec<_> = tx
            .retract_matching(None, Some(copy.id(&v("knows"))), None)?
            .into_iter()
            .collect();
        assert!(!ids.is_empty());
        Ok(())
    });
    copy.tx(|tx| {
        // re-assert exactly the same edges
        for i in 0..PERSONS {
            for k in 1..=10 {
                let j = (i * 7 + k * 13) % PERSONS;
                if j != i {
                    tx.assert(
                        v(&format!("p{i}")),
                        v("knows"),
                        v(&format!("p{j}")),
                        Valid::ALWAYS,
                    )?;
                }
            }
        }
        Ok(())
    });
    let q = join("v:knows");
    assert_eq!(
        sorted(&run(&t.db.now(), &q)),
        sorted(&run(&copy.db.now(), &q))
    );
}

// sql-execution "Stale statistics keep results"
#[test]
fn stale_statistics_keep_results() {
    let t = skewed();
    let q = b().query(
        b().bgp(&[
            ("?x", "v:type", "v:Person"),
            ("?x", "v:rare", "v:Special"),
            ("?x", "v:works", "?o"),
        ])
        .project(&["x", "o"]),
    );
    let before = sorted(&run(&t.db.now(), &q));
    // a batch smaller than optimize_every, so no new statistics are gathered
    t.tx(|tx| {
        for i in 0..300 {
            tx.assert(v(&format!("new{i}")), v("type"), v("Person"), Valid::ALWAYS)?;
            tx.assert(
                v(&format!("new{i}")),
                v("rare"),
                v("Special"),
                Valid::ALWAYS,
            )?;
            tx.assert(v(&format!("new{i}")), v("works"), v("o1"), Valid::ALWAYS)?;
        }
        Ok(())
    });
    let stale = sorted(&run(&t.db.now(), &q));
    assert_eq!(stale.len(), before.len() + 300);
    t.db.optimize().unwrap();
    assert_eq!(sorted(&run(&t.db.now(), &q)), stale);
}
