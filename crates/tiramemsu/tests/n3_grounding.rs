//! Grounding of materialized conclusions (OpenSpec change `add-n3-reasoning`,
//! task 6.3): after `materialize` and random retractions and corrections of
//! base facts, every live conclusion's support is live, and materializing again
//! reaches the same live conclusions (by content and valid time) as
//! materializing a fresh store holding only the live base facts.
#![cfg(feature = "n3")]

mod n3_common;

use std::collections::{BTreeMap, BTreeSet};

use n3_common::*;
use proptest::prelude::*;
use tiramemsu::*;

/// Joins, a chain through a derived fact, and conclusions with several
/// possible supports.
const RULES: &str = "\
{ ?a v:p ?b. ?b v:q ?c } => { ?a v:r ?c } .
{ ?a v:r ?c } => { ?c v:s ?a } .
{ ?a v:p ?b. ?b v:p ?c } => { ?a v:p2 ?c } .";

#[derive(Clone, Debug)]
struct F {
    s: u8,
    p: bool,
    o: u8,
    from: Option<i64>,
    len: Option<i64>,
}

fn fact() -> impl Strategy<Value = F> {
    (
        0u8..3,
        any::<bool>(),
        0u8..3,
        prop::option::weighted(0.8, 0i64..40),
        prop::option::weighted(0.8, 1i64..30),
    )
        .prop_map(|(s, p, o, from, len)| F { s, p, o, from, len })
}

fn interval(from: Option<i64>, len: Option<i64>) -> Valid {
    Valid {
        from,
        to: len.map(|l| from.unwrap_or(0) + l),
    }
}

/// A change to a live base fact, chosen by index among them.
#[derive(Clone, Debug)]
enum Op {
    Retract(usize),
    Supersede(usize, Option<i64>, Option<i64>),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        (0usize..64).prop_map(Op::Retract),
        (
            0usize..64,
            prop::option::weighted(0.8, 0i64..40),
            prop::option::weighted(0.8, 1i64..30)
        )
            .prop_map(|(i, from, len)| Op::Supersede(i, from, len)),
    ]
}

fn base_predicates(db: &Db) -> Vec<ObjectId> {
    [v("p"), v("q")]
        .iter()
        .filter_map(|p| db.now().encode(p).unwrap())
        .collect()
}

/// The live base facts, in eid order.
fn base(db: &Db) -> Vec<Triple> {
    let mut rows = Vec::new();
    for p in base_predicates(db) {
        rows.extend(db.now().triples(None, Some(p), None).unwrap());
    }
    rows.sort_by_key(|r| r.eid);
    rows
}

fn rule_set_statement(db: &Db) -> Eid {
    let src = db
        .now()
        .encode(&Value::iri(n3_engine::vocab::N3_SOURCE))
        .unwrap()
        .unwrap();
    db.now().triples(None, Some(src), None).unwrap()[0].eid
}

fn derived_from(db: &Db) -> Option<ObjectId> {
    db.now()
        .encode(&Value::iri(vocab::SYS_DERIVED_FROM))
        .unwrap()
}

/// Every live statement that was ever derived from `r` has every support link
/// it was written with live, and every support live.
fn assert_grounded(db: &Db, r: Eid) -> std::result::Result<(), TestCaseError> {
    let Some(df) = derived_from(db) else {
        return Ok(());
    };
    let live: BTreeSet<Eid> = db
        .now()
        .triples(None, None, None)
        .unwrap()
        .iter()
        .map(|t| t.eid)
        .collect();
    let history = db.history();
    let ever: BTreeSet<Eid> = history
        .triples(None, Some(df), Some(r.oid()))
        .unwrap()
        .iter()
        .filter_map(|t| Eid::from_oid(t.s))
        .collect();
    for d in ever.into_iter().filter(|d| live.contains(d)) {
        for link in history.triples(Some(d.oid()), Some(df), None).unwrap() {
            let support = Eid::from_oid(link.o).expect("a statement");
            prop_assert!(live.contains(&link.eid), "{d} lost link {}", link.eid);
            prop_assert!(live.contains(&support), "{d} rests on retracted {support}");
        }
    }
    Ok(())
}

/// A valid interval as `(from, to)`.
type Bounds = (Option<i64>, Option<i64>);

/// The live conclusions derived from the live rule set: content, and the union
/// of their intervals (coalesced), since one conclusion may rest on different
/// supports in different stores.
fn conclusions(db: &Db) -> BTreeMap<String, Vec<Bounds>> {
    let r = rule_set_statement(db);
    let now = db.now();
    let mut by_content: BTreeMap<String, Vec<Bounds>> = BTreeMap::new();
    let Some(df) = derived_from(db) else {
        return by_content;
    };
    let ds: BTreeSet<Eid> = now
        .triples(None, Some(df), Some(r.oid()))
        .unwrap()
        .iter()
        .filter_map(|t| Eid::from_oid(t.s))
        .collect();
    for t in now.triples(None, None, None).unwrap() {
        if !ds.contains(&t.eid) {
            continue;
        }
        let key = format!(
            "{} {} {}",
            now.decode(t.s).unwrap(),
            now.decode(t.p).unwrap(),
            now.decode(t.o).unwrap()
        );
        by_content.entry(key).or_default().push((t.v_from, t.v_to));
    }
    for intervals in by_content.values_mut() {
        // unbounded starts first
        intervals.sort_by_key(|&(from, _)| from.map_or((0, 0), |f| (1, f)));
        let mut merged: Vec<Bounds> = Vec::new();
        for &(from, to) in intervals.iter() {
            match merged.last_mut() {
                Some(last) if last.1.is_none_or(|end| from.is_some_and(|f| f <= end)) => {
                    last.1 = match (last.1, to) {
                        (Some(a), Some(b)) => Some(a.max(b)),
                        _ => None,
                    };
                }
                _ => merged.push((from, to)),
            }
        }
        *intervals = merged;
    }
    by_content
}

fn store_rules(db: &Db) -> Eid {
    let mut r = None;
    db.transact(TxOptions::default(), |tx| {
        r = Some(tx.store_rules(v("rules"), RULES)?);
        Ok(())
    })
    .unwrap();
    r.unwrap()
}

fn materialize(db: &Db) -> MaterializeReport {
    let report = db
        .materialize(&v("rules"), &MaterializeOptions::default())
        .unwrap();
    assert_eq!(report.status, ReasonStatus::Complete);
    assert!(report.stale_support.is_empty() && report.unstorable.is_empty());
    report
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, ..ProptestConfig::default() })]

    // @tg: verifies:: [[openspec:n3-materialization#Conclusions fall with premises and rules]] {since: "2026-10-28", level: integration}
    // @tg: verifies:: [[openspec:n3-materialization#Valid time of materialized conclusions]] {since: "2026-10-28", level: integration}
    #[test]
    fn conclusions_stay_grounded(
        facts in prop::collection::vec(fact(), 1..10),
        ops in prop::collection::vec(op(), 0..6),
    ) {
        for (name, _dir, db) in backends() {
            db.transact(TxOptions::default(), |tx| {
                for f in &facts {
                    let p = if f.p { v("p") } else { v("q") };
                    tx.create(v(&format!("n{}", f.s)), p, v(&format!("n{}", f.o)), interval(f.from, f.len))?;
                }
                Ok(())
            })
            .unwrap();
            let r = store_rules(&db);
            materialize(&db);
            assert_grounded(&db, r)?;
            for op in &ops {
                let live = base(&db);
                if live.is_empty() {
                    break;
                }
                db.transact(TxOptions::default(), |tx| {
                    match *op {
                        Op::Retract(i) => {
                            tx.retract(live[i % live.len()].eid)?;
                        }
                        Op::Supersede(i, from, len) => {
                            let valid = interval(from, len);
                            let row = &live[i % live.len()];
                            if (valid.from, valid.to) == (row.v_from, row.v_to) {
                                return Ok(());
                            }
                            let patch = Patch {
                                v_from: Some(valid.from),
                                v_to: Some(valid.to),
                                ..Patch::default()
                            };
                            tx.supersede(live[i % live.len()].eid, patch)?;
                        }
                    }
                    Ok(())
                })
                .unwrap();
                assert_grounded(&db, r)?;
            }
            materialize(&db);
            assert_grounded(&db, r)?;
            // a fresh store of the same backend with only the live base facts
            let (_, _fresh_dir, fresh) = backends()
                .into_iter()
                .find(|(n, _, _)| *n == name)
                .expect("the same backend");
            let rows = base(&db);
            fresh
                .transact(TxOptions::default(), |tx| {
                    for t in &rows {
                        let (s, p, o) = (db.now().decode(t.s)?, db.now().decode(t.p)?, db.now().decode(t.o)?);
                        tx.create(s, p, o, Valid { from: t.v_from, to: t.v_to })?;
                    }
                    Ok(())
                })
                .unwrap();
            store_rules(&fresh);
            materialize(&fresh);
            prop_assert_eq!(conclusions(&db), conclusions(&fresh), "{}", name);
            // and re-materializing the fresh store changes nothing
            let again = materialize(&fresh);
            prop_assert!(again.derived.is_empty() && again.widened.is_empty(), "{}", name);
        }
    }
}
