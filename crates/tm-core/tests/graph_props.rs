//! Property test: `graph_members` equals a model of memberships as a set of
//! `(eid, graph)` pairs, now and as of every earlier transaction.

mod common;
use common::*;

use std::collections::BTreeSet;

use proptest::prelude::*;
use tm_core::*;

const GRAPHS: u8 = 3;

#[derive(Clone, Debug)]
enum Op {
    Assert(u8),
    Add(u8, u8),
    Remove(u8, u8),
    Clear(u8),
    Retract(u8),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        (0u8..6).prop_map(Op::Assert),
        (0u8..12, 0..GRAPHS).prop_map(|(s, g)| Op::Add(s, g)),
        (0u8..12, 0..GRAPHS).prop_map(|(s, g)| Op::Remove(s, g)),
        (0..GRAPHS).prop_map(Op::Clear),
        (0u8..12).prop_map(Op::Retract),
    ]
}

fn graph(n: u8) -> Value {
    iri(&format!("graph{n}"))
}

// @lat: [[tests#Named Graphs#Membership Matches A Model]]
proptest! {
    #![proptest_config(ProptestConfig::with_cases(
        std::env::var("PROPTEST_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(32)
    ))]
    #[test]
    fn graph_members_match_a_model(ops in prop::collection::vec(op(), 1..30)) {
        let mut db = TestDb::new(HostKind::Rusqlite);
        let mut eids: Vec<Eid> = Vec::new();
        let mut live: BTreeSet<Eid> = BTreeSet::new();
        let mut model: BTreeSet<(Eid, u8)> = BTreeSet::new();
        // the model after each committed transaction
        let mut history: Vec<(u64, BTreeSet<(Eid, u8)>)> = Vec::new();
        for o in ops {
            let pick = |eids: &Vec<Eid>, i: u8| eids.get(i as usize % eids.len().max(1)).copied();
            let mut changed = true;
            match o {
                Op::Assert(n) => {
                    let mut e = None;
                    db.tx(|tx| { e = Some(tx.create(iri(&format!("s{}", eids.len() + n as usize)), iri("p"), Value::Int(eids.len() as i64), Valid::ALWAYS)?); Ok(()) });
                    let e = e.unwrap();
                    eids.push(e);
                    live.insert(e);
                }
                Op::Add(s, gi) => match pick(&eids, s) {
                    Some(e) if live.contains(&e) => {
                        db.tx(|tx| tx.add_to_graph(e, graph(gi), AssertOpts::default()).map(|_| ()));
                        model.insert((e, gi));
                    }
                    _ => changed = false,
                },
                Op::Remove(s, gi) => match pick(&eids, s) {
                    Some(e) => {
                        db.tx(|tx| tx.remove_from_graph(e, graph(gi)).map(|_| ()));
                        model.remove(&(e, gi));
                    }
                    None => changed = false,
                },
                Op::Clear(gi) => {
                    db.tx(|tx| tx.clear_graph(graph(gi)).map(|_| ()));
                    model.retain(|(_, g)| *g != gi);
                }
                Op::Retract(s) => match pick(&eids, s) {
                    Some(e) if live.contains(&e) => {
                        db.tx(|tx| tx.retract(e).map(|_| ()));
                        live.remove(&e);
                        model.retain(|(x, _)| *x != e);
                    }
                    _ => changed = false,
                },
            }
            if changed {
                history.push((db.last_t(), model.clone()));
            }
        }
        let check = |db: &mut TestDb, spec: ViewSpec, want: &BTreeSet<(Eid, u8)>| {
            for gi in 0..GRAPHS {
                let g = graph(gi);
                let got: BTreeSet<Eid> = match db.store().read(|e| {
                    // a graph no statement was ever in is not interned yet
                    match TermReader::encode(e, &g)? {
                        Some(id) => read::graph_members(e, &spec, id),
                        None => Ok(Vec::new()),
                    }
                }) {
                    Ok(v) => v.into_iter().collect(),
                    Err(e) => panic!("{e:?}"),
                };
                let expect: BTreeSet<Eid> = want.iter().filter(|(_, x)| *x == gi).map(|(e, _)| *e).collect();
                assert_eq!(got, expect, "graph {gi} at {spec:?}");
            }
        };
        check(&mut db, ViewSpec::NOW, &model);
        for (t, m) in &history {
            check(&mut db, ViewSpec::as_of(TimeRef::Tx(*t)), m);
        }
    }
}
