//! Property tests over random operation sequences: as-of equals replay, stable
//! historical reads, and no invariant trigger ever fires.

mod common;
use common::*;

use std::collections::{BTreeMap, BTreeSet};

use proptest::prelude::*;
use tm_core::*;

fn proptest_cases(default: u32) -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// One generated operation over a small fixed vocabulary. `target` indexes the
/// eids seen so far (modulo their number).
#[derive(Clone, Debug)]
enum GenOp {
    Assert {
        s: u8,
        p: u8,
        o: u8,
        valid: Option<(i64, i64)>,
    },
    Create {
        s: u8,
        p: u8,
        o: u8,
    },
    AssertOne {
        s: u8,
        o: u8,
        valid: Option<(i64, i64)>,
    },
    Annotate {
        target: u16,
        o: u8,
    },
    Reference {
        target: u16,
        s: u8,
    },
    Retract {
        target: u16,
    },
    RetractMatching {
        s: Option<u8>,
        p: Option<u8>,
    },
    Supersede {
        target: u16,
        o: Option<u8>,
        v_to: Option<i64>,
    },
    Confirm {
        target: u16,
    },
    Upsert {
        o: u8,
    },
    Meta {
        o: u8,
    },
    Volatile {
        s: u8,
        o: u8,
    },
}

fn valid_strategy() -> impl Strategy<Value = Option<(i64, i64)>> {
    prop::option::of((0i64..40, 1i64..40).prop_map(|(a, len)| (a, a + len)))
}

fn op_strategy() -> impl Strategy<Value = GenOp> {
    prop_oneof![
        4 => (0u8..4, 0u8..3, 0u8..6, valid_strategy())
            .prop_map(|(s, p, o, valid)| GenOp::Assert { s, p, o, valid }),
        2 => (0u8..4, 0u8..3, 0u8..6).prop_map(|(s, p, o)| GenOp::Create { s, p, o }),
        2 => (0u8..4, 0u8..6, valid_strategy()).prop_map(|(s, o, valid)| GenOp::AssertOne { s, o, valid }),
        2 => (any::<u16>(), 0u8..6).prop_map(|(target, o)| GenOp::Annotate { target, o }),
        1 => (any::<u16>(), 0u8..4).prop_map(|(target, s)| GenOp::Reference { target, s }),
        2 => any::<u16>().prop_map(|target| GenOp::Retract { target }),
        1 => (prop::option::of(0u8..4), prop::option::of(0u8..3))
            .prop_map(|(s, p)| GenOp::RetractMatching { s, p }),
        2 => (any::<u16>(), prop::option::of(0u8..6), prop::option::of(41i64..90))
            .prop_map(|(target, o, v_to)| GenOp::Supersede { target, o, v_to }),
        1 => any::<u16>().prop_map(|target| GenOp::Confirm { target }),
        1 => (0u8..6).prop_map(|o| GenOp::Upsert { o }),
        1 => (0u8..6).prop_map(|o| GenOp::Meta { o }),
        1 => (0u8..4, 0u8..6).prop_map(|(s, o)| GenOp::Volatile { s, o }),
    ]
}

fn txs_strategy() -> impl Strategy<Value = Vec<Vec<GenOp>>> {
    prop::collection::vec(prop::collection::vec(op_strategy(), 1..6), 1..10)
}

fn obj(o: u8) -> Value {
    match o {
        0..=2 => iri(&format!("o{o}")),
        3 => Value::Int(o as i64),
        4 => lit("a long literal value"),
        _ => lit("short"),
    }
}

fn valid(v: Option<(i64, i64)>) -> Valid {
    v.map_or(Valid::ALWAYS, |(a, b)| Valid::between(a, b))
}

fn pick(known: &[Eid], target: u16) -> Option<Eid> {
    (!known.is_empty()).then(|| known[target as usize % known.len()])
}

fn apply(tx: &mut Tx<'_>, op: &GenOp, known: &[Eid]) -> Result<()> {
    let s = |i: u8| iri(&format!("s{i}"));
    let p = |i: u8| iri(&format!("p{i}"));
    match op {
        GenOp::Assert {
            s: a,
            p: b,
            o,
            valid: v,
        } => {
            tx.assert(s(*a), p(*b), obj(*o), valid(*v))?;
        }
        GenOp::Create { s: a, p: b, o } => {
            tx.create(s(*a), p(*b), obj(*o), Valid::ALWAYS)?;
        }
        GenOp::AssertOne { s: a, o, valid: v } => {
            tx.assert(s(*a), iri("one"), obj(*o), valid(*v))?;
        }
        GenOp::Annotate { target, o } => {
            if let Some(e) = pick(known, *target) {
                tx.assert(e, iri("note"), obj(*o), Valid::ALWAYS)?;
            }
        }
        GenOp::Reference { target, s: a } => {
            if let Some(e) = pick(known, *target) {
                tx.create(s(*a), iri("cites"), e, Valid::ALWAYS)?;
            }
        }
        GenOp::Retract { target } => {
            if let Some(e) = pick(known, *target) {
                tx.retract(e)?;
            }
        }
        GenOp::RetractMatching { s: a, p: b } => {
            let sa = match a {
                Some(a) => Some(tx.encode(s(*a))?),
                None => None,
            };
            let pb = match b {
                Some(b) => Some(tx.encode(p(*b))?),
                None => None,
            };
            if sa.is_some() || pb.is_some() {
                tx.retract_matching(sa, pb, None)?;
            }
        }
        GenOp::Supersede { target, o, v_to } => {
            if let Some(e) = pick(known, *target) {
                let patch = Patch {
                    o: o.map(obj),
                    v_from: None,
                    v_to: v_to.map(Some),
                };
                tx.supersede(e, patch)?;
            }
        }
        GenOp::Confirm { target } => {
            if let Some(e) = pick(known, *target) {
                tx.confirm(e)?;
            }
        }
        GenOp::Upsert { o } => {
            tx.upsert(iri("uniq"), obj(*o))?;
        }
        GenOp::Meta { o } => {
            tx.meta(sys("reason"), obj(*o))?;
        }
        GenOp::Volatile { s: a, o } => {
            tx.set_volatile(s(*a), iri("seen"), obj(*o))?;
        }
    }
    Ok(())
}

/// Errors the engine may legitimately raise for a random operation.
fn expected(e: &Error) -> bool {
    matches!(
        e,
        Error::NotLive(_)
            | Error::InvalidPatch(_)
            | Error::UniqueViolation { .. }
            | Error::SelfReference(_)
            | Error::CascadeLimitExceeded { .. }
            | Error::InvalidInterval { .. }
            | Error::ValueTypeMismatch { .. }
            // superseding an engine statement (sys:confirmedBy, sys:supersedes)
            | Error::ReservedNamespace(_)
    )
}

type Content = (ObjectId, ObjectId, ObjectId, TxId, Option<i64>, Option<i64>);

fn content(t: &Triple) -> Content {
    (t.s, t.p, t.o, t.t_add, t.v_from, t.v_to)
}

/// Runs the sequence; returns the as-of results recorded right after each commit.
fn run(db: &mut TestDb, txs: &[Vec<GenOp>]) -> BTreeMap<u64, Vec<Triple>> {
    db.tx(|tx| {
        tx.assert(iri("one"), sys("cardinality"), sys("one"), Valid::ALWAYS)?;
        tx.assert(iri("uniq"), sys("unique"), Value::Bool(true), Valid::ALWAYS)?;
        Ok(())
    });
    let mut known: Vec<Eid> = Vec::new();
    let mut recorded = BTreeMap::new();
    recorded.insert(1, db.as_of(1));
    for ops in txs {
        let snapshot = known.clone();
        let r = db.try_tx(|tx| {
            for op in ops {
                apply(tx, op, &snapshot)?;
            }
            Ok(())
        });
        match r {
            Ok(rep) => {
                known.extend(rep.asserted.iter().copied());
                known.extend(rep.existing.iter().copied());
                recorded.insert(rep.t.0, db.as_of(rep.t.0));
            }
            Err(e) => assert!(expected(&e), "unexpected error (trigger abort?): {e:?}"),
        }
    }
    recorded
}

fn replay(db: &mut TestDb, upto: u64) -> BTreeSet<Eid> {
    let mut live = BTreeSet::new();
    for ev in db.events_since(0) {
        if ev.t.0 > upto {
            break;
        }
        match ev.op {
            Op::Assert => {
                live.insert(ev.eid);
            }
            Op::Retract => {
                live.remove(&ev.eid);
            }
        }
    }
    live
}

// @lat: [[tests#Time Travel#AsOf Equals Replay]]
#[test]
fn prop_as_of_equals_replay() {
    let cfg = ProptestConfig::with_cases(proptest_cases(64));
    proptest!(cfg, |(txs in txs_strategy(), minimal in any::<bool>())| {
        let mut db = TestDb::new(if minimal { HostKind::Minimal } else { HostKind::Rusqlite });
        run(&mut db, &txs);
        let history: BTreeMap<Eid, Content> =
            db.history_all().iter().map(|t| (t.eid, content(t))).collect();
        let last = db.last_t();
        for t in 0..=last {
            let as_of = db.as_of(t);
            let got: BTreeSet<Eid> = as_of.iter().map(|r| r.eid).collect();
            prop_assert_eq!(&got, &replay(&mut db, t), "t = {}", t);
            for r in &as_of {
                prop_assert_eq!(content(r), history[&r.eid]);
            }
        }
    });
}

// @lat: [[tests#Time Travel#Historical Reads Are Stable]]
#[test]
fn prop_historical_reads_are_stable() {
    let cfg = ProptestConfig::with_cases(proptest_cases(64));
    proptest!(cfg, |(first in txs_strategy(), later in txs_strategy())| {
        let mut db = TestDb::new(HostKind::Rusqlite);
        let recorded = run(&mut db, &first);
        // further random transactions (retractions, supersedes, cascades, cardinality)
        let mut known: Vec<Eid> = db.history_all().iter().map(|t| t.eid).collect();
        for ops in &later {
            let snapshot = known.clone();
            if let Ok(rep) = db.try_tx(|tx| {
                for op in ops {
                    apply(tx, op, &snapshot)?;
                }
                Ok(())
            }) {
                known.extend(rep.asserted);
            }
        }
        for (t, rows) in &recorded {
            prop_assert_eq!(&db.as_of(*t), rows, "t = {}", t);
        }
    });
}

// Spec never-forget "Engine operations under the triggers": no trigger ever fires
// and the dictionary never holds duplicate (tag, lex, dt, lang) terms.
// @lat: [[tests#Storage Invariants#Engine Never Fires A Trigger]]
#[test]
fn prop_no_trigger_fires_and_terms_stay_unique() {
    let cfg = ProptestConfig::with_cases(proptest_cases(64));
    proptest!(cfg, |(txs in txs_strategy())| {
        let mut db = TestDb::new(HostKind::Minimal);
        run(&mut db, &txs);
        let dups = db.rows(
            "SELECT tag, lex, ifnull(dt, 0), ifnull(lang, ''), count(*) FROM term \
             GROUP BY 1, 2, 3, 4 HAVING count(*) > 1",
        );
        prop_assert!(dups.is_empty(), "{:?}", dups);
        // the dry-run and speculative paths run the same operations under the triggers
        let r = db.try_tx_opts(TxOptions { dry_run: true, ..TxOptions::default() }, |tx| {
            for op in &txs[0] {
                apply(tx, op, &[])?;
            }
            Ok(())
        });
        if let Err(e) = r {
            prop_assert!(expected(&e), "{:?}", e);
        }
    });
}
