//! Spec `view-scoped-scans` "Stable historical results": an `AsOf(Tx(t))` IR join
//! returns the same multiset whenever it runs, whatever later transactions do.
//! (The M0 property test carries the `@lat` reference of
//! `tests#Time Travel#Historical Reads Are Stable`; this one checks the same
//! property through the IR executor.)

mod common;

use common::*;
use proptest::prelude::*;
use tiramemsu::ir::builder::IrBuilder;
use tiramemsu::ir::{Op, View};
use tiramemsu::*;

#[derive(Clone, Debug)]
enum O {
    Assert(u8, u8, u8),
    Create(u8, u8, u8),
    Retract(u8),
    Supersede(u8, u8),
    Annotate(u8, u8),
    One(u8, u8),
}

fn op() -> impl Strategy<Value = O> {
    prop_oneof![
        (0..4u8, 0..3u8, 0..4u8).prop_map(|(s, p, o)| O::Assert(s, p, o)),
        (0..4u8, 0..3u8, 0..4u8).prop_map(|(s, p, o)| O::Create(s, p, o)),
        (0..40u8).prop_map(O::Retract),
        (0..40u8, 0..4u8).prop_map(|(i, o)| O::Supersede(i, o)),
        (0..40u8, 0..3u8).prop_map(|(i, n)| O::Annotate(i, n)),
        (0..4u8, 0..4u8).prop_map(|(s, o)| O::One(s, o)),
    ]
}

fn txs() -> impl Strategy<Value = Vec<Vec<O>>> {
    prop::collection::vec(prop::collection::vec(op(), 1..5), 1..8)
}

fn node(i: u8) -> Value {
    v(&format!("n{i}"))
}

fn pred(i: u8) -> Value {
    v(&format!("p{i}"))
}

fn apply(tx: &mut Tx<'_>, o: &O, known: &[Eid]) -> Result<()> {
    let pick = |i: u8| known.get(i as usize % known.len().max(1)).copied();
    match o {
        O::Assert(s, p, x) => {
            tx.assert(node(*s), pred(*p), node(*x), Valid::ALWAYS)?;
        }
        O::Create(s, p, x) => {
            tx.create(node(*s), pred(*p), node(*x), Valid::ALWAYS)?;
        }
        O::Retract(i) => {
            if let Some(e) = pick(*i) {
                tx.retract(e)?;
            }
        }
        O::Supersede(i, x) => {
            if let Some(e) = pick(*i) {
                tx.supersede(e, Patch::object(node(*x)))?;
            }
        }
        O::Annotate(i, n) => {
            if let Some(e) = pick(*i) {
                tx.assert(e, v("conf"), Value::Int(*n as i64), Valid::ALWAYS)?;
            }
        }
        O::One(s, x) => {
            tx.assert(node(*s), v("one"), node(*x), Valid::ALWAYS)?;
        }
    }
    Ok(())
}

fn query(t: u64) -> IrQuery {
    let b = IrBuilder::sparql().at(View::as_of_tx(t));
    let stmt = |p: &str, eid: &str| Op::Triple(b.t("?s", p, "?o").with_eid(eid));
    let layered = Op::join(vec![stmt("?p", "?r"), b.triple("?r", "v:conf", "?c")]);
    let plain = b.bgp(&[("?a", "?p1", "?b"), ("?b", "?p2", "?c")]);
    b.query(Op::union(vec![
        layered.project(&["s", "o", "c"]),
        plain.project(&["a", "b", "c"]),
    ]))
}

fn record(db: &Db, t: u64) -> Vec<Vec<String>> {
    let r = db.now().execute_ir(&query(t), &Params::new()).unwrap();
    sorted(&r)
}

#[test]
fn historical_ir_reads_are_stable() {
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(32);
    let cfg = ProptestConfig::with_cases(cases);
    proptest!(cfg, |(first in txs(), later in txs())| {
        let t = TestDb::new();
        t.tx(|tx| tx.assert(v("one"), Value::iri(tiramemsu::vocab::SYS_CARDINALITY), Value::iri(tiramemsu::vocab::SYS_ONE), Valid::ALWAYS).map(|_| ()));
        // `v:one` gets its cardinality flag under its real name
        let mut known: Vec<Eid> = Vec::new();
        let mut recorded = Vec::new();
        let run_all = |batches: &Vec<Vec<O>>, known: &mut Vec<Eid>, rec: bool, recorded: &mut Vec<(u64, Vec<Vec<String>>)>| {
            for ops in batches {
                let snapshot = known.clone();
                let r = t.db.transact(TxOptions::default(), |tx| {
                    for o in ops {
                        apply(tx, o, &snapshot)?;
                    }
                    Ok(())
                });
                if let Ok(rep) = r {
                    known.extend(rep.asserted.iter().copied());
                    if rec {
                        let tn = t.last_t();
                        recorded.push((tn, record(&t.db, tn)));
                    }
                }
            }
        };
        run_all(&first, &mut known, true, &mut recorded);
        run_all(&later, &mut known, false, &mut recorded);
        for (tn, rows) in &recorded {
            prop_assert_eq!(&record(&t.db, *tn), rows, "t = {}", tn);
        }
    });
}
