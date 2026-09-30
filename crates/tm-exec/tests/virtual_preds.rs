//! Spec `virtual-predicates`: statement parts and metadata as virtual triples,
//! alias reuse, visibility, absent values, constant objects, and volatile keys.

mod common;

use common::*;
use tiramemsu::ir::builder::IrBuilder;
use tiramemsu::ir::Var;
use tiramemsu::ir::{Expr, Op, TriplePattern, View};
use tiramemsu::*;

fn b() -> IrBuilder {
    IrBuilder::sparql()
}

fn count_triple_aliases(sql: &str) -> usize {
    sql.matches("triple AS").count()
}

// virtual-predicates "Subject and object of a statement" / "Predicate of a statement" /
// "Variable predicate does not see virtual triples"
#[test]
fn statement_parts() {
    let f = layers();
    let e1 = f.eid("e1");
    let q = |p: &str| b().query(b().triple(e1, p, "?x"));
    let now = f.db().now();
    assert_eq!(rows(&run(&now, &q("sys:subject"))), expect(&[&["alice"]]));
    assert_eq!(
        rows(&run(&now, &q("sys:predicate"))),
        expect(&[&["worksAt"]])
    );
    assert_eq!(rows(&run(&now, &q("sys:object"))), expect(&[&["acme"]]));
    // a variable predicate sees only stored triples: e1 has stored triples about it
    // (confidence, confirmedBy) but never sys:subject
    let r = run(&now, &b().query(b().triple(e1, "?p", "?o").project(&["p"])));
    let ps = rows(&r);
    assert!(ps.iter().all(|p| !p[0].contains("subject")), "{ps:?}");
    // a statement with no stored triple about it yields nothing
    let e8 = f.eid("e8");
    assert!(run(&now, &b().query(b().triple(e8, "?p", "?o"))).is_empty());
}

// virtual-predicates "Virtual predicate wins over stored triples"
#[test]
fn virtual_wins_over_stored() {
    let t = TestDb::new();
    let mut e = None;
    t.tx(|tx| {
        e = Some(tx.assert(v("a"), v("p"), v("b"), Valid::ALWAYS)?.eid());
        Ok(())
    });
    let e = e.unwrap();
    // reserved namespace: only a raw SQL insert can store such a triple
    t.db.read_sql("SELECT 1").unwrap();
    let r = run(&t.db.now(), &b().query(b().triple(e, "tm:txAdded", "?t")));
    assert_eq!(rows(&r), expect(&[&["tx1"]]));
}

// virtual-predicates "Statement time on a bound eid costs no extra scan" /
// "Different views need a lookup" / "Unbound subject scans statements"
#[test]
fn alias_reuse_and_lookup() {
    let f = layers();
    let e1 = f.eid("e1");
    let q = b().query(Op::join(vec![
        Op::Triple(b().t("?a", "v:worksAt", "?c").with_eid("?r")),
        b().triple("?r", "tm:txAdded", "?t"),
    ]));
    let ex = explain(&f.db().now(), &q);
    assert_eq!(count_triple_aliases(ex.sql.as_deref().unwrap()), 1);
    let r = run(&f.db().now(), &q);
    assert_eq!(
        rows(&r),
        expect(&[&["alice", "acme", &e1.to_string(), "tx1"]])
    );
    // eid pattern under History, virtual under Now: a second alias by eid
    let h = b().at(View::history());
    let q = b().query(Op::join(vec![
        Op::Triple(h.t("?a", "v:worksAt", "?c").with_eid("?r")),
        b().triple("?r", "tm:txRetracted", "?t"),
    ]));
    let sql = explain(&f.db().now(), &q).sql.unwrap();
    assert_eq!(count_triple_aliases(&sql), 2, "{sql}");
    assert!(sql.contains("t0.eid = t1.eid"), "{sql}");
    assert!(sql.contains("t1.t_ret IS NULL"));
    // unbound subject under History: every statement added in tx 1
    let q = h.query(h.triple("?r", "tm:txAdded", Value::Tx(TxId(1))));
    let r = run(&f.db().now(), &q);
    assert_eq!(r.len(), 7, "seven statements were added in tx 1");
}

// virtual-predicates "Retracted statement under Now / History / AsOf"
#[test]
fn visibility_follows_the_view() {
    let t = TestDb::new();
    let mut e = None;
    t.tx(|tx| {
        e = Some(
            tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?
                .eid(),
        );
        Ok(())
    });
    t.advance_to(4);
    let e = e.unwrap();
    t.tx(|tx| tx.retract(e).map(|_| ()));
    assert_eq!(t.last_t(), 5);
    let with = |view: View, p: &str| {
        let bb = b().at(view);
        run(&t.db.now(), &bb.query(bb.triple(e, p, "?x")))
    };
    assert!(with(View::NOW, "sys:subject").is_empty());
    assert_eq!(
        rows(&with(View::history(), "sys:subject")),
        expect(&[&["alice"]])
    );
    assert_eq!(
        rows(&with(View::as_of_tx(4), "tm:txAdded")),
        expect(&[&["tx1"]])
    );
    assert!(with(View::as_of_tx(5), "tm:txAdded").is_empty());
}

// virtual-predicates "Live statement has no txRetracted" / "txRetracted and
// retractKind in history" / "Unbounded valid time"
#[test]
fn absent_values_produce_no_triple() {
    let t = TestDb::new();
    let d = 1_772_323_200_000i64; // 2026-03-01T00:00:00Z
    let mut es = Vec::new();
    t.tx(|tx| {
        es.push(tx.assert(v("a"), v("p"), v("b"), Valid::until(d))?.eid());
        es.push(tx.assert(v("c"), v("p"), v("d"), Valid::ALWAYS)?.eid());
        es.push(tx.assert(v("e1"), v("p"), v("f"), Valid::ALWAYS)?.eid());
        Ok(())
    });
    assert!(run(
        &t.db.now(),
        &b().query(b().triple("?r", "tm:txRetracted", "?t"))
    )
    .is_empty());
    let r = run(
        &t.db.now(),
        &b().query(b().triple(es[0], "tm:validFrom", "?x")),
    );
    assert!(r.is_empty());
    let r = run(
        &t.db.now(),
        &b().query(b().triple(es[0], "tm:validTo", "?x")),
    );
    assert_eq!(r.get(0, "x"), Some(&Value::DateTime { ms: d, tz: Some(0) }));
    // a cascade: retract a statement that has an annotation
    let mut ann = None;
    t.tx(|tx| {
        ann = Some(tx.assert(es[1], v("note"), s("x"), Valid::ALWAYS)?.eid());
        Ok(())
    });
    t.tx(|tx| tx.retract(es[1]).map(|_| ()));
    let h = b().at(View::history());
    let q = h.query(Op::join(vec![
        h.triple(ann.unwrap(), "tm:txRetracted", "?t"),
        h.triple(ann.unwrap(), "tm:retractKind", "?k"),
    ]));
    let r = run(&t.db.now(), &q);
    assert_eq!(rows(&r), expect(&[&["tx3", "1"]]));
    // explicit retraction kind 0
    let q = h.query(h.triple(es[1], "tm:retractKind", "?k"));
    assert_eq!(rows(&run(&t.db.now(), &q)), expect(&[&["0"]]));
}

// virtual-predicates "Statements added in one transaction" / "Wrong object kind" /
// "Valid-time constant compares by instant" / "Non-statement subject" /
// "Filter on statement time" / "Eid on a virtual pattern"
#[test]
fn constant_objects() {
    let t = TestDb::new();
    let d = 1_772_323_200_000i64;
    let mut e = None;
    for i in 0..3 {
        t.tx(|tx| {
            let r = tx.assert(v(&format!("s{i}")), v("p"), v("o"), Valid::from(d))?;
            e = Some(r.eid());
            Ok(())
        });
    }
    let e = e.unwrap();
    t.tx(|tx| tx.assert(v("z"), v("p"), v("o"), Valid::ALWAYS).map(|_| ()));
    t.db.optimize().unwrap();
    let q = b().query(b().triple("?r", "tm:txAdded", Value::Tx(TxId(2))));
    let ex = explain(&t.db.now(), &q);
    assert!(
        ex.query_plan.join("\n").contains("log_add"),
        "{:?}",
        ex.query_plan
    );
    assert_eq!(run(&t.db.now(), &q).len(), 1);
    let q = b().query(b().triple("?r", "tm:txAdded", Value::Int(2)));
    assert!(run(&t.db.now(), &q).is_empty());
    let iso = Value::literal(
        "2026-03-01T02:00:00+02:00",
        Some(tm_core::vocab::XSD_DATETIME),
        None,
    );
    let q = b().query(b().triple("?r", "tm:validFrom", iso));
    assert_eq!(run(&t.db.now(), &q).len(), 3);
    let q = b().query(b().triple("v:alice", "sys:subject", "?x"));
    assert!(run(&t.db.now(), &q).is_empty());
    let q = b().query(
        b().triple("?r", "tm:txAdded", "?t")
            .filter(Expr::gt(Expr::var("t"), Expr::val(Value::Tx(TxId(2))))),
    );
    let r = run(&t.db.now(), &q);
    assert_eq!(r.len(), 2, "tx3 and tx4");
    assert!(r.get(0, "r").is_some());
    let _ = e;
    let bad =
        TriplePattern::new("?r", tiramemsu::ir::vocab::TM_TX_ADDED, "?t", View::NOW).with_eid("?x");
    assert!(matches!(
        t.db.now()
            .execute_ir(&b().query(Op::Triple(bad)), &tiramemsu::Params::new()),
        Err(Error::InvalidQuery { .. })
    ));
}

// virtual-predicates "Path expression with a virtual hop" is in path_and_region.rs

fn volatile_db() -> (TestDb, Eid) {
    let t = TestDb::new();
    let ts = Value::DateTime {
        ms: 1_790_000_000_000,
        tz: Some(0),
    };
    let mut e = None;
    t.tx(|tx| {
        tx.assert(v("bob"), v("name"), s("Bob"), Valid::ALWAYS)?;
        e = Some(
            tx.assert(v("carol"), v("lastSeen"), Value::Int(1), Valid::ALWAYS)?
                .eid(),
        );
        tx.set_volatile(v("alice"), v("lastSeen"), ts.clone())?;
        tx.set_volatile(v("carol"), v("lastSeen"), Value::Int(2))?;
        Ok(())
    });
    (t, e.unwrap())
}

// virtual-predicates "Volatile value under Now" / "Stored triple wins" / "Absent under
// AsOf and History" / "Absent under valid-at" / "Not opted in" / "Volatile with a
// variable subject"
#[test]
fn volatile_keys() {
    let (t, _) = volatile_db();
    let vol = |bb: &IrBuilder, sub: &str| Op::Triple(bb.t(sub, "v:lastSeen", "?ts").volatile());
    let now = b();
    let r = run(&t.db.now(), &now.query(vol(&now, "v:alice")));
    assert_eq!(
        r.get(0, "ts"),
        Some(&Value::DateTime {
            ms: 1_790_000_000_000,
            tz: Some(0)
        })
    );
    // stored triple wins
    let r = run(&t.db.now(), &now.query(vol(&now, "v:carol")));
    assert_eq!(rows(&r), expect(&[&["1"]]));
    // absent under AsOf, History, and valid-at
    for view in [View::as_of_tx(1), View::history(), View::NOW.valid_at(5)] {
        let bb = b().at(view);
        let mut p = bb.t("v:alice", "v:lastSeen", "?ts").volatile();
        p.view = view;
        assert!(
            run(&t.db.now(), &bb.query(Op::Triple(p))).is_empty(),
            "{view:?}"
        );
    }
    // not opted in
    let r = run(
        &t.db.now(),
        &now.query(now.triple("v:alice", "v:lastSeen", "?ts")),
    );
    assert!(r.is_empty());
    // variable subject: every node with a live triple plus volatile-only nodes
    let r = run(&t.db.now(), &now.query(vol(&now, "?n").project(&["n"])));
    assert_eq!(sorted(&r), expect(&[&["alice"], &["carol"]]));
    // a volatile pattern that also binds an eid matches stored triples only
    let with_eid = Op::Triple(now.t("?n", "v:lastSeen", "?ts").volatile().with_eid("?r"));
    let r = run(&t.db.now(), &now.query(with_eid.project(&["n"])));
    assert_eq!(rows(&r), expect(&[&["carol"]]));
    let _ = Var::new("x");
}
