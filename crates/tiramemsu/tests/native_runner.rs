//! `native-query-execution`: the native runner on redb answers like the SQL
//! runner on SQLite, orders joins from statistics, joins cyclic cores worst-case
//! optimally, decodes late, runs large steps in parallel, honours budgets and
//! explains its plan.
#![cfg(all(feature = "redb", feature = "sparql", feature = "cypher"))]
mod differential;

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use differential::fixtures::{self, NAMES};
use differential::normalise::{apply_mode, diff, from_term, Canon, Mode};
use differential::runner::{corpus, cypher_rows, parse_view, sparql_rows};
use proptest::prelude::*;
use tiramemsu::ir::builder::IrBuilder;
use tiramemsu::ir::{Agg, Op};
use tiramemsu::*;

fn v(local: &str) -> Value {
    Value::iri(format!("urn:tiramemsu:v:{local}"))
}

/// The variant name of an error: what "the same error kind" compares.
fn kind(e: &str) -> String {
    e.split(|c: char| !c.is_alphanumeric())
        .find(|s| !s.is_empty())
        .unwrap_or("")
        .to_string()
}

/// SPARQL rows of `q` on `view`, canonical, or the error kind.
fn sparql_on(view: &View<'_>, q: &str) -> std::result::Result<Vec<Vec<Canon>>, String> {
    match view.sparql(q) {
        Ok(SparqlResult::Solutions(s)) => Ok(s
            .rows
            .iter()
            .map(|r| r.iter().map(from_term).collect())
            .collect()),
        Ok(SparqlResult::Boolean(b)) => Ok(vec![vec![Canon::Str(b.to_string())]]),
        Ok(other) => Ok(vec![vec![Canon::Str(format!("{other:?}"))]]),
        Err(e) => Err(kind(&format!("{e:?}"))),
    }
}

/// Opens a fresh database on `backend` in `dir`.
fn open(dir: &std::path::Path, backend: Backend, opts: OpenOptions) -> Db {
    Db::open(
        dir.join(format!("{backend:?}.db")),
        OpenOptions { backend, ..opts },
    )
    .unwrap()
}

/// SPARQL queries over the fixtures beyond the dialect pairs: optionals,
/// unions, negation, aggregates, modifiers, expressions, paths and time scopes.
const EXTRA: &[(&str, &str)] = &[
    ("employment", "SELECT ?p ?a WHERE { ?p a v:Person OPTIONAL { ?p v:age ?a } }"),
    ("employment", "SELECT ?x WHERE { { ?x a v:Employee } UNION { ?x v:locatedIn ?c } }"),
    ("employment", "SELECT ?p WHERE { ?p a v:Person MINUS { ?p v:worksAt v:acme } }"),
    ("employment", "SELECT ?p WHERE { ?p a v:Person FILTER NOT EXISTS { ?p v:age ?a } }"),
    ("employment", "SELECT ?p WHERE { ?p a v:Person FILTER EXISTS { ?p v:knows ?q } }"),
    ("employment", "SELECT ?c (COUNT(?p) AS ?n) WHERE { ?p v:worksAt ?c } GROUP BY ?c HAVING (COUNT(?p) > 1)"),
    ("employment", "SELECT (AVG(?a) AS ?m) (MIN(?a) AS ?lo) (MAX(?a) AS ?hi) (SUM(?a) AS ?s) WHERE { ?p v:age ?a }"),
    ("employment", "SELECT (GROUP_CONCAT(?n; separator=\",\") AS ?all) WHERE { SELECT ?n WHERE { ?p v:name ?n } ORDER BY ?n }"),
    ("employment", "SELECT ?n WHERE { ?p v:name ?n } ORDER BY DESC(?n) LIMIT 2 OFFSET 1"),
    ("employment", "SELECT DISTINCT ?c WHERE { ?p v:worksAt ?c }"),
    ("employment", "SELECT ?p ?y WHERE { ?p v:age ?a BIND(?a * 2 + 1 AS ?y) FILTER(?y > 55) }"),
    ("employment", "SELECT ?p WHERE { VALUES ?p { v:alice v:dave v:nobody } ?p a v:Person }"),
    ("employment", "SELECT ?n WHERE { ?p v:name ?n FILTER(REGEX(?n, \"^[AB]\") && STRLEN(?n) < 6) }"),
    ("employment", "SELECT ?u ?l WHERE { ?p v:name ?n BIND(UCASE(?n) AS ?u) BIND(SUBSTR(?n, 2, 2) AS ?l) }"),
    ("employment", "SELECT ?p ?city WHERE { ?p v:worksAt/v:locatedIn ?city }"),
    ("employment", "SELECT ?a ?b WHERE { ?a v:knows+ ?b }"),
    ("employment", "SELECT ?a WHERE { v:carol ^v:knows ?a }"),
    ("employment", "SELECT ?x WHERE { v:alice (v:knows|v:worksAt) ?x }"),
    ("employment", "ASK { v:alice v:knows/v:knows v:carol }"),
    ("employment", "SELECT ?p (IF(BOUND(?a), ?a, -1) AS ?age) WHERE { ?p a v:Person OPTIONAL { ?p v:age ?a } } ORDER BY ?age"),
    ("employment", "SELECT ?a ?b WHERE { ?a v:worksAt ?c . ?b v:worksAt ?c FILTER(?a != ?b) }"),
    ("employment", "SELECT ?p WHERE { ?p v:age ?a FILTER(?a IN (25, 41)) }"),
    ("paths", "SELECT ?a ?b WHERE { ?a v:knows ?b }"),
    ("paths", "SELECT ?b WHERE { v:alice v:knows* ?b }"),
    ("paths", "SELECT (COUNT(*) AS ?n) WHERE { ?a v:knows ?b . ?b v:knows ?c . ?c v:knows ?a }"),
    ("parallel", "SELECT ?n WHERE { v:alice v:nick ?n }"),
    ("parallel", "SELECT (COUNT(*) AS ?n) WHERE { v:alice v:called ?b }"),
    ("superseded", "SELECT ?c WHERE { v:alice v:worksAt ?c }"),
    ("cascade", "SELECT ?s ?p ?o WHERE { ?s ?p ?o }"),
    ("cardinality", "SELECT ?s ?p ?o WHERE { ?s ?p ?o }"),
    ("episodes", "SELECT ?s ?p ?o WHERE { ?s ?p ?o }"),
    ("literals", "SELECT ?s ?p ?o WHERE { ?s ?p ?o }"),
];

// @tg: verifies:: [[openspec:native-query-execution#Same answers as the SQL runner#Differential corpus on redb]] {since: "2026-10-27", level: integration}
#[test]
fn differential_corpus_on_redb() {
    let mut failures = Vec::new();
    let unordered = Mode {
        set: false,
        ordered: false,
    };
    for (fixture, q) in EXTRA {
        let (sql, kv) = (
            fixtures::load_on(fixture, Backend::Sqlite),
            fixtures::load_on(fixture, Backend::Redb),
        );
        assert_eq!(kv.db.backend(), Backend::Redb);
        let ordered = Mode {
            set: false,
            ordered: q.contains("ORDER BY"),
        };
        let mode = if ordered.ordered { ordered } else { unordered };
        let a = sparql_on(&sql.db.now(), q).map(|r| apply_mode(r, mode));
        let b = sparql_on(&kv.db.now(), q).map(|r| apply_mode(r, mode));
        if a != b {
            failures.push(format!("{fixture}: {q}\n  sqlite {a:?}\n  redb   {b:?}"));
        }
    }
    // every fixture under every whole-store view
    for name in NAMES {
        let (sql, kv) = (
            fixtures::load_on(name, Backend::Sqlite),
            fixtures::load_on(name, Backend::Redb),
        );
        let q = "SELECT ?s ?p ?o WHERE { ?s ?p ?o }";
        for (a, b) in [
            (sql.db.now(), kv.db.now()),
            (sql.db.history(), kv.db.history()),
        ] {
            let (a, b) = (
                sparql_on(&a, q).map(|r| apply_mode(r, unordered)),
                sparql_on(&b, q).map(|r| apply_mode(r, unordered)),
            );
            if a != b {
                failures.push(format!(
                    "{name}: whole store differs\n  sqlite {a:?}\n  redb   {b:?}"
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

// @tg: verifies:: [[openspec:dialect-differential-testing#Differential suite on both runners#Corpus on redb]] {since: "2026-10-27", level: integration}
#[test]
fn dialect_corpus_agrees_on_both_backends() {
    let mut failures = Vec::new();
    for p in corpus() {
        let mode = Mode {
            set: p.mode == "set",
            ordered: p.order == "ordered",
        };
        let (sql, kv) = (
            fixtures::load_on(&p.fixture, Backend::Sqlite),
            fixtures::load_on(&p.fixture, Backend::Redb),
        );
        let (vs, vk) = (parse_view(&sql.db, &p.view), parse_view(&kv.db, &p.view));
        for (lang, a, b) in [
            ("SPARQL", sparql_rows(&vs, &p), sparql_rows(&vk, &p)),
            ("Cypher", cypher_rows(&vs, &p), cypher_rows(&vk, &p)),
        ] {
            match (a, b) {
                (Ok(a), Ok(b)) => {
                    if let Some(d) = diff(&apply_mode(a, mode), &apply_mode(b, mode)) {
                        failures.push(format!("{} ({lang}): sqlite vs redb\n{d}", p.name));
                    }
                }
                (Err(a), Err(b))
                    if kind(&a.replace("SPARQL: ", "").replace("Cypher: ", ""))
                        == kind(&b.replace("SPARQL: ", "").replace("Cypher: ", "")) => {}
                (a, b) => failures.push(format!("{} ({lang}): {a:?} vs {b:?}", p.name)),
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// One write of a random history.
#[derive(Clone, Debug)]
enum W {
    Link(u8, u8, u8),
    Num(u8, u8, i64),
    Retract(usize),
}

fn history() -> impl Strategy<Value = Vec<Vec<W>>> {
    let w = prop_oneof![
        4 => (0u8..6, 0u8..3, 0u8..6).prop_map(|(s, p, o)| W::Link(s, p, o)),
        2 => (0u8..6, 0u8..2, -3i64..10).prop_map(|(s, p, n)| W::Num(s, p, n)),
        1 => (0usize..64).prop_map(W::Retract),
    ];
    prop::collection::vec(prop::collection::vec(w, 1..6), 1..5)
}

/// A random query: a basic graph pattern of one to three patterns over a small
/// vocabulary, with an optional filter, OPTIONAL, UNION or aggregate around it.
fn query() -> impl Strategy<Value = String> {
    let term = |vars: &'static [&'static str], nodes: u8| {
        prop_oneof![
            3 => prop::sample::select(vars).prop_map(|v| v.to_string()),
            1 => (0..nodes).prop_map(|n| format!("v:n{n}")),
        ]
    };
    let pat = (
        term(&["?a", "?b", "?c"], 6),
        0u8..3,
        term(&["?a", "?b", "?c"], 6),
    )
        .prop_map(|(s, p, o)| format!("{s} v:p{p} {o} ."));
    let bgp = prop::collection::vec(pat, 1..4).prop_map(|ps| ps.join(" "));
    (bgp.clone(), bgp, 0u8..6, -2i64..8, 0u8..2).prop_map(|(a, b, shape, k, q)| {
        let num = format!("?a v:q{q} ?x .");
        match shape {
            0 => format!("SELECT * WHERE {{ {a} }}"),
            1 => format!("SELECT * WHERE {{ {num} FILTER(?x > {k}) }}"),
            2 => format!("SELECT * WHERE {{ {a} OPTIONAL {{ {num} }} }}"),
            3 => format!("SELECT * WHERE {{ {{ {a} }} UNION {{ {b} }} }}"),
            4 => format!("SELECT ?a (COUNT(*) AS ?n) WHERE {{ {a} }} GROUP BY ?a"),
            _ => format!("SELECT DISTINCT ?a WHERE {{ {a} FILTER NOT EXISTS {{ ?a v:p0 ?a }} }}"),
        }
    })
}

fn replay(db: &Db, txs: &[Vec<W>]) {
    let mut eids: Vec<Eid> = Vec::new();
    for tx in txs {
        db.transact(TxOptions::default(), |t| {
            for w in tx {
                let made = match w {
                    W::Link(s, p, o) => Some(t.assert(
                        v(&format!("n{s}")),
                        v(&format!("p{p}")),
                        v(&format!("n{o}")),
                        Valid::ALWAYS,
                    )?),
                    W::Num(s, p, n) => Some(t.assert(
                        v(&format!("n{s}")),
                        v(&format!("q{p}")),
                        Value::Int(*n),
                        Valid::ALWAYS,
                    )?),
                    W::Retract(k) => {
                        if !eids.is_empty() {
                            t.retract(eids[k % eids.len()])?;
                        }
                        None
                    }
                };
                if let Some(a) = made {
                    eids.push(a.eid());
                }
            }
            Ok(())
        })
        .unwrap();
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

    // @tg: verifies:: [[openspec:native-query-execution#Same answers as the SQL runner#Random queries agree]] {since: "2026-10-27", level: unit}
    #[test]
    fn random_queries_agree(txs in history(), qs in prop::collection::vec(query(), 4)) {
        let dir = tempfile::tempdir().unwrap();
        let clock = Arc::new(ManualClock::new(1_700_000_000_000));
        let opts = || OpenOptions { clock: clock.clone(), ..OpenOptions::default() };
        let (sql, kv) = (open(dir.path(), Backend::Sqlite, opts()), open(dir.path(), Backend::Redb, opts()));
        replay(&sql, &txs);
        replay(&kv, &txs);
        let mode = Mode { set: false, ordered: false };
        for q in &qs {
            for (a, b) in [(sql.now(), kv.now()), (sql.history(), kv.history())] {
                let a = sparql_on(&a, q).map(|r| apply_mode(r, mode));
                let b = sparql_on(&b, q).map(|r| apply_mode(r, mode));
                prop_assert_eq!(a, b, "{}", q);
            }
        }
    }
}

fn plan_lines(db: &Db, q: &str) -> Vec<String> {
    db.now().explain_sparql(q).unwrap().query_plan
}

fn bulk(db: &Db, edges: impl IntoIterator<Item = (String, String, String)>) {
    let edges: Vec<_> = edges.into_iter().collect();
    for chunk in edges.chunks(5_000) {
        db.transact(TxOptions::default(), |tx| {
            for (s, p, o) in chunk {
                tx.assert(v(s), v(p), v(o), Valid::ALWAYS)?;
            }
            Ok(())
        })
        .unwrap();
    }
}

// The spec's ratio is 10 to 1 000 000; the test keeps the shape at 10 to 50 000.
// @tg: verifies:: [[openspec:native-query-execution#Cost-based join order#Rare predicate first]] {since: "2026-10-27", level: integration}
#[test]
fn rare_predicate_first() {
    let dir = tempfile::tempdir().unwrap();
    let db = open(dir.path(), Backend::Redb, OpenOptions::default());
    bulk(
        &db,
        (0..50_000)
            .map(|i| {
                (
                    format!("n{i}"),
                    "common".to_string(),
                    format!("n{}", (i * 7) % 50_000),
                )
            })
            .chain((0..10).map(|i| (format!("n{i}"), "rare".to_string(), format!("r{i}")))),
    );
    let lines = plan_lines(
        &db,
        "SELECT ?a ?b ?r WHERE { ?a v:common ?b . ?b v:rare ?r }",
    );
    let first = lines
        .iter()
        .find(|l| l.contains("index nested loop") || l.contains("hash join"))
        .unwrap();
    assert!(first.contains("v:rare>"), "{lines:#?}");
}

/// The first join step of an explain.
fn first_step(lines: &[String]) -> &String {
    lines
        .iter()
        .find(|l| l.contains("index nested loop") || l.contains("hash join"))
        .unwrap()
}

// WatDiv F1's shape: 20 sellers (the smallest pattern) at one end, 50 users in
// one city at the other; starting from the sellers walks every product and like.
// @tg: verifies:: [[openspec:native-query-execution#Cost-based join order#Selective constant before a small pattern]] {since: "2026-10-27", level: integration}
#[test]
fn selective_constant_before_small_pattern() {
    let dir = tempfile::tempdir().unwrap();
    let db = open(dir.path(), Backend::Redb, OpenOptions::default());
    let t = |s: String, p: &str, o: String| (s, p.to_string(), o);
    bulk(
        &db,
        (0..20)
            .map(|i| t(format!("s{i}"), "locatedIn", format!("c{}", i % 5)))
            .chain((0..2_000).map(|i| t(format!("p{i}"), "soldBy", format!("s{}", i % 20))))
            .chain((0..5_000).flat_map(|u| {
                (0..4).map(move |k| {
                    t(
                        format!("u{u}"),
                        "likes",
                        format!("p{}", (u * 7 + k) % 2_000),
                    )
                })
            }))
            .chain((0..5_000).map(|u| t(format!("u{u}"), "livesIn", format!("city{}", u % 100)))),
    );
    let q = "SELECT ?u ?p ?s ?c WHERE { ?u v:livesIn v:city1 . ?u v:likes ?p . \
             ?p v:soldBy ?s . ?s v:locatedIn ?c }";
    let lines = plan_lines(&db, q);
    assert!(first_step(&lines).contains("v:livesIn>"), "{lines:#?}");
    let rows = sparql_on(&db.now(), &format!("PREFIX v: <urn:tiramemsu:v:> {q}")).unwrap();
    assert_eq!(rows.len(), 200);
}

/// Opens the same statements on SQLite and redb.
fn both(dir: &std::path::Path, load: impl Fn(&Db)) -> (Db, Db) {
    let s = open(dir, Backend::Sqlite, OpenOptions::default());
    let r = open(dir, Backend::Redb, OpenOptions::default());
    load(&s);
    load(&r);
    (s, r)
}

fn sorted(view: &View<'_>, q: &str) -> Vec<Vec<Canon>> {
    let mut rows = sparql_on(view, &format!("PREFIX v: <urn:tiramemsu:v:> {q}")).unwrap();
    rows.sort_by_key(|r| format!("{r:?}"));
    rows
}

// @tg: verifies:: [[openspec:native-query-execution#Constant equalities seek#String equality becomes a seek]] {since: "2026-10-27", level: integration}
#[test]
fn string_equality_becomes_a_seek() {
    let dir = tempfile::tempdir().unwrap();
    let (s, r) = both(dir.path(), |db| {
        db.transact(TxOptions::default(), |tx| {
            for i in 0..2_000 {
                tx.assert(
                    v(&format!("n{i}")),
                    v("name"),
                    Value::str(format!("n{}", i % 500)),
                    Valid::ALWAYS,
                )?;
            }
            let tagged = Value::LangStr {
                lex: "n7".into(),
                lang: "en".into(),
            };
            tx.assert(v("x"), v("name"), tagged, Valid::ALWAYS)?;
            Ok(())
        })
        .unwrap();
    });
    let q = r#"SELECT ?x WHERE { ?x v:name ?n FILTER(?n = "n7") }"#;
    let lines = plan_lines(&r, q);
    assert!(
        lines.iter().any(|l| l.contains("bound n to constant")),
        "{lines:#?}"
    );
    assert!(
        first_step(&lines).contains("index nested loop")
            && first_step(&lines).contains(", 1 rows)"),
        "{lines:#?}"
    );
    let rows = sorted(&r.now(), q);
    assert_eq!(rows.len(), 4);
    assert_eq!(rows, sorted(&s.now(), q));
}

// @tg: verifies:: [[openspec:native-query-execution#Constant equalities seek#Numeric equality is not pinned]] {since: "2026-10-27", level: integration}
#[test]
fn numeric_equality_is_not_pinned() {
    let dir = tempfile::tempdir().unwrap();
    let (s, r) = both(dir.path(), |db| {
        db.transact(TxOptions::default(), |tx| {
            tx.assert(v("a"), v("age"), Value::Int(7), Valid::ALWAYS)?;
            tx.assert(v("b"), v("age"), Value::Double(7.0), Valid::ALWAYS)?;
            tx.assert(v("c"), v("age"), Value::Int(8), Valid::ALWAYS)?;
            Ok(())
        })
        .unwrap();
    });
    let q = "SELECT ?x WHERE { ?x v:age ?a FILTER(?a = 7) }";
    let lines = plan_lines(&r, q);
    assert!(
        !lines.iter().any(|l| l.contains("bound a to constant")),
        "{lines:#?}"
    );
    let rows = sorted(&r.now(), q);
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_eq!(rows, sorted(&s.now(), q));
}

#[path = "../examples/triangles.rs"]
mod harness;

fn triangle_count() -> IrQuery {
    let b = IrBuilder::sparql();
    let tri = Op::join(vec![
        b.triple("?a", "v:k", "?b"),
        b.triple("?b", "v:k", "?c"),
        b.triple("?c", "v:k", "?a"),
    ]);
    b.query(tri.aggregate(&[], vec![Agg::count_star("n")]))
}

fn load_edges(db: &Db, edges: &[(u64, u64)]) {
    bulk(
        db,
        edges
            .iter()
            .map(|(a, b)| (format!("n{a}"), "k".to_string(), format!("n{b}"))),
    );
}

// @tg: verifies:: [[openspec:native-query-execution#Worst-case-optimal cyclic joins#Skewed triangle]] {since: "2026-10-27", level: integration}
#[test]
fn skewed_triangle_is_bounded_by_agm() {
    let dir = tempfile::tempdir().unwrap();
    let mut edges = harness::skewed(6, 400, 2_000, &mut harness::Lcg(7));
    edges.sort_unstable();
    edges.dedup();
    let n = edges.len() as f64;
    let (sql, kv) = (
        open(dir.path(), Backend::Sqlite, OpenOptions::default()),
        open(dir.path(), Backend::Redb, OpenOptions::default()),
    );
    load_edges(&sql, &edges);
    load_edges(&kv, &edges);
    let q = triangle_count();
    let a = sql.now().execute_ir(&q, &Params::new()).unwrap();
    let b = kv.now().execute_ir(&q, &Params::new()).unwrap();
    assert_eq!(a.rows, b.rows);
    let ex = kv.now().explain_ir(&q, &Params::new()).unwrap();
    let visited: u64 = ex
        .query_plan
        .iter()
        .find_map(|l| {
            l.strip_prefix("generic join visited ")
                .and_then(|r| r.split(' ').next())
                .and_then(|k| k.parse().ok())
        })
        .unwrap_or_else(|| panic!("no generic join in {:#?}", ex.query_plan));
    // AGM bound of the triangle: |E|^1.5
    assert!(
        (visited as f64) <= 4.0 * n.powf(1.5),
        "{visited} keys for {n} edges"
    );
}

// @tg: verifies:: [[openspec:native-query-execution#Worst-case-optimal cyclic joins#Acyclic stays binary]] {since: "2026-10-27", level: integration}
#[test]
fn acyclic_stays_binary() {
    let dir = tempfile::tempdir().unwrap();
    let db = open(dir.path(), Backend::Redb, OpenOptions::default());
    load_edges(&db, &harness::uniform(300));
    for q in [
        "SELECT * WHERE { ?a v:k ?b . ?a v:k ?c . ?a v:k ?d }",
        "SELECT * WHERE { ?a v:k ?b . ?b v:k ?c . ?c v:k ?d }",
    ] {
        let lines = plan_lines(&db, q);
        assert!(
            !lines.iter().any(|l| l.contains("generic join")),
            "{q}: {lines:#?}"
        );
    }
}

// @tg: verifies:: [[openspec:native-query-execution#Late decoding#Count does not decode]] {since: "2026-10-27", level: integration}
#[test]
fn count_does_not_decode() {
    let dir = tempfile::tempdir().unwrap();
    let db = open(dir.path(), Backend::Redb, OpenOptions::default());
    load_edges(&db, &harness::uniform(200));
    let b = IrBuilder::sparql();
    let q = b.query(
        Op::join(vec![
            b.triple("?a", "v:k", "?b"),
            b.triple("?b", "v:k", "?c"),
        ])
        .aggregate(&[], vec![Agg::count_star("n")]),
    );
    let r = db.now().execute_ir(&q, &Params::new()).unwrap();
    assert_eq!(r.get(0, "n"), Some(&Value::Int(200 * 25)));
    assert_eq!(r.stats.dictionary_reads, 0);
    assert_eq!(r.stats.cache_hits, 0);
}

// @tg: verifies:: [[openspec:native-query-execution#Parallel scans#Same result with and without parallelism]] {since: "2026-10-27", level: integration}
#[test]
fn same_result_with_and_without_parallelism() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("par.redb");
    {
        let db = Db::create(&path, Backend::Redb, OpenOptions::default()).unwrap();
        load_edges(&db, &harness::uniform(3_000));
    }
    let q = "SELECT ?a ?c WHERE { ?a v:k ?b . ?b v:k ?c }";
    let run = |threshold: usize| {
        let db = Db::open(
            &path,
            OpenOptions {
                parallel_threshold: threshold,
                ..OpenOptions::default()
            },
        )
        .unwrap();
        let rows = sparql_on(&db.now(), q).unwrap();
        (rows, plan_lines(&db, q))
    };
    let (par, par_plan) = run(1);
    let (seq, seq_plan) = run(0);
    assert_eq!(par, seq, "same rows in the same order");
    assert_eq!(par.len(), 3_000 * 25);
    assert!(
        !seq_plan.iter().any(|l| l.contains("parallel")),
        "{seq_plan:#?}"
    );
    if thread::available_parallelism().map_or(1, |n| n.get()) > 1 {
        assert!(
            par_plan.iter().any(|l| l.contains("parallel morsels")),
            "{par_plan:#?}"
        );
    }
}

// @tg: verifies:: [[openspec:native-query-execution#Budgets on the native runner#Cancel a long native join]] {since: "2026-10-27", level: integration}
#[test]
fn cancel_a_long_native_join() {
    let dir = tempfile::tempdir().unwrap();
    let db = open(dir.path(), Backend::Redb, OpenOptions::default());
    let mut rng = harness::Lcg(3);
    load_edges(&db, &harness::layered(60, &mut rng));
    let token = CancelToken::new();
    let budget = QueryBudget {
        cancel: Some(token.clone()),
        ..QueryBudget::default()
    };
    let canceller = thread::spawn(move || {
        thread::sleep(Duration::from_millis(30));
        token.cancel();
    });
    // a cyclic core crossed with a disconnected pattern: far longer than 30 ms
    let r = db
        .now()
        .with_budget(&budget)
        .sparql("SELECT ?a ?b ?c ?x ?y WHERE { ?a v:k ?b . ?b v:k ?c . ?c v:k ?a . ?x v:k ?y }");
    canceller.join().unwrap();
    assert!(
        matches!(r, Err(Error::Cancelled)),
        "{:?}",
        r.map(|_| "a result")
    );
}

// @tg: verifies:: [[openspec:native-query-execution#Native explain#Explain names the permutation]] {since: "2026-10-27", level: integration}
#[test]
fn explain_names_the_permutation() {
    let dir = tempfile::tempdir().unwrap();
    let db = open(dir.path(), Backend::Redb, OpenOptions::default());
    bulk(
        &db,
        (0..20).map(|i| (format!("s{i}"), "p".to_string(), format!("o{}", i % 4))),
    );
    let ex = db
        .now()
        .explain_sparql("SELECT ?s WHERE { ?s v:p v:o1 }")
        .unwrap();
    assert_eq!(ex.regions[0].kind, RegionKind::Native);
    let scan = ex
        .query_plan
        .iter()
        .find(|l| l.contains("pos live scan"))
        .unwrap_or_else(|| panic!("{:#?}", ex.query_plan));
    assert!(scan.contains("estimate"), "{scan}");
}
