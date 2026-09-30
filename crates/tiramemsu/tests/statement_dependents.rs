//! Spec `statement-dependents` through the facade: `View::dependents` against the
//! dry-run cascade and the layer path `(^sys:subject|^sys:object)*`.

use std::collections::BTreeSet;

use proptest::prelude::*;
use tiramemsu::*;

fn v(s: &str) -> Value {
    Value::iri(format!("urn:tiramemsu:v:{s}"))
}

fn proptest_cases(default: u32) -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// One generated operation on a layered graph; indexes pick among the eids seen so far.
#[derive(Clone, Debug)]
enum GenOp {
    Base { s: u8, o: u8 },
    Layer { target: u16, o: u8 },
    Reference { s: u8, target: u16 },
    Link { a: u16, b: u16 },
    Member { target: u16, g: u8 },
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
        1 => (any::<u16>(), 0u8..4).prop_map(|(target, o)| GenOp::Supersede { target, o }),
        2 => any::<u16>().prop_map(|target| GenOp::Retract { target }),
        1 => any::<u16>().prop_map(|target| GenOp::Cycle { target }),
    ]
}

fn pick(known: &[Eid], i: u16) -> Option<Eid> {
    (!known.is_empty()).then(|| known[i as usize % known.len()])
}

fn next_stmt(db: &Db) -> u64 {
    db.read_sql("SELECT value FROM meta WHERE key = 'next_stmt'")
        .unwrap()[0][0]
        .as_i64()
        .unwrap() as u64
}

fn apply(db: &Db, op: &GenOp, known: &[Eid]) -> Result<TxReport> {
    let node = |i: u8| v(&format!("n{i}"));
    let next = next_stmt(db);
    db.transact(TxOptions::default(), |tx| {
        match *op {
            GenOp::Base { s, o } => {
                tx.assert(node(s), v("p"), node(o), Valid::ALWAYS)?;
            }
            GenOp::Layer { target, o } => {
                if let Some(e) = pick(known, target) {
                    tx.assert(e, v("note"), Value::Int(o as i64), Valid::ALWAYS)?;
                }
            }
            GenOp::Reference { s, target } => {
                if let Some(e) = pick(known, target) {
                    tx.create(node(s), v("cites"), e, Valid::ALWAYS)?;
                }
            }
            GenOp::Link { a, b } => {
                if let (Some(a), Some(b)) = (pick(known, a), pick(known, b)) {
                    tx.create(a, v("links"), b, Valid::ALWAYS)?;
                }
            }
            GenOp::Member { target, g } => {
                if let Some(e) = pick(known, target) {
                    tx.add_to_graph(e, node(g), AssertOpts::default())?;
                }
            }
            GenOp::Supersede { target, o } => {
                if let Some(e) = pick(known, target) {
                    tx.supersede(e, Patch::object(Value::Int(100 + o as i64)))?;
                }
            }
            GenOp::Retract { target } => {
                if let Some(e) = pick(known, target) {
                    tx.retract(e)?;
                }
            }
            GenOp::Cycle { target } => {
                let x = tx.create(v("c"), v("about"), Eid::new(next + 1), Valid::ALWAYS)?;
                tx.create(x, v("about"), v("d"), Valid::ALWAYS)?;
                if let Some(e) = pick(known, target) {
                    tx.create(e, v("links"), x, Valid::ALWAYS)?;
                }
            }
        }
        Ok(())
    })
}

// Three ways to ask "what stands on e": the read, the dry-run retraction and the
// path engine's inverse layer hops. They must agree on every live statement.
// @lat: [[tests#Dependents#Dependents Match Cascade And Path]]
#[test]
fn prop_dependents_match_cascade_and_path() {
    let cfg = ProptestConfig::with_cases(proptest_cases(32));
    proptest!(cfg, |(ops in prop::collection::vec(op_strategy(), 1..40))| {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path().join("d.db"), OpenOptions::default()).unwrap();
        let mut known: Vec<Eid> = Vec::new();
        for op in &ops {
            if let Ok(rep) = apply(&db, op, &known) {
                known.extend(rep.asserted.iter().copied());
                known.extend(rep.memberships.iter().copied());
            }
        }
        let now = db.now();
        let live: BTreeSet<Eid> = now.triples(None, None, None).unwrap().iter().map(|t| t.eid).collect();
        for t in db.history().triples(None, None, None).unwrap() {
            let e = t.eid;
            let got = now.dependents(e).unwrap();
            if !live.contains(&e) {
                prop_assert!(got.is_empty());
                continue;
            }
            let got: BTreeSet<Eid> = got.into_iter().collect();
            let dry = db
                .transact(TxOptions { dry_run: true, ..TxOptions::default() }, |tx| {
                    tx.retract(e).map(|_| ())
                })
                .unwrap();
            let cascade: BTreeSet<Eid> = dry
                .retracted
                .iter()
                .chain(&dry.memberships_retracted)
                .map(|(x, _)| *x)
                .collect();
            prop_assert_eq!(&got, &cascade, "dry run of {:?}", e);
            let ends: BTreeSet<Eid> = now
                .path(e.oid(), "(^sys:subject|^sys:object)*", PathMode::Reachability, u32::MAX)
                .unwrap()
                .iter()
                .map(|r| Eid::from_oid(r.end).expect("a statement end"))
                .collect();
            prop_assert_eq!(&got, &ends, "path from {:?}", e);
        }
    });
}

// @lat: [[tests#Dependents#Dependents Reproduce The Past]]
#[test]
fn as_of_reproduces_a_retracted_structure() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("d.db"), OpenOptions::default()).unwrap();
    let r = db
        .transact(TxOptions::default(), |tx| {
            let job = tx
                .assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?
                .eid();
            let conf = tx
                .assert(job, v("confidence"), Value::Double(0.8), Valid::ALWAYS)?
                .eid();
            tx.assert(conf, v("method"), Value::str("llm"), Valid::ALWAYS)?;
            let belief = tx
                .assert(v("belief9"), v("supportedBy"), job, Valid::ALWAYS)?
                .eid();
            tx.add_to_graph(belief, v("session12"), AssertOpts::default())?;
            Ok(())
        })
        .unwrap();
    let job = r.asserted[0];
    let mut all = r.asserted.clone();
    all.extend(&r.memberships);
    all.sort();
    let before = db.now().dependents(job).unwrap();
    db.transact(TxOptions::default(), |tx| tx.retract(job).map(|_| ()))
        .unwrap();
    assert!(db.now().dependents(job).unwrap().is_empty());
    let then = db.as_of(TimeRef::Tx(r.t.0)).dependents(job).unwrap();
    assert_eq!(then, before);
    let mut sorted = then.clone();
    sorted.sort();
    assert_eq!(sorted, all);
    // the history view still sees the structure; a speculation sees its own writes
    assert_eq!(db.history().dependents(job).unwrap(), before);
    let seen = db
        .with(
            |tx| {
                let e = tx.assert(v("x"), v("p"), v("y"), Valid::ALWAYS)?.eid();
                tx.assert(e, v("note"), Value::str("new"), Valid::ALWAYS)?;
                Ok(())
            },
            |view| {
                let x = view.encode(&v("x"))?.unwrap();
                let e = view.triples(Some(x), None, None)?[0].eid;
                view.dependents(e)
            },
        )
        .unwrap();
    assert_eq!(seen.len(), 2);
}
