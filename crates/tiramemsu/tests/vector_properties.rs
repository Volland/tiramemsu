//! Properties and fixtures of vector and hybrid recall: exact search equals brute
//! force, the prefilter returns exact similarities, fusion recomputes from the
//! standalone channels, views and graphs apply to fused and expanded statements,
//! and channel failures are typed.
#![cfg(all(feature = "sparql", feature = "cypher"))]

use std::collections::HashMap;
use std::path::Path;

use proptest::prelude::*;
use tiramemsu::vector::{f16_to_f32, f32_to_f16};
use tiramemsu::*;

fn v(s: &str) -> Value {
    Value::iri(format!("urn:tiramemsu:v:{s}"))
}

fn model() -> Value {
    v("prop-model")
}

/// The vector the store keeps for `x`: unit length for cosine, then rounded
/// through the encoding.
fn stored(x: &[f32], metric: Metric, enc: VectorEncoding) -> Vec<f64> {
    let mut y: Vec<f32> = x.to_vec();
    if metric == Metric::Cosine {
        let n = x.iter().map(|a| f64::from(*a).powi(2)).sum::<f64>().sqrt();
        y = x.iter().map(|a| (f64::from(*a) / n) as f32).collect();
    }
    if enc == VectorEncoding::F16 {
        y = y.into_iter().map(|a| f16_to_f32(f32_to_f16(a))).collect();
    }
    y.into_iter().map(f64::from).collect()
}

fn brute(metric: Metric, q: &[f64], x: &[f64]) -> f64 {
    match metric {
        Metric::Cosine | Metric::Dot => q.iter().zip(x).map(|(a, b)| a * b).sum(),
        Metric::L2 => -q
            .iter()
            .zip(x)
            .map(|(a, b)| (a - b).powi(2))
            .sum::<f64>()
            .sqrt(),
    }
}

fn metric_strategy() -> impl Strategy<Value = Metric> {
    prop_oneof![Just(Metric::Cosine), Just(Metric::Dot), Just(Metric::L2)]
}

fn encoding_strategy() -> impl Strategy<Value = VectorEncoding> {
    prop_oneof![Just(VectorEncoding::F32), Just(VectorEncoding::F16)]
}

/// Components away from zero so cosine never meets a zero vector.
fn vector(dims: usize) -> impl Strategy<Value = Vec<f32>> {
    prop::collection::vec(prop_oneof![-4.0f32..-0.05, 0.05f32..4.0], dims..=dims)
}

struct Fixture {
    _dir: tempfile::TempDir,
    db: Db,
    /// eid → stored vector, for live statements only.
    live: HashMap<Eid, Vec<f64>>,
}

fn fixture(
    dims: usize,
    metric: Metric,
    enc: VectorEncoding,
    vecs: &[Vec<f32>],
    retract: &[bool],
) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("p.db"), OpenOptions::default()).unwrap();
    let mut eids = Vec::new();
    db.transact(TxOptions::default(), |tx| {
        tx.define_embedding_model(
            model(),
            &EmbeddingModel {
                dimensions: dims as u32,
                metric,
                encoding: enc,
            },
        )?;
        for (i, x) in vecs.iter().enumerate() {
            let e = tx
                .assert(
                    v(&format!("s{i}")),
                    v("note"),
                    Value::Int(i as i64),
                    Valid::ALWAYS,
                )?
                .eid();
            tx.set_embedding(e, model(), x)?;
            eids.push(e);
        }
        Ok(())
    })
    .unwrap();
    let mut live = HashMap::new();
    db.transact(TxOptions::default(), |tx| {
        for (i, e) in eids.iter().enumerate() {
            if retract.get(i).copied().unwrap_or(false) {
                tx.retract(*e)?;
            } else {
                live.insert(*e, stored(&vecs[i], metric, enc));
            }
        }
        Ok(())
    })
    .unwrap();
    Fixture {
        _dir: dir,
        db,
        live,
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    // @tg: verifies:: [[tests#Vector Recall#Exact Search Equals Brute Force]] {since: "2026-10-11", level: unit}
    #[test]
    fn exact_search_equals_brute_force(
        (dims, vecs, q) in (2usize..10).prop_flat_map(|d| (
            Just(d),
            prop::collection::vec(vector(d), 1..30),
            vector(d),
        )),
        retract in prop::collection::vec(any::<bool>(), 30),
        metric in metric_strategy(),
        enc in encoding_strategy(),
        limit in 1usize..12,
    ) {
        let f = fixture(dims, metric, enc, &vecs, &retract);
        let m = f.db.now().encode(&model()).unwrap().unwrap();
        let qs = stored(&q, metric, VectorEncoding::F32);
        let r = f.db.now().vector_search(&VectorQuery {
            limit: Some(limit),
            ..VectorQuery::new(m, q.clone())
        }).unwrap();
        prop_assert!(r.exact);
        prop_assert_eq!(r.candidates as usize, f.live.len());
        prop_assert_eq!(r.hits.len(), limit.min(f.live.len()));
        let tol = 1e-3;
        let mut returned = std::collections::HashSet::new();
        for (i, h) in r.hits.iter().enumerate() {
            let x = f.live.get(&h.eid).expect("hits are live statements");
            prop_assert!((h.similarity - brute(metric, &qs, x)).abs() < tol);
            prop_assert_eq!(h.rank, i as u64 + 1);
            returned.insert(h.eid);
        }
        // no statement left out is closer than the last hit
        if let Some(last) = r.hits.last() {
            for (e, x) in &f.live {
                if !returned.contains(e) {
                    prop_assert!(brute(metric, &qs, x) <= last.similarity + tol);
                }
            }
        }
    }

    // @tg: verifies:: [[tests#Vector Recall#Prefilter Hits Have Exact Similarities]] {since: "2026-10-11", level: unit}
    #[test]
    fn prefilter_hits_carry_exact_similarities(
        (dims, vecs, q) in (2usize..10).prop_flat_map(|d| (
            Just(d),
            prop::collection::vec(vector(d), 1..30),
            vector(d),
        )),
        metric in metric_strategy(),
        n in 1usize..20,
    ) {
        let f = fixture(dims, metric, VectorEncoding::F32, &vecs, &[]);
        let m = f.db.now().encode(&model()).unwrap().unwrap();
        let qs = stored(&q, metric, VectorEncoding::F32);
        let r = f.db.now().vector_search(&VectorQuery {
            prefilter: Some(n),
            ..VectorQuery::new(m, q)
        }).unwrap();
        prop_assert!(!r.exact);
        prop_assert!(r.hits.windows(2).all(|w| w[0].similarity >= w[1].similarity));
        for h in &r.hits {
            let x = f.live.get(&h.eid).expect("hits are live");
            prop_assert!((h.similarity - brute(metric, &qs, x)).abs() < 1e-3);
        }
    }
}

/// Notes about one trip with words and 2-d vectors; returns (db, note eids).
fn notes(dir: &Path, items: &[(&str, [f32; 2])], text_index: bool) -> (Db, Vec<Eid>) {
    let db = Db::open(
        dir.join("h.db"),
        OpenOptions {
            text_index,
            ..OpenOptions::default()
        },
    )
    .unwrap();
    let mut eids = Vec::new();
    db.transact(TxOptions::default(), |tx| {
        tx.define_embedding_model(model(), &EmbeddingModel::new(2))?;
        for (text, x) in items {
            // parallel edges: generated notes may repeat a text
            let e = tx.create(v("trip"), v("note"), Value::str(*text), Valid::ALWAYS)?;
            tx.set_embedding(e, model(), x)?;
            eids.push(e);
        }
        Ok(())
    })
    .unwrap();
    (db, eids)
}

const WORDS: [&str; 6] = ["lisbon", "tram", "offsite", "june", "budget", "hotel"];

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    // @tg: verifies:: [[tests#Vector Recall#Fusion Recomputes From The Channels]] {since: "2026-10-11", level: unit}
    #[test]
    fn fusion_recomputes_from_the_standalone_channels(
        items in prop::collection::vec(
            (prop::collection::vec(0usize..6, 1..4), -1.0f32..1.0, 0.1f32..1.0),
            1..15,
        ),
        word in 0usize..6,
        q in (-1.0f32..1.0, 0.1f32..1.0),
    ) {
        let dir = tempfile::tempdir().unwrap();
        let texts: Vec<String> = items
            .iter()
            .map(|(ws, _, _)| ws.iter().map(|w| WORDS[*w]).collect::<Vec<_>>().join(" "))
            .collect();
        let rows: Vec<(&str, [f32; 2])> = texts
            .iter()
            .zip(&items)
            .map(|(t, (_, a, b))| (t.as_str(), [*a, *b]))
            .collect();
        let (db, _) = notes(dir.path(), &rows, true);
        let m = db.now().encode(&model()).unwrap().unwrap();
        let tq = TextQuery::new(WORDS[word]);
        let vq = VectorQuery::new(m, vec![q.0, q.1]);
        let depth = 50;
        let text = db.now().text_search(&TextQuery { limit: Some(depth), ..tq.clone() }).unwrap();
        let vecs = db.now().vector_search(&VectorQuery { limit: Some(depth), ..vq.clone() }).unwrap().hits;
        let fused = db.now().recall(&RecallQuery {
            text: Some(tq.clone()),
            vector: Some(vq.clone()),
            limit: Some(100),
            ..RecallQuery::default()
        }).unwrap();
        let mut expect: HashMap<Eid, f64> = HashMap::new();
        for h in &text {
            *expect.entry(h.eid).or_default() += 1.0 / (60.0 + h.rank as f64);
        }
        for h in &vecs {
            *expect.entry(h.eid).or_default() += 1.0 / (60.0 + h.rank as f64);
        }
        prop_assert_eq!(fused.len(), expect.len());
        for h in &fused {
            prop_assert_eq!(Some(&h.score), expect.get(&h.eid));
            prop_assert_eq!(h.text_rank, text.iter().find(|t| t.eid == h.eid).map(|t| t.rank));
            prop_assert_eq!(h.vector_rank, vecs.iter().find(|t| t.eid == h.eid).map(|t| t.rank));
        }
        prop_assert!(fused.windows(2).all(|w| w[0].score >= w[1].score));
        // a single channel keeps its standalone order
        let only_vec = db.now().recall(&RecallQuery {
            vector: Some(vq),
            limit: Some(100),
            ..RecallQuery::default()
        }).unwrap();
        prop_assert_eq!(
            only_vec.iter().map(|h| h.eid).collect::<Vec<_>>(),
            vecs.iter().map(|h| h.eid).collect::<Vec<_>>()
        );
        let only_text = db.now().recall(&RecallQuery {
            text: Some(tq),
            limit: Some(100),
            ..RecallQuery::default()
        }).unwrap();
        prop_assert_eq!(
            only_text.iter().map(|h| h.eid).collect::<Vec<_>>(),
            text.iter().map(|h| h.eid).collect::<Vec<_>>()
        );
    }
}

// @tg: verifies:: [[tests#Vector Recall#Hybrid Recall Honours Views And Graphs]] {since: "2026-10-11", level: unit}
#[test]
fn hybrid_recall_honours_views_and_graphs() {
    let dir = tempfile::tempdir().unwrap();
    let (db, eids) = notes(
        dir.path(),
        &[("lisbon offsite", [1.0, 0.0]), ("tram tickets", [0.0, 1.0])],
        true,
    );
    let t1 = db
        .events_since(0)
        .unwrap()
        .iter()
        .map(|e| e.t.0)
        .max()
        .unwrap();
    // later: a source layer on the first note, a new note, and a graph membership
    let mut later = None;
    db.transact(TxOptions::default(), |tx| {
        tx.assert(eids[0], v("source"), Value::str("chat-1"), Valid::ALWAYS)?;
        let e = tx
            .assert(
                v("trip"),
                v("note"),
                Value::str("lisbon hotel"),
                Valid::ALWAYS,
            )?
            .eid();
        tx.set_embedding(e, model(), &[0.9, 0.1])?;
        tx.add_to_graph(eids[1], v("chat"), AssertOpts::default())?;
        later = Some(e);
        Ok(())
    })
    .unwrap();
    let later = later.unwrap();
    let m = db.now().encode(&model()).unwrap().unwrap();
    let q = RecallQuery {
        text: Some(TextQuery::new("lisbon")),
        vector: Some(VectorQuery::new(m, vec![1.0, 0.0])),
        expand: 3,
        ..RecallQuery::default()
    };
    let ids = |hits: &[RecallHit]| hits.iter().map(|h| h.eid).collect::<Vec<_>>();
    // now: the later note is a fused hit and the source layer is expanded
    let now = db.now().recall(&q).unwrap();
    assert!(ids(&now).contains(&later));
    assert!(now
        .iter()
        .any(|h| h.via == Some(eids[0]) && h.s == eids[0].oid()));
    // as of t1: neither the later note nor the later layer exists
    let past = db.as_of(TimeRef::Tx(t1)).recall(&q).unwrap();
    assert!(!ids(&past).contains(&later));
    assert!(!past.iter().any(|h| h.s == eids[0].oid() && h.via.is_some()));
    assert!(ids(&past).contains(&eids[0]));
    // graphs restrict both channels' fused hits
    let chat = db.now().encode(&v("chat")).unwrap().unwrap();
    let scoped = db
        .now()
        .recall(&RecallQuery {
            text: Some(TextQuery {
                graphs: Some(vec![chat]),
                ..TextQuery::new("tram lisbon")
            }),
            vector: Some(VectorQuery {
                graphs: Some(vec![chat]),
                ..VectorQuery::new(m, vec![0.0, 1.0])
            }),
            ..RecallQuery::default()
        })
        .unwrap();
    assert_eq!(ids(&scoped), vec![eids[1]]);
}

/// The bundled host declaring no FTS5.
mod no_fts {
    use super::*;

    pub struct NoFts(pub RusqliteHost);
    pub struct NoFtsExec(Box<dyn Executor>);

    fn caps(c: Capabilities) -> Capabilities {
        Capabilities { fts5: false, ..c }
    }

    impl Executor for NoFtsExec {
        fn capabilities(&self) -> Capabilities {
            caps(self.0.capabilities())
        }
        fn execute(&mut self, sql: &str, params: &[SqlValue]) -> Result<usize> {
            self.0.execute(sql, params)
        }
        fn query(
            &mut self,
            sql: &str,
            params: &[SqlValue],
            row: &mut dyn FnMut(&[SqlValue]) -> Result<()>,
        ) -> Result<()> {
            self.0.query(sql, params, row)
        }
        fn execute_batch(&mut self, sql: &str) -> Result<()> {
            self.0.execute_batch(sql)
        }
        fn begin_immediate(&mut self) -> Result<()> {
            self.0.begin_immediate()
        }
        fn begin_read(&mut self) -> Result<()> {
            self.0.begin_read()
        }
        fn commit(&mut self) -> Result<()> {
            self.0.commit()
        }
        fn rollback(&mut self) -> Result<()> {
            self.0.rollback()
        }
        fn savepoint(&mut self, name: &str) -> Result<()> {
            self.0.savepoint(name)
        }
        fn rollback_to(&mut self, name: &str) -> Result<()> {
            self.0.rollback_to(name)
        }
        fn release(&mut self, name: &str) -> Result<()> {
            self.0.release(name)
        }
        fn registry(&mut self) -> Option<&mut dyn tm_core::HostRegistry> {
            self.0.registry()
        }
    }

    impl Host for NoFts {
        fn capabilities(&self) -> Capabilities {
            caps(self.0.capabilities())
        }
        fn open_writer(&self, path: &Path, opts: &HostOptions) -> Result<Box<dyn Executor>> {
            Ok(Box::new(NoFtsExec(self.0.open_writer(path, opts)?)))
        }
        fn open_reader(&self, path: &Path, opts: &HostOptions) -> Result<Box<dyn Executor>> {
            Ok(Box::new(NoFtsExec(self.0.open_reader(path, opts)?)))
        }
    }
}

// @tg: verifies:: [[tests#Vector Recall#Channel Failures Are Typed]] {since: "2026-10-11", level: unit}
#[test]
fn channel_failures_are_typed_and_vectors_need_no_fts5() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open_with_host(
        no_fts::NoFts(RusqliteHost::new()),
        dir.path().join("n.db"),
        OpenOptions::default(),
    )
    .unwrap();
    let mut e = None;
    db.transact(TxOptions::default(), |tx| {
        tx.define_embedding_model(model(), &EmbeddingModel::new(2))?;
        let x = tx
            .assert(v("trip"), v("note"), Value::str("lisbon"), Valid::ALWAYS)?
            .eid();
        tx.set_embedding(x, model(), &[1.0, 0.0])?;
        e = Some(x);
        Ok(())
    })
    .unwrap();
    let m = db.now().encode(&model()).unwrap().unwrap();
    // vectors need no FTS5
    let vec_only = db
        .now()
        .recall(&RecallQuery {
            vector: Some(VectorQuery::new(m, vec![1.0, 0.0])),
            ..RecallQuery::default()
        })
        .unwrap();
    assert_eq!(vec_only[0].eid, e.unwrap());
    // a text channel on a host without FTS5 is a typed error, as text recall is
    let r = db.now().recall(&RecallQuery {
        text: Some(TextQuery::new("lisbon")),
        vector: Some(VectorQuery::new(m, vec![1.0, 0.0])),
        ..RecallQuery::default()
    });
    assert!(
        matches!(&r, Err(Error::MissingCapability { capability }) if capability == "fts5"),
        "{r:?}"
    );
    // an undefined model is InvalidQuery
    let other = db.now().encode(&v("trip")).unwrap().unwrap();
    let r = db.now().recall(&RecallQuery {
        vector: Some(VectorQuery::new(other, vec![1.0, 0.0])),
        ..RecallQuery::default()
    });
    assert!(matches!(r, Err(Error::InvalidQuery { .. })), "{r:?}");
}

// @tg: verifies:: [[tests#Vector Recall#SPARQL And Cypher Share The Recall]] {since: "2026-10-11", level: unit}
#[test]
fn sparql_and_cypher_run_the_same_recall_and_predicate_filters() {
    let dir = tempfile::tempdir().unwrap();
    let (db, eids) = notes(
        dir.path(),
        &[
            ("lisbon offsite", [1.0, 0.0]),
            ("tram tickets", [0.0, 1.0]),
            ("lisbon hotel", [0.7, 0.7]),
        ],
        true,
    );
    db.transact(TxOptions::default(), |tx| {
        tx.assert(eids[0], v("source"), Value::str("chat-1"), Valid::ALWAYS)?;
        let e = tx
            .assert(
                v("trip"),
                v("budget"),
                Value::str("lisbon 2k"),
                Valid::ALWAYS,
            )?
            .eid();
        tx.set_embedding(e, model(), &[1.0, 0.05])?;
        Ok(())
    })
    .unwrap();
    let m = db.now().encode(&model()).unwrap().unwrap();
    let api = db
        .now()
        .recall(&RecallQuery {
            text: Some(TextQuery::new("lisbon")),
            vector: Some(VectorQuery::new(m, vec![1.0, 0.0])),
            expand: 1,
            ..RecallQuery::default()
        })
        .unwrap();
    assert!(api.iter().any(|h| h.via.is_some()));
    // SPARQL
    let q = "SELECT ?e ?score ?rank ?via WHERE { \
               ?e tm:recallText \"lisbon\" ; tm:recallVector \"[1, 0]\" ; \
                  tm:recallModel v:prop-model ; tm:recallExpand 1 ; \
                  tm:recallScore ?score ; tm:recallRank ?rank ; tm:recallVia ?via } ORDER BY ?rank";
    let res = db.now().sparql(q).unwrap();
    let sol = res.solutions().unwrap();
    let col = |n: &str| sol.vars.iter().position(|x| x == n).unwrap();
    assert_eq!(sol.rows.len(), api.len());
    for (row, h) in sol.rows.iter().zip(&api) {
        assert_eq!(row[col("e")], Some(Value::Stmt(h.eid)));
        assert_eq!(row[col("score")], Some(Value::Double(h.score)));
        assert_eq!(row[col("rank")], Some(Value::Int(h.rank as i64)));
        assert_eq!(row[col("via")], h.via.map(Value::Stmt));
    }
    for bad in [
        "SELECT ?e WHERE { ?e tm:recallScore ?s }",
        "SELECT ?e WHERE { ?e tm:recallVector \"[1, 0]\" }",
        "SELECT ?e WHERE { GRAPH ?g { ?e tm:recallText \"lisbon\" } }",
    ] {
        assert!(
            matches!(db.now().sparql(bad), Err(Error::InvalidQuery { .. })),
            "{bad}"
        );
    }
    // Cypher
    let none = CypherParams::default();
    let c = db
        .now()
        .cypher(
            "CALL tiramemsu.recall({text: 'lisbon', model: 'prop-model', vector: [1, 0], expand: 1}) \
             YIELD statement, score, rank, via RETURN statement, score, rank, via",
            &none,
        )
        .unwrap();
    assert_eq!(c.rows.len(), api.len());
    for (row, h) in c.rows.iter().zip(&api) {
        match &row[0] {
            CypherValue::Node(n) => {
                assert_eq!(n.element_id, format!("urn:tiramemsu:stmt:{}", h.eid.n()))
            }
            other => panic!("statement is {other:?}"),
        }
        assert_eq!(row[1], CypherValue::Float(h.score));
        assert_eq!(row[2], CypherValue::Integer(h.rank as i64));
        match (&row[3], h.via) {
            (CypherValue::Null, None) => {}
            (CypherValue::Node(n), Some(via)) => {
                assert_eq!(n.element_id, format!("urn:tiramemsu:stmt:{}", via.n()))
            }
            other => panic!("via is {other:?}"),
        }
    }
    assert!(db
        .now()
        .cypher("CALL tiramemsu.recall({model: 'prop-model'})", &none)
        .is_err());
    // the predicate filter of vector recall, in both languages
    let c = db
        .now()
        .cypher(
            "CALL tiramemsu.vector.search('prop-model', [1, 0], {predicates: ['budget']}) \
             YIELD predicate RETURN predicate",
            &none,
        )
        .unwrap();
    assert_eq!(c.rows, vec![vec![CypherValue::String("budget".into())]]);
    let s = db
        .now()
        .sparql(
            "SELECT ?e WHERE { ?e tm:vectorMatch \"[1, 0]\" ; tm:vectorModel v:prop-model ; \
             tm:vectorPredicate v:budget }",
        )
        .unwrap();
    assert_eq!(s.solutions().unwrap().rows.len(), 1);
}

// @tg: verifies:: [[tests#Vector Recall#Hybrid Expansion Is Capped And Budgeted]] {since: "2026-10-11", level: unit}
#[test]
fn hybrid_expansion_is_capped_and_a_budget_bounds_recall() {
    let dir = tempfile::tempdir().unwrap();
    let (db, eids) = notes(dir.path(), &[("lisbon offsite", [1.0, 0.0])], true);
    db.transact(TxOptions::default(), |tx| {
        for i in 0..5 {
            tx.assert(eids[0], v("source"), Value::Int(i), Valid::ALWAYS)?;
        }
        Ok(())
    })
    .unwrap();
    let m = db.now().encode(&model()).unwrap().unwrap();
    let q = |expand| RecallQuery {
        text: Some(TextQuery::new("lisbon")),
        vector: Some(VectorQuery::new(m, vec![1.0, 0.0])),
        expand,
        ..RecallQuery::default()
    };
    for n in [0usize, 1, 3, 10] {
        let hits = db.now().recall(&q(n)).unwrap();
        let layers = hits
            .iter()
            .filter(|h| h.via == Some(eids[0]) && h.s == eids[0].oid())
            .count();
        assert_eq!(layers, n.min(5), "expand {n}");
        // expanded statements follow the fused hits and are numbered after them
        assert!(hits.iter().skip(1).all(|h| h.via.is_some()));
        assert_eq!(
            hits.iter().map(|h| h.rank).collect::<Vec<_>>(),
            (1..=hits.len() as u64).collect::<Vec<_>>()
        );
    }
    let tight = QueryBudget {
        max_rows: Some(2),
        ..QueryBudget::default()
    };
    assert!(matches!(
        db.now().with_budget(&tight).recall(&q(5)),
        Err(Error::ResultLimitExceeded { .. })
    ));
    let cancel = CancelToken::new();
    cancel.cancel();
    let stopped = QueryBudget {
        cancel: Some(cancel),
        ..QueryBudget::default()
    };
    assert!(matches!(
        db.now().with_budget(&stopped).recall(&q(0)),
        Err(Error::Cancelled)
    ));
}
