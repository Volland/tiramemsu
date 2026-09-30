//! Property tests of the path searches against brute-force oracles, and batch-size
//! invariance of the neighbour fetcher (tasks 4.4 and 5.8).

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::*;
use proptest::prelude::*;
use tiramemsu::*;
use tm_exec::path::automaton::{Dir, Letter, LetterKind, Nfa};
use tm_exec::path::syntax;
use tm_exec::{PathEngine, PathOptions, PathRequest};
use tm_rusqlite::RusqliteExec;

const NODES: [&str; 4] = ["n0", "n1", "n2", "n3"];
const PREDS: [&str; 2] = ["p", "q"];
const EXPRS: [&str; 6] = ["p+", "(p|q)+", "p*/q?", "(p|^q){1,3}", "p/^p", "(p/q)*"];
const BOUND: u32 = 4;

type Edge = (usize, usize, usize);

fn arb_edges() -> impl Strategy<Value = Vec<Edge>> {
    prop::collection::vec((0..NODES.len(), 0..PREDS.len(), 0..NODES.len()), 1..7)
}

fn exec_of(t: &TestDb) -> RusqliteExec {
    let conn = rusqlite::Connection::open(&t.path).unwrap();
    tm_rusqlite::register::register_defaults(&conn).unwrap();
    RusqliteExec::from_connection(conn, Capabilities::default())
}

fn build(edges: &[Edge]) -> (TestDb, Vec<i64>) {
    let t = TestDb::new();
    let mut eids = Vec::new();
    t.tx(|tx| {
        for (s, p, o) in edges {
            eids.push(
                tx.create(v(NODES[*s]), v(PREDS[*p]), v(NODES[*o]), Valid::ALWAYS)?
                    .oid()
                    .raw(),
            );
        }
        Ok(())
    });
    (t, eids)
}

/// One step of a walk: `(edge index, direction, node reached)`.
type Step = (usize, Dir, usize);

/// Every walk from `start` of at most `BOUND` steps that the expression accepts.
fn walks(edges: &[Edge], nfa: &Nfa, start: usize) -> Vec<Vec<Step>> {
    let mut out = Vec::new();
    let mut stack: Vec<(Vec<Step>, usize)> = vec![(Vec::new(), start)];
    while let Some((w, at)) = stack.pop() {
        let word: Vec<Letter> = w
            .iter()
            .map(|(e, d, _)| Letter {
                kind: LetterKind::Pred {
                    iri: vi(PREDS[edges[*e].1]),
                    rv: None,
                },
                dir: *d,
            })
            .collect();
        if nfa.accepts(&word) {
            out.push(w.clone());
        }
        if w.len() as u32 == BOUND {
            continue;
        }
        for (i, (s, _, o)) in edges.iter().enumerate() {
            for (d, from, to) in [(Dir::Out, *s, *o), (Dir::In, *o, *s)] {
                if from == at {
                    let mut n = w.clone();
                    n.push((i, d, to));
                    stack.push((n, to));
                }
            }
        }
    }
    out
}

fn end_of(w: &[Step], start: usize) -> usize {
    w.last().map_or(start, |s| s.2)
}

struct Fixture {
    t: TestDb,
    eids: Vec<i64>,
    exec: RusqliteExec,
}

impl Fixture {
    fn new(edges: &[Edge]) -> Fixture {
        let (t, eids) = build(edges);
        let exec = exec_of(&t);
        Fixture { t, eids, exec }
    }

    fn run(&mut self, batch: usize, start: usize, text: &str, mode: PathMode) -> Vec<PathRow> {
        let engine = PathEngine::new(PathOptions {
            batch,
            ..PathOptions::default()
        });
        let sid = self.t.db.now().encode(&v(NODES[start])).unwrap();
        let Some(sid) = sid else { return Vec::new() };
        engine
            .eval(
                &mut self.exec,
                &PathRequest {
                    start: sid,
                    path: text,
                    mode,
                    max_hops: Some(BOUND),
                    view: ViewSpec::NOW,
                    end: None,
                    graphs: None,
                },
            )
            .unwrap()
    }

    fn node(&self, id: ObjectId) -> usize {
        let n = self.t.db.now().decode(id).unwrap();
        NODES.iter().position(|x| Value::iri(vi(x)) == n).unwrap()
    }
}

fn nfa_of(text: &str) -> Nfa {
    let e = syntax::parse(text, &mut tm_core::Vocab::default()).unwrap();
    Nfa::compile(&e).unwrap()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    // REACH ends and hop counts equal the shortest accepted walk of the product graph
    #[test]
    fn reach_equals_naive_product_walk(edges in arb_edges(), start in 0..NODES.len(), ei in 0..EXPRS.len()) {
        prop_assume!(edges.iter().any(|e| e.0 == start || e.2 == start));
        let mut f = Fixture::new(&edges);
        let nfa = nfa_of(EXPRS[ei]);
        let mut best: BTreeMap<usize, u32> = BTreeMap::new();
        for w in walks(&edges, &nfa, start) {
            let e = end_of(&w, start);
            let n = w.len() as u32;
            best.entry(e).and_modify(|m| *m = (*m).min(n)).or_insert(n);
        }
        let got: BTreeMap<usize, u32> = f
            .run(256, start, EXPRS[ei], PathMode::Reachability)
            .iter()
            .map(|r| (f.node(r.end), r.hops))
            .collect();
        prop_assert_eq!(got, best);
    }

    // TRAIL paths equal the edge-distinct accepted walks
    #[test]
    fn trail_equals_brute_force(edges in arb_edges(), start in 0..NODES.len(), ei in 0..EXPRS.len()) {
        prop_assume!(edges.iter().any(|e| e.0 == start || e.2 == start));
        let mut f = Fixture::new(&edges);
        let nfa = nfa_of(EXPRS[ei]);
        let want: BTreeSet<Vec<(i64, Dir)>> = walks(&edges, &nfa, start)
            .into_iter()
            .filter(|w| {
                let mut ids: Vec<usize> = w.iter().map(|s| s.0).collect();
                ids.sort_unstable();
                ids.dedup();
                ids.len() == w.len()
            })
            .map(|w| w.iter().map(|s| (f.eids[s.0], s.1)).collect())
            .collect();
        let rows = f.run(256, start, EXPRS[ei], PathMode::Trail);
        let got: Vec<Vec<(i64, Dir)>> = rows
            .iter()
            .map(|r| r.path.as_ref().unwrap().hops.iter().map(|h| (h.eid.raw(), h.dir)).collect())
            .collect();
        prop_assert_eq!(got.len(), got.iter().collect::<BTreeSet<_>>().len(), "no duplicate rows");
        prop_assert_eq!(got.into_iter().collect::<BTreeSet<_>>(), want);
    }

    // ALL_SHORTEST equals every minimal accepted walk per end
    #[test]
    fn all_shortest_equals_brute_force(edges in arb_edges(), start in 0..NODES.len(), ei in 0..EXPRS.len()) {
        prop_assume!(edges.iter().any(|e| e.0 == start || e.2 == start));
        let mut f = Fixture::new(&edges);
        let nfa = nfa_of(EXPRS[ei]);
        let all = walks(&edges, &nfa, start);
        let mut min: BTreeMap<usize, usize> = BTreeMap::new();
        for w in &all {
            let e = end_of(w, start);
            min.entry(e).and_modify(|m| *m = (*m).min(w.len())).or_insert(w.len());
        }
        let want: BTreeSet<Vec<(i64, Dir)>> = all
            .iter()
            .filter(|w| min[&end_of(w, start)] == w.len())
            .map(|w| w.iter().map(|s| (f.eids[s.0], s.1)).collect())
            .collect();
        let rows = f.run(256, start, EXPRS[ei], PathMode::AllShortest);
        let got: Vec<Vec<(i64, Dir)>> = rows
            .iter()
            .map(|r| r.path.as_ref().unwrap().hops.iter().map(|h| (h.eid.raw(), h.dir)).collect())
            .collect();
        prop_assert_eq!(got.len(), got.iter().collect::<BTreeSet<_>>().len(), "no duplicate rows");
        prop_assert_eq!(got.into_iter().collect::<BTreeSet<_>>(), want);
        // any-shortest: one row per end, with the minimal length
        let any = f.run(256, start, EXPRS[ei], PathMode::AnyShortest);
        let got_any: BTreeMap<usize, u32> = any.iter().map(|r| (f.node(r.end), r.hops)).collect();
        prop_assert_eq!(any.len(), got_any.len());
        prop_assert_eq!(
            got_any,
            min.iter().map(|(k, v)| (*k, *v as u32)).collect::<BTreeMap<_, _>>()
        );
    }

    // task 4.4: batch sizes 1, 7 and 256 give identical row sequences
    #[test]
    fn batch_size_does_not_change_results(edges in arb_edges(), start in 0..NODES.len(), ei in 0..EXPRS.len()) {
        prop_assume!(edges.iter().any(|e| e.0 == start || e.2 == start));
        let mut f = Fixture::new(&edges);
        for mode in [PathMode::Reachability, PathMode::Trail, PathMode::AnyShortest, PathMode::AllShortest] {
            let a = f.run(1, start, EXPRS[ei], mode);
            let b = f.run(7, start, EXPRS[ei], mode);
            let c = f.run(256, start, EXPRS[ei], mode);
            prop_assert_eq!(&a, &b);
            prop_assert_eq!(&b, &c);
        }
    }
}
