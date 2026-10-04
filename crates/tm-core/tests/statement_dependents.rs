//! Spec `statement-dependents`: the read-only cascade walk on any view.

mod common;
use common::*;

use std::collections::BTreeSet;

use proptest::prelude::*;
use tm_core::*;

fn deps(db: &mut TestDb, spec: ViewSpec, e: Eid) -> Vec<Eid> {
    db.read(|x| read::dependents(x, &spec, e))
}

/// e1 = (alice worksAt acme), e2 = (e1 confidence 0.8), e7 = (belief9 supportedBy e1),
/// e8 = (e7 method "llm-extraction"), plus (alice name "Alice").
fn layers(db: &mut TestDb) -> [Eid; 4] {
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
        tx.assert(iri("alice"), iri("name"), lit("Alice"), Valid::ALWAYS)?;
        out = Some([e1, e2, e7, e8]);
        Ok(())
    });
    out.unwrap()
}

// @lat: [[tests#Dependents#Dependents Follow Layers]]
host_test! {
    fn layers_and_references(db) {
        let [e1, e2, e7, e8] = layers(db);
        // cascade order: root, then each expansion by eid; the name of alice is not walked
        assert_eq!(deps(db, ViewSpec::NOW, e1), vec![e1, e2, e7, e8]);
        assert_eq!(deps(db, ViewSpec::NOW, e7), vec![e7, e8]);
        assert_eq!(deps(db, ViewSpec::NOW, e8), vec![e8]);
        // a dependent that is retracted is not walked, nor what stands on it
        db.tx(|tx| tx.retract(e7).map(|_| ()));
        assert_eq!(deps(db, ViewSpec::NOW, e1), vec![e1, e2]);
        // not visible: retracted, or never existed
        assert!(deps(db, ViewSpec::NOW, e7).is_empty());
        assert!(deps(db, ViewSpec::NOW, Eid::new(9_999)).is_empty());
    }
}

// @lat: [[tests#Dependents#Dependents Terminate On Cycles]]
host_test! {
    fn cycles_terminate(db) {
                let mut ids = None;
        db.tx(|tx| {
            // e7 names the eid e8 will get, and e8 is about e7
            let e7 = tx.create(iri("b1"), iri("about"), iri("b2"), Valid::ALWAYS)?;
            let e8 = tx.create(e7, iri("about"), iri("b2"), Valid::ALWAYS)?;
            let d = tx.create(e7, iri("links"), e8, Valid::ALWAYS)?;
            ids = Some((e7, e8, d));
            Ok(())
        });
        let (e7, e8, d) = ids.unwrap();
        db.legacy_object_reference(e7, e8);
        assert_eq!(deps(db, ViewSpec::NOW, e7), vec![e7, e8, d]);
        assert_eq!(deps(db, ViewSpec::NOW, e8), vec![e8, e7, d]);
        assert_eq!(deps(db, ViewSpec::history(), e8), vec![e8, e7, d]);
    }
}

// @lat: [[tests#Dependents#Dependents Follow The View]]
host_test! {
    fn views_apply(db) {
        let [e1, e2, e7, e8] = layers(db);
        let t_built = db.last_t();
        // a layer valid only in 2024
        let role = db
            .tx(|tx| {
                tx.assert(e1, iri("role"), lit("lead"), Valid::between(day("2024-01-01"), day("2025-01-01")))?;
                Ok(())
            })
            .asserted[0];
        let t_role = db.last_t();
        db.tx(|tx| tx.retract(e2).map(|_| ()));
        let late = db
            .tx(|tx| {
                tx.assert(e1, iri("note"), lit("late"), Valid::ALWAYS)?;
                Ok(())
            })
            .asserted[0];
        db.tx(|tx| tx.retract(e1).map(|_| ()));

        // what depended on e1 back then, after the whole structure was retracted
        assert!(deps(db, ViewSpec::NOW, e1).is_empty());
        let then = ViewSpec::as_of(TimeRef::Tx(t_built));
        assert_eq!(deps(db, then, e1), vec![e1, e2, e7, e8]);
        let with_role = ViewSpec::as_of(TimeRef::Tx(t_role));
        assert_eq!(deps(db, with_role, e1), vec![e1, e2, e7, role, e8]);
        // everything that ever depended, including e2 retracted before `late` existed
        assert_eq!(deps(db, ViewSpec::history(), e1), vec![e1, e2, e7, role, late, e8]);
        // valid time filters each walked statement
        assert_eq!(
            deps(db, with_role.valid_at(day("2025-06-01")), e1),
            vec![e1, e2, e7, e8]
        );
        assert_eq!(
            deps(db, with_role.valid_at(day("2024-06-01")), e1),
            vec![e1, e2, e7, role, e8]
        );
        // the instant form resolves inside the read
        let instant = db.scalar(&format!("SELECT instant FROM tx WHERE t = {t_built}"));
        assert_eq!(
            deps(db, ViewSpec::as_of(TimeRef::Instant(instant)), e1),
            vec![e1, e2, e7, e8]
        );
    }
}

// @lat: [[tests#Dependents#Dependents Are Unbounded]]
host_test! {
    fn larger_than_the_cascade_limit(db) {
        let mut root = None;
        db.tx(|tx| {
            let e = tx.assert(iri("a"), iri("p"), iri("b"), Valid::ALWAYS)?.eid();
            for i in 0..20 {
                tx.assert(e, iri("note"), Value::Int(i), Valid::ALWAYS)?;
            }
            root = Some(e);
            Ok(())
        });
        let root = root.unwrap();
        assert_eq!(deps(db, ViewSpec::NOW, root).len(), 21);
        let limited = TxOptions { max_cascade: 10, ..TxOptions::default() };
        assert_err!(
            db.try_tx_opts(limited, |tx| tx.retract(root).map(|_| ())),
            Error::CascadeLimitExceeded { .. }
        );
        // reading burns no id: the counters are unchanged by the read
        let before = db.meta("next_stmt");
        deps(db, ViewSpec::NOW, root);
        assert_eq!(db.meta("next_stmt"), before);
    }
}

fn proptest_cases(default: u32) -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// One generated operation on a layered graph. `target`, `a` and `b` index the eids
/// seen so far.
#[derive(Clone, Debug)]
enum GenOp {
    Base { s: u8, o: u8 },
    Layer { target: u16, o: u8 },
    Reference { s: u8, target: u16 },
    Link { a: u16, b: u16 },
    Member { target: u16, g: u8 },
    Confirm { target: u16 },
    Supersede { target: u16, o: u8 },
    Retract { target: u16 },
    Cycle { target: u16 },
}

fn op_strategy() -> impl Strategy<Value = GenOp> {
    prop_oneof![
        3 => (0u8..4, 0u8..4).prop_map(|(s, o)| GenOp::Base { s, o }),
        4 => (any::<u16>(), 0u8..4).prop_map(|(target, o)| GenOp::Layer { target, o }),
        2 => (0u8..4, any::<u16>()).prop_map(|(s, target)| GenOp::Reference { s, target }),
        2 => (any::<u16>(), any::<u16>()).prop_map(|(a, b)| GenOp::Link { a, b }),
        1 => (any::<u16>(), 0u8..2).prop_map(|(target, g)| GenOp::Member { target, g }),
        1 => any::<u16>().prop_map(|target| GenOp::Confirm { target }),
        1 => (any::<u16>(), 0u8..4).prop_map(|(target, o)| GenOp::Supersede { target, o }),
        2 => any::<u16>().prop_map(|target| GenOp::Retract { target }),
        1 => any::<u16>().prop_map(|target| GenOp::Cycle { target }),
    ]
}

fn pick(known: &[Eid], i: u16) -> Option<Eid> {
    (!known.is_empty()).then(|| known[i as usize % known.len()])
}

fn apply(db: &mut TestDb, op: &GenOp, known: &[Eid]) -> Result<TxReport> {
    let node = |i: u8| iri(&format!("n{i}"));
    let next = db.meta("next_stmt") as u64;
    let op = op.clone();
    let known = known.to_vec();
    db.try_tx(move |tx| {
        match op {
            GenOp::Base { s, o } => {
                tx.assert(node(s), iri("p"), node(o), Valid::ALWAYS)?;
            }
            GenOp::Layer { target, o } => {
                if let Some(e) = pick(&known, target) {
                    tx.assert(e, iri("note"), Value::Int(o as i64), Valid::ALWAYS)?;
                }
            }
            GenOp::Reference { s, target } => {
                if let Some(e) = pick(&known, target) {
                    tx.create(node(s), iri("cites"), e, Valid::ALWAYS)?;
                }
            }
            GenOp::Link { a, b } => {
                if let (Some(a), Some(b)) = (pick(&known, a), pick(&known, b)) {
                    tx.create(a, iri("links"), b, Valid::ALWAYS)?;
                }
            }
            GenOp::Member { target, g } => {
                if let Some(e) = pick(&known, target) {
                    tx.add_to_graph(e, node(g), AssertOpts::default())?;
                }
            }
            GenOp::Confirm { target } => {
                if let Some(e) = pick(&known, target) {
                    tx.confirm(e)?;
                }
            }
            GenOp::Supersede { target, o } => {
                if let Some(e) = pick(&known, target) {
                    tx.supersede(e, Patch::object(Value::Int(100 + o as i64)))?;
                }
            }
            GenOp::Retract { target } => {
                if let Some(e) = pick(&known, target) {
                    tx.retract(e)?;
                }
            }
            GenOp::Cycle { target } => {
                // x names the eid y will get, and y is about x: a two-statement cycle,
                // hung off an existing statement when there is one
                let x = tx.create(iri("c"), iri("about"), Eid::new(next + 1), Valid::ALWAYS)?;
                let y = tx.create(x, iri("about"), iri("d"), Valid::ALWAYS)?;
                assert_eq!(y, Eid::new(next + 1));
                if let Some(e) = pick(&known, target) {
                    tx.create(e, iri("links"), x, Valid::ALWAYS)?;
                }
            }
        }
        Ok(())
    })
}

// @lat: [[tests#Dependents#Dependents Match The Dry Run]]
#[test]
fn prop_dependents_match_the_dry_run() {
    let cfg = ProptestConfig::with_cases(proptest_cases(48));
    proptest!(cfg, |(ops in prop::collection::vec(op_strategy(), 1..40), minimal in any::<bool>())| {
        let mut db = TestDb::new(if minimal { HostKind::Minimal } else { HostKind::Rusqlite });
        let mut known: Vec<Eid> = Vec::new();
        for op in &ops {
            if let Ok(rep) = apply(&mut db, op, &known) {
                known.extend(rep.asserted.iter().copied());
                known.extend(rep.memberships.iter().copied());
            }
        }
        let live: BTreeSet<Eid> = db.now_eids().into_iter().collect();
        for e in db.history_all().iter().map(|t| t.eid) {
            let got = deps(&mut db, ViewSpec::NOW, e);
            if !live.contains(&e) {
                prop_assert!(got.is_empty());
                continue;
            }
            let dry = db
                .try_tx_opts(TxOptions { dry_run: true, ..TxOptions::default() }, |tx| {
                    tx.retract(e).map(|_| ())
                })
                .expect("dry run");
            let members: BTreeSet<Eid> =
                dry.memberships_retracted.iter().map(|(m, _)| *m).collect();
            let mut want: BTreeSet<Eid> = dry.retracted.iter().map(|(x, _)| *x).collect();
            want.extend(members.iter().copied());
            prop_assert_eq!(got.iter().copied().collect::<BTreeSet<_>>(), want);
            prop_assert_eq!(got.len(), got.iter().collect::<BTreeSet<_>>().len());
            // the order is the cascade's: statements (without memberships) in report order
            let ordered: Vec<Eid> = got.iter().copied().filter(|x| !members.contains(x)).collect();
            let reported: Vec<Eid> = dry.retracted.iter().map(|(x, _)| *x).collect();
            prop_assert_eq!(ordered, reported);
        }
    });
}
