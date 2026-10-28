//! The oracle harness shared by every backend's tests: random operation
//! sequences run on `tm_core::Store` over `tm-rusqlite` (the oracle) and on
//! `tm_store::Engine` over the backend under test, and every report, error and
//! the final stored state must be identical. Included with `#[path]`.

#![allow(dead_code)]

use std::sync::Arc;

use proptest::prelude::*;
use tm_core::read as core_read;
use tm_core::storage::meta::Counters;
use tm_core::vector::EmbeddingModel;
use tm_core::vocab::{self, v};
use tm_core::{
    AssertOpts, Eid, Error, ManualClock, OnExisting, Patch, Result, Tag, TxOptions, TxReport,
    Valid, Value, ViewSpec,
};
use tm_rusqlite::RusqliteHost;
use tm_store::{Backend, Engine, EngineOptions, ReadStore};

/// One operation of a generated transaction. Indices pick from pools, so every
/// generated sequence is valid input; the engines decide what fails.
#[derive(Clone, Debug)]
pub enum Op {
    Assert(u8, u8, u8, u8),
    Create(u8, u8, u8),
    AssertOnStmt(u8, u8, u8),
    Confirm(u8),
    AssertConfirm(u8, u8, u8),
    Retract(u8),
    RetractMatching(Option<u8>, Option<u8>),
    Supersede(u8, u8, u8),
    AddToGraph(u8, u8),
    RemoveFromGraph(u8, u8),
    ClearGraph(u8),
    CreateGraph(u8),
    DropGraph(u8),
    Schema(u8, u8),
    Upsert(u8, u8),
    Meta(u8),
    Volatile(u8, u8, bool),
    Vocab(u8),
    Prefix(u8, u8),
    Model(u8),
    Embed(u8, u8),
    Derive(u8, u8, u8, u8, u8),
    UpsertDerived(u8, u8, u8),
    Fail,
}

fn subject(i: u8) -> Value {
    Value::iri(v(&format!("s{}", i % 6)))
}

fn predicate(i: u8) -> Value {
    Value::iri(v(["p", "q", "r", "name", "age"][(i % 5) as usize]))
}

fn object(i: u8) -> Value {
    match i % 8 {
        0 => Value::Int(i64::from(i % 3)),
        1 => Value::str(format!("text {}", i % 4)),
        2 => Value::str("a much longer string value that is stored in the dictionary"),
        3 => Value::iri(v(&format!("o{}", i % 3))),
        4 => Value::Double(f64::from(i % 5) / 2.0),
        5 => Value::LangStr {
            lex: format!("Grüße {}", i % 2),
            lang: "de".to_string(),
        },
        6 => Value::Bool(i.is_multiple_of(2)),
        _ => subject(i),
    }
}

fn valid(i: u8) -> Valid {
    match i % 5 {
        0 | 1 => Valid::ALWAYS,
        2 => Valid::between(10, 20),
        3 => Valid::from(15),
        _ => Valid::between(i64::from(i % 3) * 7, 30),
    }
}

fn graph(i: u8) -> Value {
    Value::iri(v(&format!("g{}", i % 3)))
}

fn op() -> impl Strategy<Value = Op> {
    let b = any::<u8>;
    prop_oneof![
        6 => (b(), b(), b(), b()).prop_map(|(a, c, d, e)| Op::Assert(a, c, d, e)),
        2 => (b(), b(), b()).prop_map(|(a, c, d)| Op::Create(a, c, d)),
        3 => (b(), b(), b()).prop_map(|(a, c, d)| Op::AssertOnStmt(a, c, d)),
        1 => b().prop_map(Op::Confirm),
        1 => (b(), b(), b()).prop_map(|(a, c, d)| Op::AssertConfirm(a, c, d)),
        3 => b().prop_map(Op::Retract),
        1 => (proptest::option::of(b()), proptest::option::of(b()))
            .prop_map(|(a, c)| Op::RetractMatching(a, c)),
        2 => (b(), b(), b()).prop_map(|(a, c, d)| Op::Supersede(a, c, d)),
        2 => (b(), b()).prop_map(|(a, c)| Op::AddToGraph(a, c)),
        1 => (b(), b()).prop_map(|(a, c)| Op::RemoveFromGraph(a, c)),
        1 => b().prop_map(Op::ClearGraph),
        1 => b().prop_map(Op::CreateGraph),
        1 => b().prop_map(Op::DropGraph),
        2 => (b(), b()).prop_map(|(a, c)| Op::Schema(a, c)),
        1 => (b(), b()).prop_map(|(a, c)| Op::Upsert(a, c)),
        1 => b().prop_map(Op::Meta),
        1 => (b(), b(), any::<bool>()).prop_map(|(a, c, d)| Op::Volatile(a, c, d)),
        1 => b().prop_map(Op::Vocab),
        1 => (b(), b()).prop_map(|(a, c)| Op::Prefix(a, c)),
        1 => b().prop_map(Op::Model),
        1 => (b(), b()).prop_map(|(a, c)| Op::Embed(a, c)),
        3 => (b(), b(), b(), b(), b()).prop_map(|(a, c, d, e, f)| Op::Derive(a, c, d, e, f)),
        1 => (b(), b(), b()).prop_map(|(a, c, d)| Op::UpsertDerived(a, c, d)),
        1 => Just(Op::Fail),
    ]
}

/// One transaction: its operations and how it ends.
#[derive(Clone, Debug)]
pub struct TxPlan {
    pub ops: Vec<Op>,
    pub dry_run: bool,
    pub max_cascade: usize,
}

pub fn tx_plan() -> impl Strategy<Value = TxPlan> {
    (
        prop::collection::vec(op(), 1..5),
        prop::bool::weighted(0.1),
        prop_oneof![9 => Just(10_000usize), 1 => Just(2usize)],
    )
        .prop_map(|(ops, dry_run, max_cascade)| TxPlan {
            ops,
            dry_run,
            max_cascade,
        })
}

/// The same body for both `Tx` types: they share every method name and signature.
macro_rules! apply_fn {
    ($name:ident, $tx:ty) => {
        fn $name(tx: &mut $tx, op: &Op, known: &[Eid]) -> Result<()> {
            let pick = |i: u8| known.get(i as usize % known.len().max(1)).copied();
            match op {
                Op::Assert(s, p, o, w) => {
                    tx.assert(subject(*s), predicate(*p), object(*o), valid(*w))?;
                }
                Op::Create(s, p, o) => {
                    tx.create(subject(*s), predicate(*p), object(*o), valid(*o))?;
                }
                Op::AssertOnStmt(e, p, o) => {
                    if let Some(e) = pick(*e) {
                        tx.assert(e, predicate(*p), object(*o), Valid::ALWAYS)?;
                    }
                }
                Op::Confirm(e) => {
                    if let Some(e) = pick(*e) {
                        tx.confirm(e)?;
                    }
                }
                Op::AssertConfirm(s, p, o) => {
                    tx.assert_with(
                        subject(*s),
                        predicate(*p),
                        object(*o),
                        AssertOpts {
                            valid: Valid::ALWAYS,
                            on_existing: OnExisting::Confirm,
                        },
                    )?;
                }
                Op::Retract(e) => {
                    if let Some(e) = pick(*e) {
                        tx.retract(e)?;
                    }
                }
                Op::RetractMatching(s, p) => {
                    let s = s.map(|s| tx.encode(subject(s))).transpose()?;
                    let p = p.map(|p| tx.encode(predicate(p))).transpose()?;
                    tx.retract_matching(s, p, None)?;
                }
                Op::Supersede(e, o, w) => {
                    if let Some(e) = pick(*e) {
                        let mut patch = Patch::object(object(*o));
                        if *w % 3 == 0 {
                            patch.v_from = Some(Some(i64::from(*w % 7)));
                        }
                        tx.supersede(e, patch)?;
                    }
                }
                Op::AddToGraph(e, g) => {
                    if let Some(e) = pick(*e) {
                        tx.add_to_graph(e, graph(*g), AssertOpts::default())?;
                    }
                }
                Op::RemoveFromGraph(e, g) => {
                    if let Some(e) = pick(*e) {
                        tx.remove_from_graph(e, graph(*g))?;
                    }
                }
                Op::ClearGraph(g) => {
                    tx.clear_graph(graph(*g))?;
                }
                Op::CreateGraph(g) => {
                    tx.create_graph(graph(*g))?;
                }
                Op::DropGraph(g) => {
                    tx.drop_graph(graph(*g))?;
                }
                Op::Schema(p, f) => {
                    let (flag, o) = match f % 6 {
                        0 => (vocab::SYS_CARDINALITY, Value::iri(vocab::SYS_ONE)),
                        1 => (vocab::SYS_UNIQUE, Value::Bool(true)),
                        2 => (vocab::SYS_VALUE_TYPE, Value::iri(vocab::XSD_INTEGER)),
                        3 => (
                            vocab::SYS_SUBJECT_TYPE,
                            Value::iri(vocab::tag_iri(Tag::Stmt)),
                        ),
                        4 => (
                            vocab::SYS_SUBJECT_TYPE,
                            Value::iri(vocab::tag_iri(Tag::Iri)),
                        ),
                        _ => (vocab::SYS_CARDINALITY, Value::iri(vocab::SYS_MANY)),
                    };
                    tx.assert(predicate(*p), Value::iri(flag), o, Valid::ALWAYS)?;
                }
                Op::Upsert(p, o) => {
                    tx.upsert(predicate(*p), object(*o))?;
                }
                Op::Meta(o) => {
                    tx.meta(Value::iri(vocab::SYS_REASON), object(*o))?;
                }
                Op::Volatile(s, o, set) => {
                    if *set {
                        tx.set_volatile(subject(*s), predicate(*o), object(*o))?;
                    } else {
                        tx.clear_volatile(subject(*s), predicate(*o))?;
                    }
                }
                Op::Vocab(i) => tx.set_vocab(&format!("http://example.org/v{}/", i % 2))?,
                Op::Prefix(n, i) => tx.set_prefix(
                    ["ex", "foaf", "sys"][(*n % 3) as usize],
                    &format!("http://example.org/p{}/", i % 2),
                )?,
                Op::Model(d) => {
                    tx.define_embedding_model(
                        Value::iri(v("model")),
                        &EmbeddingModel::new(u32::from(d % 2) + 2),
                    )?;
                }
                Op::Embed(e, x) => {
                    if let Some(e) = pick(*e) {
                        let x = f32::from(*x % 4);
                        tx.set_embedding(e, Value::iri(v("model")), &[1.0, x])?;
                    }
                }
                Op::Derive(s, p, o, a, b) => {
                    if let (Some(a), Some(b)) = (pick(*a), pick(*b)) {
                        let support = if a == b { vec![a] } else { vec![a, b] };
                        tx.derive(subject(*s), predicate(*p), object(*o), valid(*o), &support)?;
                    }
                }
                Op::UpsertDerived(p, o, a) => {
                    if let Some(a) = pick(*a) {
                        tx.upsert_derived(predicate(*p), object(*o), &[a])?;
                    }
                }
                Op::Fail => return Err(Error::custom("body failed")),
            }
            Ok(())
        }
    };
}

apply_fn!(apply_core, tm_core::Tx<'_>);
apply_fn!(apply_store, tm_store::Tx<'_>);

/// An outcome that compares across engines: the report or the error text.
fn outcome(r: Result<TxReport>) -> std::result::Result<TxReport, String> {
    r.map_err(|e| e.to_string())
}

// @tg: implements:: [[openspec:storage-trait#Engine equivalence with SQLite]] {since: "2026-10-27", confidence: full}
pub struct Pair {
    _dir: tempfile::TempDir,
    core: tm_core::Store,
    store: Engine,
    clock: Arc<ManualClock>,
    known: Vec<Eid>,
    /// Whether the text index is on (and text recall compared).
    text: bool,
}

impl Pair {
    pub fn new(backend: Arc<dyn Backend>) -> Pair {
        Pair::new_with(backend, true)
    }

    /// A pair with the text index on or off on both sides.
    pub fn new_with(backend: Arc<dyn Backend>, text: bool) -> Pair {
        let dir = tempfile::tempdir().unwrap();
        let clock = Arc::new(ManualClock::new(1_000));
        let core = tm_core::Store::open(
            &RusqliteHost::new(),
            &dir.path().join("oracle.db"),
            tm_core::StoreOptions {
                clock: clock.clone(),
                ..Default::default()
            },
        )
        .unwrap();
        let mut core = core;
        if text {
            core.enable_text_index().unwrap();
        }
        let store = Engine::open(
            backend,
            EngineOptions {
                clock: clock.clone(),
                ..Default::default()
            },
        )
        .unwrap();
        let mut store = store;
        if text {
            store.enable_text_index().unwrap();
        }
        Pair {
            _dir: dir,
            core,
            store,
            clock,
            known: Vec::new(),
            text,
        }
    }

    pub fn run(
        &mut self,
        plan: &TxPlan,
    ) -> (
        std::result::Result<TxReport, String>,
        std::result::Result<TxReport, String>,
    ) {
        let opts = TxOptions {
            dry_run: plan.dry_run,
            max_cascade: plan.max_cascade,
            ..TxOptions::default()
        };
        let known = self.known.clone();
        let a = outcome(self.core.transact(opts, |tx| {
            plan.ops
                .iter()
                .try_for_each(|op| apply_core(tx, op, &known))
        }));
        let b = outcome(self.store.transact(opts, |tx| {
            plan.ops
                .iter()
                .try_for_each(|op| apply_store(tx, op, &known))
        }));
        if let Ok(r) = &a {
            if !plan.dry_run {
                self.known.extend(r.asserted.iter().copied());
                self.known.extend(r.memberships.iter().copied());
            }
        }
        self.clock.advance(7);
        (a, b)
    }

    pub fn speculate(
        &mut self,
        plan: &TxPlan,
    ) -> (
        std::result::Result<Vec<tm_core::Triple>, String>,
        std::result::Result<Vec<tm_core::Triple>, String>,
    ) {
        let known = self.known.clone();
        let a = self
            .core
            .speculate(
                |tx| {
                    plan.ops
                        .iter()
                        .try_for_each(|op| apply_core(tx, op, &known))
                },
                |e| core_read::triples(e, &ViewSpec::NOW, None, None, None),
            )
            .map_err(|e| e.to_string());
        let b = self
            .store
            .speculate(
                |tx| {
                    plan.ops
                        .iter()
                        .try_for_each(|op| apply_store(tx, op, &known))
                },
                |r| tm_store::read::triples(r, &ViewSpec::NOW, None, None, None),
            )
            .map_err(|e| e.to_string());
        self.clock.advance(3);
        (a, b)
    }

    /// Compares text, vector and hybrid recall on both engines.
    pub fn compare_recall(&mut self, r: &dyn ReadStore, last_t: u64) {
        use tm_core::text::{TextMode, TextQuery};
        use tm_core::vector::VectorQuery;
        let core_terms = tm_core::TermReader::new(64);
        let store_terms = tm_store::dict::TermCache::new(64);
        let views = [
            ViewSpec::NOW,
            ViewSpec::history(),
            ViewSpec::as_of(tm_core::TimeRef::Tx(last_t / 2)),
        ];
        for spec in views {
            for text in [
                "text",
                "text 1",
                "TEXT 3",
                "much string*",
                "lon*",
                "grüße",
                "GRUSSE 1",
                "value longer",
                "*",
            ] {
                for mode in [TextMode::All, TextMode::Any, TextMode::Phrase] {
                    let q = TextQuery {
                        mode,
                        ..TextQuery::new(text)
                    };
                    let a = self
                        .core
                        .read(|e| tm_core::text::search(e, &spec, &q, &core_terms, true))
                        .map_err(|x| x.to_string());
                    let b = tm_store::search::text_search(r, &spec, &q, &store_terms, true)
                        .map_err(|x| x.to_string());
                    match (&a, &b) {
                        (Ok(a), Ok(b)) => {
                            assert_eq!(a.len(), b.len(), "text {text:?} {mode:?} in {spec:?}");
                            for (x, y) in a.iter().zip(b) {
                                assert!(
                                    (x.lexical - y.lexical).abs()
                                        <= 1e-9 * x.lexical.abs().max(1.0),
                                    "score of {text:?}: {} vs {}",
                                    x.lexical,
                                    y.lexical
                                );
                                assert_eq!(
                                    (x.eid, x.rank, &x.text, &x.lang, &x.evidence),
                                    (y.eid, y.rank, &y.text, &y.lang, &y.evidence),
                                    "text hit of {text:?} {mode:?}"
                                );
                            }
                        }
                        _ => assert_eq!(a.map(|_| ()), b.map(|_| ()), "text {text:?}"),
                    }
                }
            }
            let model = self
                .core
                .read(|e| tm_core::TermReader::encode(e, &Value::iri(v("model"))))
                .unwrap();
            if let Some(model) = model {
                for prefilter in [None, Some(1)] {
                    let q = VectorQuery {
                        prefilter,
                        ..VectorQuery::new(model, vec![1.0, 0.5])
                    };
                    let a = self
                        .core
                        .read(|e| tm_core::vector::search(e, &spec, &q, &core_terms, true))
                        .map_err(|x| x.to_string());
                    let b = tm_store::search::vector_search(r, &spec, &q, &store_terms, true)
                        .map_err(|x| x.to_string());
                    assert_eq!(a, b, "vector search in {spec:?}");
                }
                let a = self
                    .core
                    .read(|e| tm_core::vector::unembedded(e, &spec, model, Some(5)))
                    .unwrap();
                assert_eq!(
                    a,
                    tm_store::search::unembedded(r, &spec, model, Some(5)).unwrap()
                );
                let q = tm_core::RecallQuery {
                    text: Some(TextQuery::new("text value")),
                    vector: Some(VectorQuery::new(model, vec![1.0, 0.5])),
                    expand: 2,
                    ..Default::default()
                };
                let a = self
                    .core
                    .read(|e| tm_core::recall::recall(e, &spec, &q, &core_terms, true))
                    .map_err(|x| x.to_string());
                let b = tm_store::search::recall(r, &spec, &q, &store_terms, true)
                    .map_err(|x| x.to_string());
                match (&a, &b) {
                    (Ok(a), Ok(b)) => {
                        let key = |h: &tm_core::RecallHit| {
                            (h.eid, h.rank, h.via, h.text_rank, h.vector_rank)
                        };
                        assert_eq!(
                            a.iter().map(key).collect::<Vec<_>>(),
                            b.iter().map(key).collect::<Vec<_>>(),
                            "recall in {spec:?}"
                        );
                    }
                    _ => assert_eq!(a.map(|_| ()), b.map(|_| ()), "recall"),
                }
            }
        }
    }

    /// Compares every stored table of both engines.
    pub fn compare_state(&mut self) {
        let core_hist = self
            .core
            .read(|e| core_read::triples(e, &ViewSpec::history(), None, None, None))
            .unwrap();
        let reader = self.store.reader().unwrap();
        let r: &dyn ReadStore = &*reader;
        let store_hist =
            tm_store::read::triples(r, &ViewSpec::history(), None, None, None).unwrap();
        assert_eq!(core_hist, store_hist, "statement tables differ");

        let c = self.core.read(|e| Counters::load(e)).unwrap();
        let s = tm_store::engine::load_counters(r).unwrap();
        assert_eq!(c, s, "counters differ");

        for id in 1..c.next_term {
            let a = self.core.read(|e| tm_core::term::load_term(e, id)).unwrap();
            let b = tm_store::dict::load_term(r, id).unwrap();
            assert_eq!(a, b, "term {id} differs");
        }
        for t in 1..=c.last_t {
            let a = self
                .core
                .read(|e| core_read::tx_instant(e, t as u64))
                .unwrap();
            assert_eq!(a, r.tx_instant(t).unwrap(), "transaction {t} differs");
        }
        let a = self
            .core
            .read(|e| e.rows("SELECT p FROM pred_multi ORDER BY p", &[]))
            .unwrap()
            .into_iter()
            .filter_map(|row| row[0].as_i64())
            .collect::<Vec<_>>();
        assert_eq!(a, r.pred_multi_all().unwrap(), "pred_multi differs");

        let a = self.core.read(|e| core_read::events_since(e, 0)).unwrap();
        assert_eq!(
            a,
            tm_store::read::events_since(r, 0).unwrap(),
            "event logs differ"
        );

        for spec in [ViewSpec::NOW, ViewSpec::NOW.valid_at(16)] {
            let a = self.core.read(|e| core_read::graphs(e, &spec)).unwrap();
            assert_eq!(
                a,
                tm_store::read::graphs(r, &spec).unwrap(),
                "graphs differ"
            );
            for g in &a {
                let x = self
                    .core
                    .read(|e| core_read::graph_members(e, &spec, *g))
                    .unwrap();
                assert_eq!(x, tm_store::read::graph_members(r, &spec, *g).unwrap());
            }
        }
        for e in self.known.iter().take(12) {
            let a = self
                .core
                .read(|x| core_read::dependents(x, &ViewSpec::NOW, *e))
                .unwrap();
            assert_eq!(
                a,
                tm_store::read::dependents(r, &ViewSpec::NOW, *e).unwrap()
            );
            let a = self
                .core
                .read(|x| core_read::bundle(x, &ViewSpec::NOW, *e))
                .map_err(|x| x.to_string());
            let b = tm_store::read::bundle(r, &ViewSpec::NOW, *e).map_err(|x| x.to_string());
            assert_eq!(a, b, "bundle of {e:?} differs");
        }
        let last_t = c.last_t as u64;
        for spec in [
            ViewSpec::NOW,
            ViewSpec::as_of(tm_core::TimeRef::Tx(last_t / 2)),
            ViewSpec::NOW.valid_at(12),
        ] {
            let q = tm_core::ConflictQuery::default();
            let a = self
                .core
                .read(|e| {
                    tm_core::conflict::inspect(e, &spec, &q, &tm_core::TermReader::new(64), true)
                })
                .map_err(|x| x.to_string());
            let b = tm_store::conflict::inspect(
                r,
                &spec,
                &q,
                &tm_store::dict::TermCache::new(64),
                true,
            )
            .map_err(|x| x.to_string());
            assert_eq!(a, b, "conflicts differ in {spec:?}");
            let a = self
                .core
                .read(|e| core_read::triples(e, &spec, None, None, None))
                .unwrap();
            assert_eq!(
                a,
                tm_store::read::triples(r, &spec, None, None, None).unwrap(),
                "view {spec:?} differs"
            );
        }
        if self.text {
            self.compare_recall(r, c.last_t as u64);
        }
        for s in 0..6u8 {
            for p in 0..5u8 {
                let (Some(sid), Some(pid)) = (
                    self.core
                        .read(|e| tm_core::TermReader::encode(e, &subject(s)))
                        .unwrap(),
                    self.core
                        .read(|e| tm_core::TermReader::encode(e, &predicate(p)))
                        .unwrap(),
                ) else {
                    continue;
                };
                let a = self
                    .core
                    .read(|e| core_read::values(e, &ViewSpec::NOW, sid, pid))
                    .unwrap();
                assert_eq!(
                    a,
                    tm_store::read::values(r, &ViewSpec::NOW, sid, pid).unwrap()
                );
            }
        }
    }
}

pub fn cases() -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(64)
}

/// Runs generated histories on the oracle and on backends made by `make`.
pub fn random_histories(make: impl Fn() -> Arc<dyn Backend>) {
    random_histories_with(make, true)
}

/// [`random_histories`] with the text index on or off.
pub fn random_histories_with(make: impl Fn() -> Arc<dyn Backend>, text: bool) {
    let cfg = ProptestConfig::with_cases(cases());
    proptest!(cfg, |(plans in prop::collection::vec((tx_plan(), prop::bool::weighted(0.1)), 1..25))| {
        let mut pair = Pair::new_with(make(), text);
        for (i, (plan, speculate)) in plans.iter().enumerate() {
            if *speculate {
                let (a, b) = pair.speculate(plan);
                prop_assert_eq!(a, b, "speculation {} differs: {:?}", i, plan);
            } else {
                let (a, b) = pair.run(plan);
                prop_assert_eq!(a, b, "transaction {} differs: {:?}", i, plan);
            }
        }
        pair.compare_state();
    });
}

/// A fixed history of derivations: derive, re-derive, a parallel derivation, a
/// derived key, a supersede of a support and of a derived statement (widening),
/// and a retraction of a support, with every report and the final state equal.
pub fn derived_history(backend: Arc<dyn Backend>) {
    let mut pair = Pair::new_with(backend, false);
    let plans = [
        // known: 0..=3 base facts
        vec![
            Op::Assert(0, 0, 3, 0),
            Op::Assert(0, 0, 11, 0),
            Op::Assert(1, 1, 3, 0),
            Op::Assert(2, 2, 3, 0),
        ],
        // 4 = D from [0, 1], 5, 6 its links; again (Existing); 7 = D2 from [2], 8 its link
        vec![
            Op::Derive(3, 1, 19, 0, 1),
            Op::Derive(3, 1, 19, 1, 0),
            Op::Derive(3, 1, 19, 2, 2),
        ],
        // a layer on D, a derived key, then the same key again
        vec![
            Op::AssertOnStmt(4, 2, 1),
            Op::UpsertDerived(3, 1, 3),
            Op::UpsertDerived(3, 1, 3),
        ],
        // correcting a support drops D; widening D2 keeps its link
        vec![Op::Supersede(0, 9, 3)],
        vec![Op::Supersede(7, 19, 3)],
        // retracting a support retracts the derived key
        vec![Op::Retract(3)],
        vec![Op::Retract(2)],
    ];
    for ops in plans {
        let (a, b) = pair.run(&TxPlan {
            ops,
            dry_run: false,
            max_cascade: 10_000,
        });
        assert!(a.is_ok(), "{a:?}");
        assert_eq!(a, b);
    }
    pair.compare_state();
}

/// A fixed history touching every operation kind.
pub fn fixed_history(backend: Arc<dyn Backend>) {
    fixed_history_with(backend, true)
}

/// [`fixed_history`] with the text index on or off.
pub fn fixed_history_with(backend: Arc<dyn Backend>, text: bool) {
    let mut pair = Pair::new_with(backend, text);
    let plans = [
        vec![
            Op::Assert(0, 0, 3, 0),
            Op::Assert(1, 1, 1, 2),
            Op::Create(0, 0, 3),
        ],
        vec![
            Op::AssertOnStmt(0, 2, 1),
            Op::AssertOnStmt(1, 3, 5),
            Op::AddToGraph(0, 1),
        ],
        vec![Op::Schema(0, 0), Op::Assert(0, 0, 0, 0)],
        vec![Op::Supersede(0, 9, 3)],
        vec![Op::Retract(1), Op::Volatile(2, 3, true)],
        vec![Op::Model(0), Op::Embed(0, 2)],
        vec![Op::Prefix(0, 1), Op::Vocab(1), Op::Upsert(1, 1)],
        vec![Op::Derive(2, 2, 3, 0, 1), Op::UpsertDerived(3, 3, 0)],
        vec![Op::Retract(0)],
    ];
    for ops in plans {
        let (a, b) = pair.run(&TxPlan {
            ops,
            dry_run: false,
            max_cascade: 10_000,
        });
        assert_eq!(a, b);
    }
    pair.compare_state();
}
