//! Time-respecting path evaluation (`add-time-respecting-paths`): a brute-force
//! model of every mode, hand scenarios of the hop rule, the combination with graph
//! sets and layers, and the `arrival` column of `tm_path`.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::paths::*;
use common::*;
use proptest::prelude::*;
use tiramemsu::*;
use tm_exec::path::automaton::{Dir, Letter, LetterKind, Nfa};
use tm_exec::path::syntax;
use tm_exec::{PathEngine, PathOptions, PathRequest};
use tm_rusqlite::RusqliteExec;

const NODES: [&str; 4] = ["n0", "n1", "n2", "n3"];
const PREDS: [&str; 2] = ["p", "q"];
const EXPRS: [&str; 5] = ["p+", "(p|q)+", "p*/q?", "(p|^q){1,3}", "(p/q)*"];
const BOUND: u32 = 4;
const MINUS_INF: i64 = i64::MIN;

/// `(s, p, o, v_from, v_to)`.
type Edge = (usize, usize, usize, Option<i64>, Option<i64>);

fn arb_edges() -> impl Strategy<Value = Vec<Edge>> {
    let edge = (
        0..NODES.len(),
        0..PREDS.len(),
        0..NODES.len(),
        prop::option::of(0i64..6),
        prop::option::of(1i64..5),
    )
        .prop_map(|(s, p, o, from, len)| {
            // a bounded end is always after the start
            let to = len.map(|l| from.unwrap_or(0) + l);
            (s, p, o, from, to)
        });
    prop::collection::vec(edge, 2..10)
}

fn exec_of(t: &TestDb) -> RusqliteExec {
    let conn = rusqlite::Connection::open(&t.path).unwrap();
    tm_rusqlite::register::register_defaults(&conn).unwrap();
    RusqliteExec::from_connection(conn, Capabilities::default())
}

/// One step of a walk: `(edge index, direction, node reached)`.
type Step = (usize, Dir, usize);

/// A path as `(eid, direction)` hops, with its arrival.
type Timed = (Vec<(i64, Dir)>, Option<i64>);

/// A path as hop keys, with its arrival.
type Keyed = (Vec<(i64, u8, Dir)>, Option<i64>);

/// Every walk from `start` of at most `BOUND` steps that the expression accepts
/// (time is ignored here).
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
        for (i, (s, _, o, _, _)) in edges.iter().enumerate() {
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

/// The arrival of `w` under the hop rule from `tau0`, `None` when a hop is not
/// allowed: `v_to > τ`, then `τ = max(τ, v_from)`.
fn arrival_of(edges: &[Edge], w: &[Step], tau0: i64) -> Option<i64> {
    let mut tau = tau0;
    for (e, _, _) in w {
        let (_, _, _, from, to) = edges[*e];
        if to.is_some_and(|t| t <= tau) {
            return None;
        }
        tau = tau.max(from.unwrap_or(MINUS_INF));
    }
    Some(tau)
}

fn end_of(w: &[Step], start: usize) -> usize {
    w.last().map_or(start, |s| s.2)
}

fn visible(tau: i64) -> Option<i64> {
    (tau != MINUS_INF).then_some(tau)
}

struct Fixture {
    t: TestDb,
    eids: Vec<i64>,
    exec: RusqliteExec,
}

impl Fixture {
    fn new(edges: &[Edge]) -> Fixture {
        let t = TestDb::new();
        let mut eids = Vec::new();
        t.tx(|tx| {
            for (s, p, o, from, to) in edges {
                let valid = Valid {
                    from: *from,
                    to: *to,
                };
                eids.push(
                    tx.create(v(NODES[*s]), v(PREDS[*p]), v(NODES[*o]), valid)?
                        .oid()
                        .raw(),
                );
            }
            Ok(())
        });
        let exec = exec_of(&t);
        Fixture { t, eids, exec }
    }

    fn run(
        &mut self,
        start: usize,
        text: &str,
        mode: PathMode,
        after: Option<i64>,
    ) -> Vec<PathRow> {
        let engine = PathEngine::new(PathOptions::default());
        let Some(sid) = self.t.db.now().encode(&v(NODES[start])).unwrap() else {
            return Vec::new();
        };
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
                    time_respecting: Some(TimeRespecting { after }),
                },
            )
            .unwrap()
    }

    fn node(&self, id: ObjectId) -> usize {
        let n = self.t.db.now().decode(id).unwrap();
        NODES.iter().position(|x| Value::iri(vi(x)) == n).unwrap()
    }

    fn hops_of(&self, w: &[Step]) -> Vec<(i64, Dir)> {
        w.iter().map(|s| (self.eids[s.0], s.1)).collect()
    }
}

fn row_hops(r: &PathRow) -> Vec<(i64, Dir)> {
    r.path
        .as_ref()
        .unwrap()
        .hops
        .iter()
        .map(|h| (h.eid.raw(), h.dir))
        .collect()
}

fn nfa_of(text: &str) -> Nfa {
    let e = syntax::parse(text, &mut tm_core::Vocab::default()).unwrap();
    Nfa::compile(&e).unwrap()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    // @lat: [[tests#Query#Time Respecting Paths Match Brute Force]]
    #[test]
    fn time_respecting_paths_match_brute_force(
        edges in arb_edges(),
        start in 0..NODES.len(),
        ei in 0..EXPRS.len(),
        after in prop::option::of(0i64..7),
    ) {
        prop_assume!(edges.iter().any(|e| e.0 == start || e.2 == start));
        let mut f = Fixture::new(&edges);
        let nfa = nfa_of(EXPRS[ei]);
        let tau0 = after.unwrap_or(MINUS_INF);
        // every accepted walk within the bound that respects time, with its arrival
        let feasible: Vec<(Vec<Step>, i64)> = walks(&edges, &nfa, start)
            .into_iter()
            .filter_map(|w| arrival_of(&edges, &w, tau0).map(|a| (w, a)))
            .collect();

        // REACH: each end once, the shortest length, the earliest arrival
        let mut reach: BTreeMap<usize, (u32, Option<i64>)> = BTreeMap::new();
        let mut earliest: BTreeMap<usize, i64> = BTreeMap::new();
        let mut shortest: BTreeMap<usize, usize> = BTreeMap::new();
        for (w, a) in &feasible {
            let e = end_of(w, start);
            earliest.entry(e).and_modify(|x| *x = (*x).min(*a)).or_insert(*a);
            shortest.entry(e).and_modify(|x| *x = (*x).min(w.len())).or_insert(w.len());
        }
        for (e, n) in &shortest {
            reach.insert(*e, (*n as u32, visible(earliest[e])));
        }
        let rows = f.run(start, EXPRS[ei], PathMode::Reachability, after);
        let got: BTreeMap<usize, (u32, Option<i64>)> =
            rows.iter().map(|r| (f.node(r.end), (r.hops, r.arrival))).collect();
        prop_assert_eq!(rows.len(), got.len(), "each end once");
        prop_assert_eq!(&got, &reach);
        // rows in non-decreasing hops, then raw id
        let order: Vec<(u32, i64)> = rows.iter().map(|r| (r.hops, r.end.raw())).collect();
        let mut sorted_order = order.clone();
        sorted_order.sort_unstable();
        prop_assert_eq!(order, sorted_order);

        // TRAIL: the time-respecting edge-distinct walks, each with its arrival
        let want: BTreeSet<Timed> = feasible
            .iter()
            .filter(|(w, _)| {
                let mut ids: Vec<usize> = w.iter().map(|s| s.0).collect();
                ids.sort_unstable();
                ids.dedup();
                ids.len() == w.len()
            })
            .map(|(w, a)| (f.hops_of(w), visible(*a)))
            .collect();
        let trail = f.run(start, EXPRS[ei], PathMode::Trail, after);
        let got: Vec<Timed> =
            trail.iter().map(|r| (row_hops(r), r.arrival)).collect();
        prop_assert_eq!(got.len(), got.iter().collect::<BTreeSet<_>>().len(), "no duplicate rows");
        prop_assert_eq!(got.into_iter().collect::<BTreeSet<_>>(), want);

        // ALL_SHORTEST: every minimal-length time-respecting walk per end
        let minimal: Vec<&(Vec<Step>, i64)> = feasible
            .iter()
            .filter(|(w, _)| shortest[&end_of(w, start)] == w.len())
            .collect();
        let want: BTreeSet<Timed> = minimal
            .iter()
            .map(|(w, a)| (f.hops_of(w), visible(*a)))
            .collect();
        let all = f.run(start, EXPRS[ei], PathMode::AllShortest, after);
        let got: Vec<Timed> =
            all.iter().map(|r| (row_hops(r), r.arrival)).collect();
        prop_assert_eq!(got.len(), got.iter().collect::<BTreeSet<_>>().len(), "no duplicate rows");
        prop_assert_eq!(got.into_iter().collect::<BTreeSet<_>>(), want.clone());

        // ANY_SHORTEST: per end, the lexicographically smallest of those walks
        let mut first: BTreeMap<usize, Keyed> = BTreeMap::new();
        for (w, a) in &minimal {
            let key: Vec<(i64, u8, Dir)> = w.iter().map(|s| (f.eids[s.0], 0u8, s.1)).collect();
            let e = end_of(w, start);
            if first.get(&e).is_none_or(|(k, _)| key < *k) {
                first.insert(e, (key, visible(*a)));
            }
        }
        let any = f.run(start, EXPRS[ei], PathMode::AnyShortest, after);
        let got: BTreeMap<usize, Keyed> = any
            .iter()
            .map(|r| {
                let key = row_hops(r).into_iter().map(|(e, d)| (e, 0u8, d)).collect();
                (f.node(r.end), (key, r.arrival))
            })
            .collect();
        prop_assert_eq!(any.len(), got.len(), "one row per end");
        prop_assert_eq!(got, first);
    }
}

/// `(end, hops, arrival)` of a time-respecting search on `now`.
fn journey(
    g: &G,
    start: &str,
    path: &str,
    mode: PathMode,
    after: Option<i64>,
) -> Vec<(String, u32, Option<i64>)> {
    let view = g.t.db.now();
    let args = PathArgs {
        mode,
        time_respecting: Some(TimeRespecting { after }),
        ..PathArgs::default()
    };
    view.path_with(g.id(start), path, &args)
        .unwrap()
        .iter()
        .map(|r| (g.name(r.end), r.hops, r.arrival))
        .collect()
}

fn j(items: &[(&str, u32, Option<i64>)]) -> Vec<(String, u32, Option<i64>)> {
    items
        .iter()
        .map(|(n, h, a)| (n.to_string(), *h, *a))
        .collect()
}

fn timed(g: &G, edges: &[(&str, &str, &str, Valid)]) -> Vec<Eid> {
    let mut out = Vec::new();
    g.t.tx(|tx| {
        for (s, p, o, valid) in edges {
            out.push(tx.create(v(s), v(p), v(o), *valid)?);
        }
        Ok(())
    });
    out
}

// @lat: [[tests#Query#Time Respecting Scenarios]]
#[test]
fn time_respecting_scenarios() {
    // an infection chain: A met B in [1, 5), B met C in [3, 9)
    let g = G::new();
    timed(
        &g,
        &[
            ("A", "met", "B", Valid::between(1, 5)),
            ("B", "met", "C", Valid::between(3, 9)),
        ],
    );
    assert_eq!(
        journey(&g, "A", "met+", REACH, None),
        j(&[("B", 1, Some(1)), ("C", 2, Some(3))])
    );
    // the start instant cuts facts that ended before it, and raises the arrival
    assert!(journey(&g, "A", "met+", REACH, Some(6)).is_empty());
    assert_eq!(
        journey(&g, "A", "met+", REACH, Some(2)),
        j(&[("B", 1, Some(2)), ("C", 2, Some(3))])
    );
    assert_eq!(
        journey(&g, "A", "met+", REACH, Some(4)),
        j(&[("B", 1, Some(4)), ("C", 2, Some(4))])
    );
    // backwards in time: C met D before B met C
    timed(&g, &[("C", "met", "D", Valid::between(0, 2))]);
    assert!(!journey(&g, "A", "met+", REACH, None)
        .iter()
        .any(|(n, _, _)| n == "D"));
    // without the option, D is reachable and no row has an arrival
    let plain =
        g.t.db
            .now()
            .path(g.id("A"), "met+", REACH, u32::MAX)
            .unwrap();
    assert_eq!(plain.len(), 3);
    assert!(plain.iter().all(|r| r.arrival.is_none()));

    // same instant chains; the boundary is exclusive
    for (to, reachable) in [(2, true), (1, false)] {
        let g = G::new();
        timed(
            &g,
            &[
                ("A", "met", "B", Valid::between(1, 5)),
                ("B", "met", "C", Valid::between(0, to)),
            ],
        );
        let c = journey(&g, "A", "met+", REACH, None)
            .into_iter()
            .find(|(n, _, _)| n == "C");
        assert_eq!(c.is_some(), reachable, "B met C in [0, {to})");
        if let Some((_, _, a)) = c {
            assert_eq!(a, Some(1), "taken at instant 1");
        }
    }

    // unbounded intervals: no bound gives no arrival
    let g = G::new();
    timed(
        &g,
        &[
            ("A", "p", "B", Valid::ALWAYS),
            ("B", "p", "C", Valid::from(7)),
            ("C", "p", "D", Valid::until(20)),
        ],
    );
    assert_eq!(
        journey(&g, "A", "p+", REACH, None),
        j(&[("B", 1, None), ("C", 2, Some(7)), ("D", 3, Some(7))])
    );
    assert!(journey(&g, "A", "p*", REACH, Some(25))
        .iter()
        .all(|(n, _, _)| n != "D"));

    // a longer walk can arrive earlier: REACH keeps the shortest hops and the
    // earliest arrival, which come from different walks
    let g = G::new();
    timed(
        &g,
        &[
            ("A", "p", "C", Valid::between(10, 20)),
            ("A", "p", "B", Valid::between(1, 2)),
            ("B", "p", "C", Valid::between(1, 3)),
        ],
    );
    // (one layer: ordered by raw id, and C was interned before B)
    assert_eq!(
        sorted_vec(&journey(&g, "A", "p+", REACH, None)),
        j(&[("B", 1, Some(1)), ("C", 1, Some(1))])
    );
    // TRAIL: each trail with its own arrival
    let mut trails = journey(&g, "A", "p+", TRAIL, None);
    trails.sort();
    assert_eq!(
        trails,
        j(&[("B", 1, Some(1)), ("C", 1, Some(10)), ("C", 2, Some(1))])
    );
    // the shortest time-respecting path to C is the direct one
    assert_eq!(
        sorted_vec(&journey(&g, "A", "p+", ANY, None)),
        j(&[("B", 1, Some(1)), ("C", 1, Some(10))])
    );

    // a node reached again later with an earlier time is expanded again: X at
    // 1 hop is reached at 5, too late for `X p D`; at 2 hops it is reached at 1
    let g = G::new();
    timed(
        &g,
        &[
            ("A", "p", "X", Valid::between(5, 9)),
            ("A", "p", "B", Valid::between(1, 2)),
            ("B", "p", "X", Valid::between(1, 3)),
            ("X", "p", "D", Valid::between(0, 3)),
        ],
    );
    for mode in [REACH, TRAIL, ANY, ALL] {
        let d: Vec<_> = journey(&g, "A", "p+", mode, None)
            .into_iter()
            .filter(|(n, _, _)| n == "D")
            .collect();
        assert_eq!(d, j(&[("D", 3, Some(1))]), "{mode:?}");
    }

    // shortest paths skip a shorter walk that goes back in time
    let g = G::new();
    timed(
        &g,
        &[
            ("A", "p", "B", Valid::between(5, 9)),
            ("B", "p", "D", Valid::between(0, 3)),
            ("A", "p", "C", Valid::between(1, 2)),
            ("C", "p", "E", Valid::between(1, 4)),
            ("E", "p", "D", Valid::between(2, 6)),
        ],
    );
    let d = |mode| {
        journey(&g, "A", "p+", mode, None)
            .into_iter()
            .filter(|(n, _, _)| n == "D")
            .collect::<Vec<_>>()
    };
    assert_eq!(d(ALL), j(&[("D", 3, Some(2))]));
    assert_eq!(d(ANY), j(&[("D", 3, Some(2))]));
    let paths =
        g.t.db
            .now()
            .path_with(
                g.id("A"),
                "p+",
                &PathArgs {
                    mode: ALL,
                    time_respecting: Some(TimeRespecting::default()),
                    ..PathArgs::default()
                },
            )
            .unwrap();
    let to_d = paths.iter().find(|r| g.name(r.end) == "D").unwrap();
    let names: Vec<String> = to_d
        .path
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .map(|n| g.name(*n))
        .collect();
    assert_eq!(names, ["A", "C", "E", "D"]);
}

// @lat: [[tests#Query#Time Respecting Combines With Graphs And Layers]]
#[test]
fn time_respecting_combines_with_graphs_and_layers() {
    // virtual hops are structural: the supporting fact ended before the belief
    let g = G::new();
    g.t.tx(|tx| {
        let e1 = tx
            .assert(v("alice"), v("worksAt"), v("acme"), Valid::between(1, 2))?
            .eid();
        let e7 = tx
            .assert(v("belief9"), v("supportedBy"), e1, Valid::between(5, 9))?
            .eid();
        tx.add_to_graph(e7, Value::iri("urn:g:1"), AssertOpts::default())?;
        tx.add_to_graph(e1, Value::iri("urn:g:1"), AssertOpts::default())?;
        Ok(())
    });
    let mut got = journey(
        &g,
        "belief9",
        "supportedBy/(sys:subject|sys:object)",
        REACH,
        None,
    );
    got.sort();
    assert_eq!(got, j(&[("acme", 2, Some(5)), ("alice", 2, Some(5))]));
    // …but a stored hop after it still needs the fact to hold
    assert!(journey(
        &g,
        "belief9",
        "supportedBy/sys:subject/worksAt",
        REACH,
        None
    )
    .is_empty());
    assert!(journey(&g, "alice", "worksAt", REACH, Some(3)).is_empty());
    // with a graph set: the graph filter and the time rule both apply
    let view = g.t.db.now();
    let g1 = view.encode(&Value::iri("urn:g:1")).unwrap().unwrap();
    let args = |graphs: Vec<ObjectId>| PathArgs {
        graphs: Some(graphs),
        time_respecting: Some(TimeRespecting::default()),
        ..PathArgs::default()
    };
    let rows = view
        .path_with(g.id("belief9"), "supportedBy/sys:subject", &args(vec![g1]))
        .unwrap();
    assert_eq!(
        rows.iter()
            .map(|r| (g.name(r.end), r.arrival))
            .collect::<Vec<_>>(),
        [("alice".to_string(), Some(5))]
    );
    let none = view
        .path_with(g.id("belief9"), "supportedBy/sys:subject", &args(vec![]))
        .unwrap();
    assert!(none.is_empty());
    // the view's validAt still filters every hop on its own
    let at = g.t.db.now().valid_at(6);
    let rows = at
        .path_with(
            g.id("belief9"),
            "supportedBy/sys:subject",
            &PathArgs {
                time_respecting: Some(TimeRespecting::default()),
                ..PathArgs::default()
            },
        )
        .unwrap();
    assert!(
        rows.is_empty(),
        "e1 is not valid at 6, so its row is not visible"
    );
}

fn q(g: &G, sql: &str) -> Vec<Vec<SqlValue>> {
    g.t.db
        .read_sql(sql)
        .unwrap_or_else(|e| panic!("{sql}: {e}"))
}

// @lat: [[tests#Query#tm_path Arrival Column]]
#[test]
fn tm_path_arrival_column() {
    let g = G::new();
    timed(
        &g,
        &[
            ("a", "p", "b", Valid::between(1, 5)),
            ("b", "p", "c", Valid::between(3, 9)),
        ],
    );
    let a = g.id("a").raw();
    let rows = |view: &str| -> Vec<(String, Option<i64>)> {
        q(
            &g,
            &format!("SELECT \"end\", arrival FROM tm_path({a}, 'p+', 'REACH', NULL, '{view}')"),
        )
        .iter()
        .map(|r| {
            (
                g.name(ObjectId::from_raw(r[0].as_i64().unwrap())),
                r[1].as_i64(),
            )
        })
        .collect()
    };
    let named = |xs: &[(&str, Option<i64>)]| -> Vec<(String, Option<i64>)> {
        xs.iter().map(|(n, a)| (n.to_string(), *a)).collect()
    };
    assert_eq!(
        rows("timeRespecting"),
        named(&[("b", Some(1)), ("c", Some(3))])
    );
    assert_eq!(rows("now"), named(&[("b", None), ("c", None)]));
    assert_eq!(
        rows("now;timeRespecting/2"),
        named(&[("b", Some(2)), ("c", Some(3))])
    );
    assert!(rows("timeRespecting/6").is_empty());
    assert!(rows("urn:tiramemsu:tm:timeRespecting/1970-01-01T00:00:00.006Z").is_empty());
    // the same rows as the API
    let api =
        g.t.db
            .now()
            .path_with(
                g.id("a"),
                "p+",
                &PathArgs {
                    time_respecting: Some(TimeRespecting { after: Some(2) }),
                    ..PathArgs::default()
                },
            )
            .unwrap();
    assert_eq!(
        api.iter()
            .map(|r| (g.name(r.end), r.arrival))
            .collect::<Vec<_>>(),
        rows("timeRespecting/2")
    );
    // TRAIL rows too
    let trail = q(
        &g,
        &format!("SELECT hops, arrival FROM tm_path({a}, 'p+', 'TRAIL', NULL, 'timeRespecting')"),
    );
    assert_eq!(
        trail
            .iter()
            .map(|r| (r[0].as_i64(), r[1].as_i64()))
            .collect::<Vec<_>>(),
        [(Some(1), Some(1)), (Some(2), Some(3))]
    );
    for bad in ["timeRespecting/soon", "timeRespecting;timeRespecting"] {
        match g.t.db.read_sql(&format!(
            "SELECT * FROM tm_path({a}, 'p+', 'REACH', NULL, '{bad}')"
        )) {
            Err(Error::Sqlite(e)) => assert!(e.message.starts_with("tm_path: view:"), "{e:?}"),
            other => panic!("{bad}: {other:?}"),
        }
    }
}
