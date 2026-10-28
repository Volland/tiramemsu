//! Property of pointwise valid time (OpenSpec change `add-n3-reasoning`, task
//! 6.2): for random facts with random intervals and a fixed monotonic rule set,
//! a conclusion of the sweep holds at `t` iff the `valid_at(t)` snapshot
//! concludes it.
#![cfg(feature = "n3")]

mod n3_common;

use std::collections::BTreeSet;

use n3_common::*;
use proptest::prelude::*;
use tiramemsu::*;

/// Joins, a chain through a derived fact, and a transitive closure.
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
        0u8..4,
        any::<bool>(),
        0u8..4,
        prop::option::weighted(0.8, 0i64..100),
        prop::option::weighted(0.8, 1i64..60),
    )
        .prop_map(|(s, p, o, from, len)| F { s, p, o, from, len })
}

fn interval(f: &F) -> Valid {
    Valid {
        from: f.from,
        to: f.len.map(|l| f.from.unwrap_or(0) + l),
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

    // @tg: verifies:: [[openspec:n3-reasoning#Pointwise valid time]] {since: "2026-10-28", level: unit}
    #[test]
    fn sweep_agrees_with_snapshots(facts in prop::collection::vec(fact(), 1..10), probes in prop::collection::vec(-10i64..170, 6)) {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path().join("p.db"), OpenOptions::default()).unwrap();
        db.transact(TxOptions::default(), |tx| {
            for f in &facts {
                let p = if f.p { v("p") } else { v("q") };
                tx.create(v(&format!("n{}", f.s)), p, v(&format!("n{}", f.o)), interval(f))?;
            }
            Ok(())
        })
        .unwrap();
        let rules = text(RULES);
        let sweep = db.now().reason(rules.clone(), &ReasonOptions::default()).unwrap();
        prop_assert_eq!(&sweep.status, &ReasonStatus::Complete);
        // every bound and its neighbours, plus random instants
        let mut instants: BTreeSet<i64> = probes.into_iter().collect();
        for f in &facts {
            let v = interval(f);
            for b in [v.from, v.to].into_iter().flatten() {
                instants.extend([b - 1, b, b + 1]);
            }
        }
        for t in instants {
            let held: BTreeSet<String> = sweep
                .conclusions
                .iter()
                .filter(|c| c.valid.contains(t))
                .map(content)
                .collect();
            let snap = db.now().valid_at(t).reason(rules.clone(), &ReasonOptions::default()).unwrap();
            let snapshot: BTreeSet<String> = snap.conclusions.iter().map(content).collect();
            prop_assert_eq!(held, snapshot, "at {}", t);
        }
    }
}
