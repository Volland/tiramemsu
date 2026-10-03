//! `add-temporal-path-syntax`: the SPARQL scope `SERVICE
//! <urn:tiramemsu:tm:timeRespecting…>` with `tm:arrival`, the Cypher `MATCH TIME
//! RESPECTING [AFTER t] [ARRIVAL AS x]`, and path completeness reporting
//! (`lat.md/query#Temporal Path Syntax`).
#![cfg(all(feature = "sparql", feature = "cypher"))]
mod cypher_common;

use std::collections::BTreeMap;

use cypher_common::*;
use tiramemsu::*;

fn local(v: &Value) -> String {
    match v {
        Value::Iri(i) => i.trim_start_matches("urn:tiramemsu:v:").to_string(),
        other => format!("{other:?}"),
    }
}

/// Asserts `(a met b)` facts with their valid intervals; every node gets an `id`.
fn contacts(t: &T, facts: &[(&str, &str, Option<i64>, Option<i64>)]) {
    t.tx(|tx| {
        let mut seen = Vec::new();
        for (a, b, from, to) in facts {
            for n in [a, b] {
                if !seen.contains(n) {
                    seen.push(*n);
                    tx.assert(v(n), v("id"), sv(n), Valid::ALWAYS)?;
                }
            }
            tx.assert(
                v(a),
                v("met"),
                v(b),
                Valid {
                    from: *from,
                    to: *to,
                },
            )?;
        }
        Ok(())
    });
}

/// SPARQL `SELECT ?y ?t`: end -> arrival (`None` = unbound).
fn sparql_arrivals(view: &View<'_>, q: &str, params: Params) -> BTreeMap<String, Option<i64>> {
    let opts = SparqlOptions {
        params,
        ..Default::default()
    };
    let r = view.sparql_with(q, &opts).unwrap();
    let s = r.solutions().unwrap();
    s.rows
        .iter()
        .map(|row| {
            let t = match &row[1] {
                None => None,
                Some(Value::Int(n)) => Some(*n),
                other => panic!("arrival {other:?}"),
            };
            (local(row[0].as_ref().unwrap()), t)
        })
        .collect()
}

/// Cypher `RETURN y.id, t` rows reduced to the earliest arrival per end.
fn cypher_arrivals(view: &View<'_>, q: &str, p: &CypherParams) -> BTreeMap<String, Option<i64>> {
    let r = view.cypher(q, p).unwrap();
    let mut out: BTreeMap<String, Option<i64>> = BTreeMap::new();
    for row in &r.rows {
        let CypherValue::String(y) = &row[0] else {
            panic!("{row:?}")
        };
        let t = match &row[1] {
            CypherValue::Null => None,
            CypherValue::Integer(n) => Some(*n),
            other => panic!("arrival {other:?}"),
        };
        out.entry(y.clone())
            .and_modify(|old| *old = (*old).min(t))
            .or_insert(t);
    }
    out
}

/// The Rust API: `REACH` with time respect from `after`.
fn api_arrivals(
    view: &View<'_>,
    start: &str,
    path: &str,
    after: Option<i64>,
) -> BTreeMap<String, Option<i64>> {
    let s = view.encode(&v(start)).unwrap().unwrap();
    let args = PathArgs {
        time_respecting: Some(TimeRespecting { after }),
        ..PathArgs::default()
    };
    view.path_with(s, path, &args)
        .unwrap()
        .iter()
        .map(|r| (local(&view.decode(r.end).unwrap()), r.arrival))
        .collect()
}

const TR: &str = "urn:tiramemsu:tm:timeRespecting";

// "Reverse valid-time order"
// @lat: [[tests#Temporal Path Syntax#Reverse Order Does Not Match]]
#[test]
fn reverse_valid_time_order_does_not_match() {
    let t = T::new();
    // x met y in [5, 10), then y met z in [1, 3): z cannot be reached in time order
    contacts(
        &t,
        &[("x", "y", Some(5), Some(10)), ("y", "z", Some(1), Some(3))],
    );
    let view = t.db.now();
    // an ordinary path matches in both languages
    let plain = view.sparql("ASK { v:x v:met+ v:z }").unwrap();
    assert_eq!(plain, SparqlResult::Boolean(true));
    assert_eq!(
        view.cypher(
            "MATCH (a {id:'x'})-[:met*]->(b {id:'z'}) RETURN count(*) AS n",
            &no_params()
        )
        .unwrap()
        .rows,
        vec![vec![i(1)]]
    );
    // the temporal one does not
    let q = format!("ASK {{ SERVICE <{TR}> {{ v:x v:met+ v:z }} }}");
    assert_eq!(view.sparql(&q).unwrap(), SparqlResult::Boolean(false));
    let ends = sparql_arrivals(
        &view,
        &format!("SELECT ?y ?t WHERE {{ SERVICE <{TR}> {{ v:x v:met+ ?y . ?y tm:arrival ?t }} }}"),
        Params::new(),
    );
    assert_eq!(ends, BTreeMap::from([("y".to_string(), Some(5))]));
    assert_eq!(
        view.cypher(
            "MATCH TIME RESPECTING (a {id:'x'})-[:met*]->(b {id:'z'}) RETURN count(*) AS n",
            &no_params()
        )
        .unwrap()
        .rows,
        vec![vec![i(0)]]
    );
    // an earlier chain order would: y met z in [6, 8) after x met y in [5, 10)
    t.tx(|tx| {
        tx.assert(v("y"), v("met"), v("z"), Valid::between(6, 8))?;
        Ok(())
    });
    let view = t.db.now();
    assert_eq!(view.sparql(&q).unwrap(), SparqlResult::Boolean(true));
}

// "Virtual layer hop"
// @lat: [[tests#Temporal Path Syntax#Virtual Hop Keeps The Arrival]]
#[test]
fn virtual_layer_hop_keeps_the_arrival() {
    let t = T::new();
    let mut e1 = None;
    t.tx(|tx| {
        for n in ["r", "u", "w"] {
            tx.assert(v(n), v("id"), sv(n), Valid::ALWAYS)?;
        }
        // u met w in [2, 9); r is supported by that statement from 7 on
        let e = tx.assert(v("u"), v("met"), v("w"), Valid::between(2, 9))?;
        tx.assert(
            v("r"),
            v("supportedBy"),
            Value::Stmt(e.eid()),
            Valid::from(7),
        )?;
        e1 = Some(e.eid());
        Ok(())
    });
    let view = t.db.now();
    // r -supportedBy-> e1 at 7, sys:subject to u keeps 7, met (still valid at 7) to w
    let q = format!(
        "SELECT ?y ?t WHERE {{ SERVICE <{TR}> {{ v:r (v:supportedBy/sys:subject/v:met)+ ?y . \
         ?y tm:arrival ?t }} }}"
    );
    let sparql = sparql_arrivals(&view, &q, Params::new());
    assert_eq!(sparql, BTreeMap::from([("w".to_string(), Some(7))]));
    let api = api_arrivals(&view, "r", "(supportedBy/sys:subject/met)+", None);
    assert_eq!(sparql, api);
    // the arrival at the subject itself: still 7, not the statement's 2
    let q = format!(
        "SELECT ?y ?t WHERE {{ SERVICE <{TR}> {{ v:r (v:supportedBy/sys:subject)+ ?y . \
         ?y tm:arrival ?t }} }}"
    );
    assert_eq!(
        sparql_arrivals(&view, &q, Params::new()),
        BTreeMap::from([("u".to_string(), Some(7))])
    );
    let cy = cypher_arrivals(
        &view,
        "MATCH TIME RESPECTING ARRIVAL AS t (a {id:'r'})-[:supportedBy|`sys:subject`*2]->(y) \
         RETURN y.id AS y, t",
        &no_params(),
    );
    assert_eq!(cy, BTreeMap::from([("u".to_string(), Some(7))]));
    let _ = e1;
}

// "Multiple journeys"
// @lat: [[tests#Temporal Path Syntax#Earliest Arrival Over Journeys]]
#[test]
fn earliest_arrival_over_several_journeys() {
    let t = T::new();
    // a reaches d directly at 10, or through e at 1 (a longer walk arriving earlier)
    contacts(
        &t,
        &[
            ("a", "d", Some(10), Some(20)),
            ("a", "e", Some(1), Some(2)),
            ("e", "d", Some(1), Some(3)),
        ],
    );
    let view = t.db.now();
    let q =
        format!("SELECT ?y ?t WHERE {{ SERVICE <{TR}> {{ v:a v:met+ ?y . ?y tm:arrival ?t }} }}");
    let sparql = sparql_arrivals(&view, &q, Params::new());
    let want = BTreeMap::from([("d".to_string(), Some(1)), ("e".to_string(), Some(1))]);
    assert_eq!(sparql, want);
    assert_eq!(api_arrivals(&view, "a", "met+", None), want);
    // Cypher trails carry their own arrival: d is reached at 10 and at 1
    let r = view
        .cypher(
            "MATCH TIME RESPECTING ARRIVAL AS t (a {id:'a'})-[:met*]->(y {id:'d'}) \
             RETURN t ORDER BY t",
            &no_params(),
        )
        .unwrap();
    assert_eq!(r.rows, vec![vec![i(1)], vec![i(10)]]);
    let cy = cypher_arrivals(
        &view,
        "MATCH TIME RESPECTING ARRIVAL AS t (a {id:'a'})-[:met*]->(y) RETURN y.id AS y, t",
        &no_params(),
    );
    assert_eq!(cy, want);
    // shortestPath binds the arrival of the minimal-length journey
    let r = view
        .cypher(
            "MATCH TIME RESPECTING ARRIVAL AS t p = shortestPath((a {id:'a'})-[:met*]->(y {id:'d'})) \
             RETURN length(p) AS n, t",
            &no_params(),
        )
        .unwrap();
    assert_eq!(r.rows, vec![vec![i(1), i(10)]]);
}

// "Parameterized start"
// @lat: [[tests#Temporal Path Syntax#Parameterized Start Matches The API]]
#[test]
fn parameterized_start_matches_the_rust_api() {
    let t = T::new();
    contacts(
        &t,
        &[
            ("a", "b", Some(1), Some(5)),
            ("b", "c", Some(3), Some(9)),
            ("c", "d", Some(0), Some(4)),
            ("c", "e", Some(8), None),
            ("a", "f", None, Some(2)),
            ("f", "g", None, None),
            ("b", "a", Some(4), Some(6)),
        ],
    );
    let view = t.db.now();
    let sparql_q = format!(
        "SELECT ?y ?t WHERE {{ SERVICE <{TR}/$start> {{ v:a v:met+ ?y . ?y tm:arrival ?t }} }}"
    );
    let cypher_q = "MATCH TIME RESPECTING AFTER $start ARRIVAL AS t (x {id:'a'})-[:met*]->(y) \
                    RETURN y.id AS y, t";
    for start in [-10, 0, 1, 2, 3, 4, 5, 6, 8, 9, 100] {
        let api = api_arrivals(&view, "a", "met+", Some(start));
        let sparql = sparql_arrivals(&view, &sparql_q, params_of(start));
        let cypher = cypher_arrivals(&view, cypher_q, &params(&[("start", i(start))]));
        assert_eq!(sparql, api, "SPARQL from {start}");
        assert_eq!(cypher, api, "Cypher from {start}");
        // literal starts mean the same
        let lit = sparql_arrivals(
            &view,
            &format!(
                "SELECT ?y ?t WHERE {{ SERVICE <{TR}/{start}> {{ v:a v:met+ ?y . ?y tm:arrival ?t }} }}"
            ),
            Params::new(),
        );
        assert_eq!(lit, api);
        let lit = cypher_arrivals(
            &view,
            &format!(
                "MATCH TIME RESPECTING AFTER {start} ARRIVAL AS t (x {{id:'a'}})-[:met*]->(y) \
                 RETURN y.id AS y, t"
            ),
            &no_params(),
        );
        assert_eq!(lit, api);
    }
    // a date start: 1970-01-01 is instant 0
    let api = api_arrivals(&view, "a", "met+", Some(0));
    assert_eq!(
        sparql_arrivals(
            &view,
            &format!("SELECT ?y ?t WHERE {{ SERVICE <{TR}/1970-01-01> {{ v:a v:met+ ?y . ?y tm:arrival ?t }} }}"),
            Params::new()
        ),
        api
    );
    assert_eq!(
        cypher_arrivals(
            &view,
            "MATCH TIME RESPECTING AFTER date('1970-01-01') ARRIVAL AS t (x {id:'a'})-[:met*]->(y) \
             RETURN y.id AS y, t",
            &no_params()
        ),
        api
    );
    // a missing parameter fails
    let opts = SparqlOptions::default();
    assert!(view.sparql_with(&sparql_q, &opts).is_err());
    assert!(view.cypher(cypher_q, &no_params()).is_err());
}

fn params_of(start: i64) -> Params {
    tiramemsu::ir::params([("start", Value::Int(start))])
}

// "Earliest arrival binding" with an unbounded initial instant
// @lat: [[tests#Temporal Path Syntax#Unbounded Arrival Is Unbound]]
#[test]
fn unbounded_arrival_is_unbound() {
    let t = T::new();
    contacts(&t, &[("a", "b", None, None), ("b", "c", None, Some(5))]);
    let view = t.db.now();
    let want = BTreeMap::from([("b".to_string(), None), ("c".to_string(), None)]);
    assert_eq!(api_arrivals(&view, "a", "met+", None), want);
    let q =
        format!("SELECT ?y ?t WHERE {{ SERVICE <{TR}> {{ v:a v:met+ ?y . ?y tm:arrival ?t }} }}");
    assert_eq!(sparql_arrivals(&view, &q, Params::new()), want);
    let cy = cypher_arrivals(
        &view,
        "MATCH TIME RESPECTING ARRIVAL AS t (x {id:'a'})-[:met*]->(y) RETURN y.id AS y, t",
        &no_params(),
    );
    assert_eq!(cy, want);
    // a zero-length match arrives at the start instant
    let q = format!(
        "SELECT ?y ?t WHERE {{ SERVICE <{TR}/42> {{ v:a v:met* ?y . ?y tm:arrival ?t }} }}"
    );
    let got = sparql_arrivals(&view, &q, Params::new());
    assert_eq!(got.get("a"), Some(&Some(42)));
    // so does one from a constant that is in no statement
    let q = format!(
        "SELECT ?y ?t WHERE {{ SERVICE <{TR}/42> {{ v:nobody v:met* ?y . ?y tm:arrival ?t }} }}"
    );
    assert_eq!(
        sparql_arrivals(&view, &q, Params::new()),
        BTreeMap::from([("nobody".to_string(), Some(42))])
    );
}

// Shared causal journey semantics: the selected transaction and graph view
// @lat: [[tests#Temporal Path Syntax#Uses The Graph Scope And Snapshot]]
#[test]
fn uses_the_graph_scope_and_snapshot() {
    let t = T::new();
    let mut ab = None;
    t.tx(|tx| {
        for n in ["a", "b", "c"] {
            tx.assert(v(n), v("id"), sv(n), Valid::ALWAYS)?;
        }
        let e = tx.assert(v("a"), v("met"), v("b"), Valid::between(1, 5))?;
        tx.assert(v("b"), v("met"), v("c"), Valid::between(3, 9))?;
        tx.add_to_graph(e.eid(), v("g1"), AssertOpts::default())?;
        ab = Some(e.eid());
        Ok(())
    });
    let before = t.last_t();
    t.tx(|tx| {
        tx.retract(ab.unwrap())?;
        Ok(())
    });
    let view = t.db.now();
    let q = |inner: &str| format!("SELECT ?y ?t WHERE {{ {inner} }}");
    let tr = |body: &str| format!("SERVICE <{TR}> {{ {body} }}");
    let path = "v:a v:met+ ?y . ?y tm:arrival ?t";
    // retracted now: nothing
    assert!(sparql_arrivals(&view, &q(&tr(path)), Params::new()).is_empty());
    // the snapshot before the retraction: both, in the scope's view
    let at = format!(
        "SERVICE <urn:tiramemsu:tm:asOf/{before}> {{ {} }}",
        tr(path)
    );
    let want = BTreeMap::from([("b".to_string(), Some(1)), ("c".to_string(), Some(3))]);
    assert_eq!(sparql_arrivals(&view, &q(&at), Params::new()), want);
    let inner_asof = tr(&format!(
        "SERVICE <urn:tiramemsu:tm:asOf/{before}> {{ {path} }}"
    ));
    assert_eq!(sparql_arrivals(&view, &q(&inner_asof), Params::new()), want);
    let old = t.db.as_of(TimeRef::Tx(before));
    assert_eq!(api_arrivals(&old, "a", "met+", None), want);
    let cy = format!(
        "USE AS OF {before} MATCH TIME RESPECTING ARRIVAL AS t (x {{id:'a'}})-[:met*]->(y) \
         RETURN y.id AS y, t"
    );
    assert_eq!(cypher_arrivals(&view, &cy, &no_params()), want);
    // in graph g1 only `a met b` is a member
    let g = format!(
        "SERVICE <urn:tiramemsu:tm:asOf/{before}> {{ GRAPH v:g1 {{ {} }} }}",
        tr(path)
    );
    assert_eq!(
        sparql_arrivals(&view, &q(&g), Params::new()),
        BTreeMap::from([("b".to_string(), Some(1))])
    );
    let g_inside = format!(
        "SERVICE <urn:tiramemsu:tm:asOf/{before}> {{ {} }}",
        tr(&format!("GRAPH v:g1 {{ {path} }}"))
    );
    assert_eq!(
        sparql_arrivals(&view, &q(&g_inside), Params::new()),
        BTreeMap::from([("b".to_string(), Some(1))])
    );
}

// Ordinary row shapes are unchanged unless the arrival is requested
// @lat: [[tests#Temporal Path Syntax#Row Shapes Are Unchanged]]
#[test]
fn row_shapes_are_unchanged() {
    let t = T::new();
    contacts(
        &t,
        &[("a", "b", Some(1), Some(5)), ("b", "c", Some(3), Some(9))],
    );
    let view = t.db.now();
    let r = view
        .sparql(&format!(
            "SELECT * WHERE {{ SERVICE <{TR}> {{ v:a v:met+ ?y }} }}"
        ))
        .unwrap();
    let sol = r.solutions().unwrap();
    assert_eq!(sol.vars, ["y"]);
    assert_eq!(sol.rows.len(), 2);
    let r = view
        .cypher(
            "MATCH TIME RESPECTING p = (x {id:'a'})-[r:met*]->(y) RETURN y.id AS y, length(p) AS n \
             ORDER BY n",
            &no_params(),
        )
        .unwrap();
    assert_eq!(r.columns, ["y", "n"]);
    assert_eq!(r.rows, vec![vec![s("b"), i(1)], vec![s("c"), i(2)]]);
    // the same text without the modifier keeps its rows
    let plain = view
        .cypher(
            "MATCH p = (x {id:'a'})-[r:met*]->(y) RETURN y.id AS y, length(p) AS n ORDER BY n",
            &no_params(),
        )
        .unwrap();
    assert_eq!(plain.rows, r.rows);
    // the tm_path call is unchanged for an ordinary query: four columns read, arrival NULL
    let a = view.encode(&v("a")).unwrap().unwrap();
    let rows =
        t.db.read_sql(&format!(
            "SELECT \"end\", hops, arrival FROM tm_path({}, 'met+', 'REACH')",
            a.raw()
        ))
        .unwrap();
    assert!(rows.iter().all(|r| r[2] == tm_core_null()));
}

fn tm_core_null() -> tiramemsu::SqlValue {
    tiramemsu::SqlValue::Null
}

// "Configured cap": an explicit bound against the configured hop cap
// @lat: [[tests#Temporal Path Syntax#Completeness Tells Bound From Cap]]
#[test]
fn completeness_tells_an_explicit_bound_from_the_cap() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(
        dir.path().join("c.db"),
        OpenOptions {
            path_max_hops: 3,
            ..OpenOptions::default()
        },
    )
    .unwrap();
    // a chain n0 -> n1 -> ... -> n6
    db.transact(TxOptions::default(), |tx| {
        for k in 0..7 {
            tx.assert(
                v(&format!("n{k}")),
                v("id"),
                sv(&format!("n{k}")),
                Valid::ALWAYS,
            )?;
        }
        for k in 0..6 {
            tx.assert(
                v(&format!("n{k}")),
                v("next"),
                v(&format!("n{}", k + 1)),
                Valid::between(k, 100),
            )?;
        }
        Ok(())
    })
    .unwrap();
    let view = db.now();
    let pc = |q: &str| view.cypher(q, &no_params()).unwrap().path_completeness;
    // an unbounded trail stopped by the cap of 3: not exhaustive
    let capped = pc("MATCH (a {id:'n0'})-[:next*]->(b) RETURN b.id");
    assert_eq!(capped, Some(PathCompleteness::StoppedAtCap { max_hops: 3 }));
    assert!(!capped.unwrap().is_complete());
    // also when time-respecting
    assert_eq!(
        pc("MATCH TIME RESPECTING (a {id:'n0'})-[:next*]->(b) RETURN b.id"),
        Some(PathCompleteness::StoppedAtCap { max_hops: 3 })
    );
    // an explicit bound the query wrote is part of the expression: `*1..2` was
    // evaluated exhaustively, nothing longer was asked for
    let bound = pc("MATCH (a {id:'n0'})-[:next*1..2]->(b) RETURN b.id");
    assert_eq!(bound, Some(PathCompleteness::Exhaustive));
    assert!(bound.unwrap().is_complete());
    // a search that runs out of states before the cap is exhaustive
    assert_eq!(
        pc("MATCH (a {id:'n4'})-[:next*]->(b) RETURN b.id"),
        Some(PathCompleteness::Exhaustive)
    );
    // shortestPath under the cap
    assert_eq!(
        pc("MATCH p = shortestPath((a {id:'n0'})-[:next*]->(b {id:'n6'})) RETURN length(p)"),
        Some(PathCompleteness::StoppedAtCap { max_hops: 3 })
    );
    // no path region: no verdict
    assert_eq!(pc("MATCH (a {id:'n0'})-[:next]->(b) RETURN b.id"), None);
    // SPARQL paths are uncapped: exhaustive, also with time respect
    let sol = |q: &str| {
        view.sparql(q)
            .unwrap()
            .solutions()
            .unwrap()
            .path_completeness
    };
    assert_eq!(
        sol("SELECT ?b WHERE { v:n0 v:next+ ?b }"),
        Some(PathCompleteness::Exhaustive)
    );
    assert_eq!(
        sol(&format!(
            "SELECT ?b WHERE {{ SERVICE <{TR}> {{ v:n0 v:next+ ?b }} }}"
        )),
        Some(PathCompleteness::Exhaustive)
    );
    assert_eq!(sol("SELECT ?b WHERE { v:n0 v:next ?b }"), None);
    // the Rust API: explicit bound, opt-in cap, and the default
    let n0 = view.encode(&v("n0")).unwrap().unwrap();
    let trail = |max_hops, capped| PathArgs {
        mode: PathMode::Trail,
        max_hops,
        capped,
        ..PathArgs::default()
    };
    let r = view
        .path_report(n0, "next+", &trail(u32::MAX, true))
        .unwrap();
    assert_eq!(
        r.completeness,
        PathCompleteness::StoppedAtCap { max_hops: 3 }
    );
    assert_eq!(r.rows.len(), 3);
    let r = view.path_report(n0, "next+", &trail(2, false)).unwrap();
    assert_eq!(
        r.completeness,
        PathCompleteness::StoppedAtBound { max_hops: 2 }
    );
    let r = view.path_report(n0, "next+", &trail(2, true)).unwrap(); // `capped` ignored
    assert_eq!(
        r.completeness,
        PathCompleteness::StoppedAtBound { max_hops: 2 }
    );
    let r = view
        .path_report(n0, "next+", &trail(u32::MAX, false))
        .unwrap();
    assert_eq!(r.completeness, PathCompleteness::Exhaustive);
    assert_eq!(r.rows.len(), 6);
    // a bound exactly at the end of the chain leaves nothing to expand
    let r = view.path_report(n0, "next+", &trail(6, false)).unwrap();
    assert_eq!(r.completeness, PathCompleteness::Exhaustive);
}

// "State guard"
// @lat: [[tests#Temporal Path Syntax#State Guard Still Fails]]
#[test]
fn state_guard_still_fails() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(
        dir.path().join("c.db"),
        OpenOptions {
            path_max_states: 5,
            ..OpenOptions::default()
        },
    )
    .unwrap();
    db.transact(TxOptions::default(), |tx| {
        for k in 0..20 {
            tx.assert(
                v(&format!("n{k}")),
                v("id"),
                sv(&format!("n{k}")),
                Valid::ALWAYS,
            )?;
            tx.assert(
                v(&format!("n{k}")),
                v("next"),
                v(&format!("n{}", k + 1)),
                Valid::between(k, 100),
            )?;
        }
        Ok(())
    })
    .unwrap();
    let view = db.now();
    let limit = |r: Result<()>| matches!(r, Err(Error::PathLimitExceeded { limit: 5 }));
    assert!(limit(
        view.sparql(&format!(
            "SELECT ?b ?t WHERE {{ SERVICE <{TR}> {{ v:n0 v:next+ ?b . ?b tm:arrival ?t }} }}"
        ))
        .map(|_| ())
    ));
    assert!(limit(
        view.cypher(
            "MATCH TIME RESPECTING ARRIVAL AS t (a {id:'n0'})-[:next*]->(b) RETURN b.id, t",
            &no_params()
        )
        .map(|_| ())
    ));
    let n0 = view.encode(&v("n0")).unwrap().unwrap();
    let args = PathArgs {
        time_respecting: Some(TimeRespecting::default()),
        ..PathArgs::default()
    };
    assert!(limit(view.path_report(n0, "next+", &args).map(|_| ())));
}

// Grammar errors of both extensions
// @lat: [[tests#Temporal Path Syntax#Grammar Errors]]
#[test]
fn grammar_errors() {
    let t = T::new();
    contacts(
        &t,
        &[("a", "b", Some(1), Some(5)), ("b", "c", Some(3), Some(9))],
    );
    let view = t.db.now();
    let parse = |q: &str| matches!(view.sparql(q), Err(Error::Parse { .. }));
    // tm:arrival outside a time-respecting scope
    assert!(parse(
        "SELECT ?y ?t WHERE { v:a v:met+ ?y . ?y tm:arrival ?t }"
    ));
    // tm:arrival on a node that ends no temporal path of the group
    assert!(parse(&format!(
        "SELECT ?y ?t WHERE {{ SERVICE <{TR}> {{ v:a v:met+ ?y . v:a tm:arrival ?t }} }}"
    )));
    // tm:arrival with a constant object
    assert!(parse(&format!(
        "SELECT ?y WHERE {{ SERVICE <{TR}> {{ v:a v:met+ ?y . ?y tm:arrival 5 }} }}"
    )));
    // a scope with no path
    assert!(parse(&format!(
        "SELECT ?y WHERE {{ SERVICE <{TR}> {{ v:a v:met ?y }} }}"
    )));
    // malformed start, and the modifier in FROM
    assert!(parse(&format!(
        "SELECT ?y WHERE {{ SERVICE <{TR}/soon> {{ v:a v:met+ ?y }} }}"
    )));
    assert!(parse(&format!(
        "SELECT ?y FROM <{TR}> WHERE {{ v:a v:met+ ?y }}"
    )));
    // a journey is not run backwards from its end
    assert!(matches!(
        view.sparql(&format!(
            "SELECT ?x WHERE {{ SERVICE <{TR}> {{ ?x v:met+ v:c }} }}"
        )),
        Err(Error::Unsupported { .. })
    ));
    let cy = |q: &str| view.cypher(q, &no_params());
    assert!(matches!(
        cy("MATCH TIME RESPECTING (a {id:'a'})-[:met]->(b) RETURN b"),
        Err(Error::Parse { .. })
    ));
    assert!(matches!(
        cy("MATCH TIME RESPECTING ARRIVAL AS t (a {id:'a'})-[:met*]->(b)-[:met*]->(c) RETURN c"),
        Err(Error::Parse { .. })
    ));
    assert!(matches!(
        cy("MATCH (t {id:'a'}) MATCH TIME RESPECTING ARRIVAL AS t (a {id:'a'})-[:met*]->(b) RETURN b"),
        Err(Error::Parse { .. })
    ));
    assert!(matches!(
        cy("MATCH TIME RESPECTING ARRIVAL (a {id:'a'})-[:met*]->(b) RETURN b"),
        Err(Error::Parse { .. })
    ));
    assert!(matches!(
        cy("MATCH TIME RESPECTING AFTER 1.5 (a {id:'a'})-[:met*]->(b) RETURN b"),
        Err(Error::Parse { .. })
    ));
    assert!(matches!(
        cy("MATCH TIME RESPECTING (b)<-[:met*]-(a {id:'c'}) RETURN b"),
        Err(Error::Unsupported { .. })
    ));
    // OPTIONAL MATCH keeps unmatched rows with a null arrival
    let r = cy(
        "MATCH (a {id:'c'}) OPTIONAL MATCH TIME RESPECTING ARRIVAL AS t (a)-[:met*]->(b) \
         RETURN b, t",
    )
    .unwrap();
    assert_eq!(r.rows, vec![vec![null(), null()]]);
}
