//! Datalog (OpenSpec change `add-datalog-engine`) through the facade: one test
//! per scenario of the change's delta specs, each on SQLite and, with the
//! `redb` feature, on redb, where every backend must give the same rows.
#![cfg(feature = "datalog")]

use std::sync::Arc;
use std::time::Duration;

use tiramemsu::*;

const D: i64 = 86_400_000;

fn v(s: &str) -> Value {
    Value::iri(format!("urn:tiramemsu:v:{s}"))
}

fn tv(s: &str) -> DatalogTerm {
    DatalogTerm::Value(v(s))
}

/// January 1 of `y`, epoch ms.
fn year(y: i64) -> i64 {
    // days from 1970 to y-01-01 (proleptic Gregorian)
    let days = |y: i64| {
        let y1 = y - 1;
        365 * (y1 - 1969) + (y1 / 4 - 1969 / 4) - (y1 / 100 - 1969 / 100) + (y1 / 400 - 1969 / 400)
    };
    days(y) * D
}

fn backends() -> Vec<Backend> {
    #[allow(unused_mut)]
    let mut out = vec![Backend::Sqlite];
    #[cfg(feature = "redb")]
    out.push(Backend::Redb);
    out
}

fn open(dir: &tempfile::TempDir, backend: Backend) -> Db {
    Db::open(
        dir.path().join(format!("d.{}", backend.name())),
        OpenOptions {
            backend,
            clock: Arc::new(ManualClock::new(1_000)),
            ..OpenOptions::default()
        },
    )
    .unwrap()
}

/// Runs `scenario` on a fresh database of every backend; the scenario returns
/// a rendering of what it observed, which must be equal on every backend.
fn each<R: std::fmt::Debug + PartialEq>(scenario: impl Fn(&Db) -> R) {
    let mut seen: Vec<(Backend, R)> = Vec::new();
    for b in backends() {
        let dir = tempfile::tempdir().unwrap();
        let db = open(&dir, b);
        let r = scenario(&db);
        seen.push((b, r));
    }
    for (b, r) in &seen[1..] {
        assert_eq!(r, &seen[0].1, "{} differs from {}", b.name(), seen[0].0.name());
    }
}

fn tx<R>(db: &Db, f: impl FnOnce(&mut Tx<'_>) -> Result<R>) -> R {
    let mut out = None;
    db.transact(TxOptions::default(), |t| {
        out = Some(f(t)?);
        Ok(())
    })
    .unwrap();
    out.unwrap()
}

fn assert_fact(db: &Db, s: &str, p: &str, o: Value, valid: Valid) -> Eid {
    tx(db, |t| Ok(t.assert(v(s), v(p), o, valid)?.eid()))
}

fn text(t: &str) -> DatalogProgram {
    DatalogProgram::Text(t.to_string())
}

fn opts() -> DatalogOptions {
    DatalogOptions::default()
}

fn goal(db: &Db, program: &str, g: &str) -> DatalogResult {
    db.now().datalog(&text(program), Some(g), &opts()).unwrap()
}

/// Rows as `(values, intervals)`.
fn rows(r: &DatalogResult) -> Vec<(Vec<String>, Vec<Valid>)> {
    r.rows
        .iter()
        .map(|row| (row.values.iter().map(|t| t.to_string()).collect(), row.valid.clone()))
        .collect()
}

const REPORTS: &str = "reports(X, Y) :- v:manager(X, Y).\nreports(X, Z) :- reports(X, Y), v:manager(Y, Z).\n";

fn chain(db: &Db) -> (Eid, Eid) {
    let e1 = assert_fact(db, "alice", "manager", v("bob"), Valid::ALWAYS);
    let e2 = assert_fact(db, "bob", "manager", v("carol"), Valid::ALWAYS);
    (e1, e2)
}

// ---------------------------------------------------------------- language

// @tg: verifies:: [[openspec:datalog-language#Program text#Rules goal and constraint parse]] {level: integration}
#[test]
fn rules_goal_and_constraint_parse() {
    each(|db| {
        let info = db
            .now()
            .program_info(&text(
                "@prefix ex: <http://example.org/> .\n\
                 reports(X, Y) :- ex:manager(X, Y).\n\
                 reports(X, Z) :- reports(X, Y), ex:manager(Y, Z).\n\
                 ?- reports(ex:alice, X).\n\
                 constraint no_cycle error :- reports(X, X).\n",
            ))
            .unwrap();
        assert_eq!(info.analysis.rules, 2);
        assert_eq!(info.analysis.goals, 1);
        assert_eq!(info.analysis.constraints, vec![("no_cycle".to_string(), Severity::Error)]);
    });
}

// @tg: verifies:: [[openspec:datalog-language#Program text#Syntax error has a position]] {level: integration}
#[test]
fn syntax_error_has_a_position() {
    each(|db| {
        let e = db.now().program_info(&text("reports(X, Y) :- v:manager(X, Y)")).unwrap_err();
        assert!(matches!(e, Error::DatalogParse { line: 1, column: 33, .. }), "{e:?}");
    });
}

// @tg: verifies:: [[openspec:datalog-language#Stored atoms#Parallel statements under set semantics]] {level: integration}
#[test]
fn parallel_statements_under_set_semantics() {
    each(|db| {
        assert_fact(db, "a", "knows", v("b"), Valid::between(year(2020), year(2021)));
        assert_fact(db, "a", "knows", v("b"), Valid::between(year(2022), year(2023)));
        let r = db.now().datalog(&text("?- v:knows(v:a, Y)."), None, &opts()).unwrap();
        assert_eq!(r.rows.len(), 1);
        assert_eq!(r.rows[0].values, vec![tv("b")]);
        assert_eq!(r.rows[0].valid.len(), 2);
        rows(&r)
    });
}

// @tg: verifies:: [[openspec:datalog-language#Stored atoms#Eid binding keeps parallel statements apart]] {level: integration}
#[test]
fn eid_binding_keeps_parallel_statements_apart() {
    each(|db| {
        assert_fact(db, "a", "knows", v("b"), Valid::between(year(2020), year(2021)));
        assert_fact(db, "a", "knows", v("b"), Valid::between(year(2022), year(2023)));
        let r = db.now().datalog(&text("?- v:knows(v:a, Y) @ E."), None, &opts()).unwrap();
        assert_eq!(r.rows.len(), 2);
        rows(&r)
    });
}

// @tg: verifies:: [[openspec:datalog-language#Stored atoms#Layers are ordinary atoms]] {level: integration}
#[test]
fn layers_are_ordinary_atoms() {
    each(|db| {
        let e1 = assert_fact(db, "alice", "worksAt", v("acme"), Valid::ALWAYS);
        tx(db, |t| t.assert(Value::Stmt(e1), v("confidence"), Value::Double(0.9), Valid::ALWAYS));
        let r = goal(db, "", "v:worksAt(X, C) @ E, v:confidence(E, K), K > 0.8");
        assert_eq!(r.columns, ["X", "C", "E", "K"]);
        assert_eq!(
            r.rows[0].values,
            vec![tv("alice"), tv("acme"), DatalogTerm::Value(Value::Stmt(e1)), DatalogTerm::Value(Value::Double(0.9))]
        );
        r.rows.len()
    });
}

// @tg: verifies:: [[openspec:datalog-language#Reserved atoms#Valid interval as data]] {level: integration}
#[test]
fn valid_interval_as_data() {
    each(|db| {
        assert_fact(db, "alice", "worksAt", v("acme"), Valid::between(year(2020), year(2022)));
        let r = goal(db, "", "v:worksAt(v:alice, C) @ E, valid(E, F, T)");
        let row = &r.rows[0].values;
        assert_eq!(row[2], DatalogTerm::Value(Value::DateTime { ms: year(2020), tz: Some(0) }));
        assert_eq!(row[3], DatalogTerm::Value(Value::DateTime { ms: year(2022), tz: Some(0) }));
        rows(&r)
    });
}

// @tg: verifies:: [[openspec:datalog-language#Reserved atoms#Reserved head refused]] {level: integration}
#[test]
fn reserved_head_refused() {
    each(|db| {
        let e = db
            .now()
            .program_info(&text("valid(X, Y, Z) :- v:a(X, Y), v:a(Y, Z)."))
            .unwrap_err();
        assert!(matches!(e, Error::DatalogReserved { .. }), "{e:?}");
    });
}

// @tg: verifies:: [[openspec:datalog-language#Builtins#Duration arithmetic]] {level: integration}
#[test]
fn duration_arithmetic() {
    each(|db| {
        let due = Value::DateTime { ms: year(2026) + 9 * D, tz: Some(0) };
        assert_fact(db, "task1", "due", due, Valid::ALWAYS);
        let r = goal(db, "", "v:due(T, D), E = D + 30d");
        assert_eq!(r.rows[0].values[2], DatalogTerm::Value(Value::DateTime { ms: year(2026) + 39 * D, tz: Some(0) }));
        rows(&r)
    });
}

// @tg: verifies:: [[openspec:datalog-language#Builtins#Wrong kind is false]] {level: integration}
#[test]
fn wrong_kind_is_false() {
    each(|db| {
        assert_fact(db, "a", "name", Value::str("Ada"), Valid::ALWAYS);
        let r = goal(db, "", "v:name(X, N), N > 3");
        assert!(r.rows.is_empty());
    });
}

const UNMANAGED: &str = "unmanaged(X) :- rdf:type(X, v:Employee), not v:manager(X, _).\n";

fn employee(db: &Db, name: &str) -> Eid {
    tx(db, |t| Ok(t.assert(v(name), Value::iri(vocab::RDF_TYPE), v("Employee"), Valid::ALWAYS)?.eid()))
}

// @tg: verifies:: [[openspec:datalog-language#Stratified negation and aggregates#Unmanaged employees]] {level: integration}
#[test]
fn unmanaged_employees() {
    each(|db| {
        employee(db, "bob");
        employee(db, "carol");
        assert_fact(db, "carol", "manager", v("dave"), Valid::ALWAYS);
        let r = goal(db, UNMANAGED, "unmanaged(X)");
        assert_eq!(r.rows.iter().map(|r| r.values.clone()).collect::<Vec<_>>(), vec![vec![tv("bob")]]);
        rows(&r)
    });
}

// @tg: verifies:: [[openspec:datalog-language#Stratified negation and aggregates#Negative cycle refused]] {level: integration}
#[test]
fn negative_cycle_refused() {
    each(|db| {
        let e = db
            .now()
            .program_info(&text("q(X) :- v:a(X, X).\np(X) :- q(X), not r(X).\nr(X) :- q(X), not p(X)."))
            .unwrap_err();
        match e {
            Error::DatalogUnstratifiable { message } => {
                assert!(message.contains('p') && message.contains('r'), "{message}")
            }
            other => panic!("{other:?}"),
        }
    });
}

// @tg: verifies:: [[openspec:datalog-language#Safety#Unbound head variable]] {level: integration}
#[test]
fn unbound_head_variable() {
    each(|db| {
        let e = db.now().program_info(&text("p(X, Y) :- v:q(X, X).")).unwrap_err();
        match e {
            Error::DatalogUnsafe { message, .. } => assert!(message.contains('Y'), "{message}"),
            other => panic!("{other:?}"),
        }
    });
}

// @tg: verifies:: [[openspec:datalog-language#Value invention#Every employee has a manager]] {level: integration}
#[test]
fn every_employee_has_a_manager() {
    each(|db| {
        employee(db, "bob");
        let r = db
            .now()
            .datalog_reason(
                &text("exists M: v:assignedManager(X, M) :- rdf:type(X, v:Employee), not v:manager(X, _)."),
                &opts(),
            )
            .unwrap();
        assert_eq!(r.conclusions.len(), 1);
        assert_eq!(r.conclusions[0].s, tv("bob"));
        assert!(matches!(r.conclusions[0].o, DatalogTerm::Invented(_)));
        r.conclusions.len()
    });
}

// @tg: verifies:: [[openspec:datalog-language#Value invention#Recursive invention needs a limit]] {level: integration}
#[test]
fn recursive_invention_needs_a_limit() {
    each(|db| {
        let e = db
            .now()
            .datalog(&text("n(0).\nn(Y) :- n(X), Y = X + 1."), Some("n(X)"), &opts())
            .unwrap_err();
        assert!(matches!(e, Error::DatalogUnboundedInvention { .. }), "{e:?}");
    });
}

// @tg: verifies:: [[openspec:datalog-language#Value invention#Invention fuse]] {level: integration}
#[test]
fn invention_fuse() {
    each(|db| {
        let mut o = opts();
        o.limits.max_new_values = Some(100);
        let r = db
            .now()
            .datalog(&text("n(0).\nn(Y) :- n(X), Y = X + 1."), Some("n(X)"), &o)
            .unwrap();
        assert_eq!(r.status, DatalogStatus::Incomplete { limits: vec!["NewValues".into()] });
        assert!(r.rows.len() <= 100, "{}", r.rows.len());
        r.rows.len()
    });
}

// @tg: verifies:: [[openspec:datalog-language#Termination analysis#Analysis of a recursive temporal program]] {level: integration}
#[test]
fn analysis_of_a_recursive_temporal_program() {
    each(|db| {
        let info = db
            .now()
            .program_info(&text(
                "contagious(X) :- v:sick(X, true).\ncontagious(Y) :- past[0d, 3d] contagious(X), v:contact(X, Y).\n",
            ))
            .unwrap();
        let a = info.analysis;
        assert_eq!(a.strata.len(), 1);
        assert!(a.strata[0].recursive && a.strata[0].monotonic);
        assert_eq!(a.predicates, vec![v("contact").to_string().trim_matches(['<', '>']).to_string(), "urn:tiramemsu:v:sick".to_string()]);
        assert!(a.metric_recursion);
    });
}

// ---------------------------------------------------------------- evaluation

// @tg: verifies:: [[openspec:datalog-evaluation#Goal queries over a view#Reporting chain]] {level: integration}
#[test]
fn reporting_chain() {
    each(|db| {
        chain(db);
        let before = db.history().triples(None, None, None).unwrap();
        let r = goal(db, REPORTS, "reports(v:alice, X)");
        assert_eq!(r.rows.iter().map(|r| r.values.clone()).collect::<Vec<_>>(), vec![vec![tv("bob")], vec![tv("carol")]]);
        assert_eq!(db.history().triples(None, None, None).unwrap(), before);
        rows(&r)
    });
}

// @tg: verifies:: [[openspec:datalog-evaluation#Goal queries over a view#Goal in the text]] {level: integration}
#[test]
fn goal_in_the_text() {
    each(|db| {
        chain(db);
        let r = db
            .now()
            .datalog(&text(&format!("{REPORTS}?- reports(v:alice, X).")), None, &opts())
            .unwrap();
        assert_eq!(r.rows.len(), 2);
        rows(&r)
    });
}

// @tg: verifies:: [[openspec:datalog-evaluation#Full reasoning over a view#Conclusions as statements]] {level: integration}
#[test]
fn conclusions_as_statements() {
    each(|db| {
        assert_fact(db, "alice", "worksAt", v("acme"), Valid::ALWAYS);
        assert_fact(db, "acme", "locatedIn", v("berlin"), Valid::ALWAYS);
        let r = db
            .now()
            .datalog_reason(&text("v:worksIn(P, L) :- v:worksAt(P, C), v:locatedIn(C, L)."), &opts())
            .unwrap();
        assert_eq!(r.conclusions.len(), 1);
        let c = &r.conclusions[0];
        assert_eq!((c.s.clone(), c.p.clone(), c.o.clone()), (tv("alice"), v("worksIn"), tv("berlin")));
        r.conclusions.len()
    });
}

// @tg: verifies:: [[openspec:datalog-evaluation#Scope and pruning#Unused predicates are not read]] {level: integration}
#[test]
fn unused_predicates_are_not_read() {
    each(|db| {
        tx(db, |t| {
            for i in 0..100 {
                t.assert(v(&format!("p{i}")), v("name"), Value::str(format!("n{i}")), Valid::ALWAYS)?;
            }
            Ok(())
        });
        chain(db);
        let r = goal(db, REPORTS, "reports(v:alice, X)");
        assert_eq!(r.predicates, vec!["urn:tiramemsu:v:manager".to_string()]);
        assert!(r.pruned);
    });
}

// @tg: verifies:: [[openspec:datalog-evaluation#Scope and pruning#Graph scope]] {level: integration}
#[test]
fn graph_scope() {
    each(|db| {
        let (e1, _) = (
            assert_fact(db, "a", "manager", v("b"), Valid::ALWAYS),
            assert_fact(db, "b", "manager", v("c"), Valid::ALWAYS),
        );
        tx(db, |t| t.add_to_graph(e1, v("g1"), AssertOpts::default()));
        let mut o = opts();
        o.scope = DatalogScope::Graphs(vec![v("g1")]);
        let r = db.now().datalog(&text(REPORTS), Some("reports(v:a, X)"), &o).unwrap();
        assert_eq!(r.rows.iter().map(|r| r.values.clone()).collect::<Vec<_>>(), vec![vec![tv("b")]]);
        rows(&r)
    });
}

fn random_graph(db: &Db, seed: u64, nodes: u64, edges: usize) {
    let mut x = seed;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    tx(db, |t| {
        for _ in 0..edges {
            let a = next() % nodes;
            let b = next() % nodes;
            t.assert(v(&format!("n{a}")), v("manager"), v(&format!("n{b}")), Valid::ALWAYS)?;
        }
        Ok(())
    });
}

// @tg: verifies:: [[openspec:datalog-evaluation#Magic sets preserve answers#Goal-directed equals full]] {level: integration}
#[test]
fn goal_directed_equals_full() {
    for seed in [3, 17, 99] {
        each(|db| {
            random_graph(db, seed, 60, 90);
            let mut full = opts();
            full.demand = false;
            let on = db.now().datalog(&text(REPORTS), Some("reports(v:n3, X)"), &opts()).unwrap();
            let off = db.now().datalog(&text(REPORTS), Some("reports(v:n3, X)"), &full).unwrap();
            assert_eq!(rows(&on), rows(&off));
            assert!(on.demand.is_some());
            assert!(on.derived < off.derived, "{} vs {}", on.derived, off.derived);
            rows(&on)
        });
    }
}

// @tg: verifies:: [[openspec:datalog-evaluation#Provenance on request#Proof of a chain]] {level: integration}
#[test]
fn proof_of_a_chain() {
    each(|db| {
        let (e1, e2) = chain(db);
        let mut o = opts();
        o.proof = true;
        let r = db.now().datalog(&text(REPORTS), Some("reports(v:alice, v:carol)"), &o).unwrap();
        assert_eq!(r.rows.len(), 1);
        let goal_proof = &r.rows[0].proof.as_ref().unwrap()[0].proof;
        // the goal rests on the derived reports(alice, carol)
        let DatalogPremise::Fact(i) = goal_proof.premises[0] else { panic!("{goal_proof:?}") };
        let p = r.facts[i].proof.as_ref().unwrap();
        assert_eq!(p.rule, Some(1));
        let DatalogPremise::Fact(j) = p.premises[0] else { panic!("{p:?}") };
        assert_eq!(r.facts[j].args, vec![tv("alice"), tv("bob")]);
        assert_eq!(r.facts[j].proof.as_ref().unwrap().premises, vec![DatalogPremise::Statement(e1)]);
        assert_eq!(p.premises[1], DatalogPremise::Statement(e2));
        let mut plain = opts();
        plain.proof = false;
        let q = db.now().datalog(&text(REPORTS), Some("reports(v:alice, v:carol)"), &plain).unwrap();
        assert_eq!(rows(&q), rows(&r));
        assert!(q.rows[0].proof.is_none());
    });
}

fn line(db: &Db, n: usize) {
    tx(db, |t| {
        for i in 0..n {
            t.assert(v(&format!("n{i}")), v("manager"), v(&format!("n{}", i + 1)), Valid::ALWAYS)?;
        }
        Ok(())
    });
}

// @tg: verifies:: [[openspec:datalog-evaluation#Limits and completion#Derived-fact limit]] {level: integration}
#[test]
fn derived_fact_limit() {
    each(|db| {
        line(db, 1000);
        let mut o = opts();
        o.limits.max_facts = Some(100);
        o.demand = false;
        let r = db.now().datalog(&text(REPORTS), Some("reports(X, Y)"), &o).unwrap();
        assert_eq!(r.status, DatalogStatus::Incomplete { limits: vec!["Facts".into()] });
        // every row is a true fact: X precedes Y on the line
        for row in &r.rows {
            let n = |t: &DatalogTerm| t.to_string().trim_start_matches("<urn:tiramemsu:v:n").trim_end_matches('>').parse::<u32>().unwrap();
            assert!(n(&row.values[0]) < n(&row.values[1]));
        }
        o.require_complete = true;
        let e = db.now().datalog(&text(REPORTS), Some("reports(X, Y)"), &o).unwrap_err();
        assert!(matches!(e, Error::DatalogIncomplete { .. }), "{e:?}");
    });
}

// @tg: verifies:: [[openspec:datalog-evaluation#Budgets and cancellation#Deadline stops a closure]] {level: integration}
#[test]
fn deadline_stops_a_closure() {
    each(|db| {
        line(db, 3000);
        let budget = QueryBudget {
            timeout: Some(Duration::from_millis(1)),
            ..Default::default()
        };
        let mut o = opts();
        o.demand = false;
        let e = db.now().with_budget(&budget).datalog(&text(REPORTS), Some("reports(X, Y)"), &o).unwrap_err();
        assert!(matches!(e, Error::DeadlineExceeded { .. }), "{e:?}");
    });
}

// @tg: verifies:: [[openspec:datalog-evaluation#Deterministic results#Backends agree]] {level: integration}
#[test]
fn backends_agree() {
    // `each` compares the rows of every backend
    each(|db| {
        random_graph(db, 7, 40, 70);
        let r = db.now().datalog(&text(REPORTS), Some("reports(X, Y)"), &opts()).unwrap();
        rows(&r)
    });
}

// @tg: verifies:: [[openspec:datalog-evaluation#Deterministic results#Parallel and sequential agree]] {level: integration}
#[test]
fn parallel_and_sequential_agree() {
    each(|db| {
        random_graph(db, 11, 300, 600);
        let mut seq = opts();
        seq.parallel_threshold = 0;
        seq.demand = false;
        let mut par = seq.clone();
        par.parallel_threshold = 1;
        let a = db.now().datalog(&text(REPORTS), Some("reports(X, Y)"), &seq).unwrap();
        let b = db.now().datalog(&text(REPORTS), Some("reports(X, Y)"), &par).unwrap();
        assert_eq!(rows(&a), rows(&b));
        rows(&a).len()
    });
}

// @tg: verifies:: [[openspec:datalog-evaluation#Explain a program run#Explain shows access choice]] {level: integration}
#[test]
fn explain_shows_access_choice() {
    each(|db| {
        chain(db);
        let e = db.now().explain_datalog(&text(REPORTS), Some("reports(v:alice, X)"), &opts()).unwrap();
        let plan = e.plan.join("\n");
        assert!(plan.contains("access <urn:tiramemsu:v:manager>"), "{plan}");
        assert!(plan.contains("iteration"), "{plan}");
        assert_eq!(e.result.rows.len(), 2);
    });
}

// ---------------------------------------------------------------- time

const WORKS_IN: &str = "v:worksIn(P, L) :- v:worksAt(P, C), v:locatedIn(C, L).";

fn works_in(db: &Db, a: Valid, b: Valid) -> Vec<(Value, Valid)> {
    assert_fact(db, "alice", "worksAt", v("acme"), a);
    assert_fact(db, "acme", "locatedIn", v("berlin"), b);
    db.now()
        .datalog_reason(&text(WORKS_IN), &opts())
        .unwrap()
        .conclusions
        .into_iter()
        .map(|c| (c.p, c.valid))
        .collect()
}

// @tg: verifies:: [[openspec:datalog-temporal#Pointwise valid time with intervals#Facts never simultaneously valid do not combine]] {level: integration}
#[test]
fn facts_never_simultaneously_valid_do_not_combine() {
    each(|db| {
        let c = works_in(db, Valid::between(year(2020), year(2022)), Valid::from(year(2024)));
        assert!(c.is_empty());
    });
}

// @tg: verifies:: [[openspec:datalog-temporal#Pointwise valid time with intervals#Overlap gives the intersection]] {level: integration}
#[test]
fn overlap_gives_the_intersection() {
    each(|db| {
        let c = works_in(db, Valid::between(year(2020), year(2025)), Valid::from(year(2024)));
        assert_eq!(c, vec![(v("worksIn"), Valid::between(year(2024), year(2025)))]);
        c
    });
}

// @tg: verifies:: [[openspec:datalog-temporal#Pointwise valid time with intervals#Two derivations coalesce]] {level: integration}
#[test]
fn two_derivations_coalesce() {
    each(|db| {
        assert_fact(db, "alice", "worksAt", v("acme"), Valid::between(year(2020), year(2023)));
        assert_fact(db, "alice", "worksAt", v("beta"), Valid::between(year(2023), year(2026)));
        assert_fact(db, "acme", "locatedIn", v("berlin"), Valid::ALWAYS);
        assert_fact(db, "beta", "locatedIn", v("berlin"), Valid::ALWAYS);
        let c = db.now().datalog_reason(&text(WORKS_IN), &opts()).unwrap().conclusions;
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].valid, Valid::between(year(2020), year(2026)));
        c.len()
    });
}

// @tg: verifies:: [[openspec:datalog-temporal#Pointwise valid time with intervals#Instant-by-instant oracle]] {level: integration}
#[test]
fn instant_by_instant_oracle() {
    // the oracle: an atemporal run over the statements valid at each instant
    let programs = [
        "p(X, Z) :- v:e(X, Y), v:e(Y, Z).\n?- p(X, Z).",
        "r(X, Y) :- v:e(X, Y).\nr(X, Z) :- r(X, Y), v:e(Y, Z).\n?- r(X, Y).",
        "n(X) :- v:e(X, _), not v:f(X, _).\n?- n(X).",
        "c(X, N) :- v:node(X, true), N = count : { v:e(X, _) }.\n?- c(X, N).",
    ];
    for seed in [1u64, 5, 9] {
        each(|db| {
            let mut x = seed;
            let mut next = |m: u64| {
                x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                (x >> 33) % m
            };
            tx(db, |t| {
                for i in 0..5 {
                    t.assert(v(&format!("n{i}")), v("node"), Value::Bool(true), Valid::ALWAYS)?;
                }
                for _ in 0..14 {
                    let (a, b) = (next(5), next(5));
                    let f = next(10) as i64;
                    let len = 1 + next(6) as i64;
                    let p = if next(4) == 0 { "f" } else { "e" };
                    t.assert(v(&format!("n{a}")), v(p), v(&format!("n{b}")), Valid::between(f, f + len))?;
                }
                Ok(())
            });
            let mut seen = Vec::new();
            for prog in programs {
                let r = db.now().datalog(&text(prog), None, &opts()).unwrap();
                for t in -1..17 {
                    let mut at = opts();
                    at.time = DatalogTime::Atemporal;
                    let snap = db.now().valid_at(t).datalog(&text(prog), None, &at).unwrap();
                    let want: Vec<Vec<DatalogTerm>> = snap.rows.iter().map(|r| r.values.clone()).collect();
                    let got: Vec<Vec<DatalogTerm>> = r
                        .rows
                        .iter()
                        .filter(|row| row.valid.iter().any(|iv| iv.contains(t)))
                        .map(|r| r.values.clone())
                        .collect();
                    assert_eq!(got, want, "{prog} at {t}");
                }
                seen.push(rows(&r));
            }
            seen
        });
    }
}

// @tg: verifies:: [[openspec:datalog-temporal#Metric temporal operators#Fever within the last three days]] {level: integration}
#[test]
fn fever_within_the_last_three_days() {
    each(|db| {
        assert_fact(db, "p", "fever", Value::Bool(true), Valid::between(10 * D, 11 * D));
        let r = goal(db, "alert(X) :- past[0d, 3d] v:fever(X, true).", "alert(X)");
        assert_eq!(r.rows[0].valid, vec![Valid::between(10 * D, 14 * D)]);
        rows(&r)
    });
}

// @tg: verifies:: [[openspec:datalog-temporal#Metric temporal operators#Always in the past]] {level: integration}
#[test]
fn always_in_the_past() {
    each(|db| {
        assert_fact(db, "p", "employed", Value::Bool(true), Valid::between(0, 100 * D));
        let r = goal(db, "tenured(X) :- always_past[0d, 30d] v:employed(X, true).", "tenured(X)");
        assert_eq!(r.rows[0].valid, vec![Valid::between(30 * D, 100 * D)]);
        rows(&r)
    });
}

// @tg: verifies:: [[openspec:datalog-temporal#Metric temporal operators#Since]] {level: integration}
#[test]
fn since() {
    each(|db| {
        assert_fact(db, "s1", "alarm", Value::Bool(true), Valid::between(5, 6));
        assert_fact(db, "s1", "unacked", Value::Bool(true), Valid::between(5, 20));
        let r = goal(
            db,
            "pending(S) :- v:unacked(S, true) since[0, inf] v:alarm(S, true).",
            "pending(S)",
        );
        assert_eq!(r.rows[0].valid, vec![Valid::between(5, 20)]);
        rows(&r)
    });
}

// @tg: verifies:: [[openspec:datalog-temporal#Always operators in heads#Holds for the next week]] {level: integration}
#[test]
fn holds_for_the_next_week() {
    each(|db| {
        assert_fact(db, "r1", "booked", Value::Bool(true), Valid::between(D, 2 * D));
        let r = goal(db, "always_future[0d, 7d] blocked(R) :- v:booked(R, true).", "blocked(R)");
        assert_eq!(r.rows[0].valid, vec![Valid::between(D, 9 * D)]);
        rows(&r)
    });
}

// @tg: verifies:: [[openspec:datalog-temporal#Always operators in heads#Sometime in a head is refused]] {level: integration}
#[test]
fn sometime_in_a_head_is_refused() {
    each(|db| {
        let e = db.now().program_info(&text("future[0d, 1d] p(X) :- v:a(X, X).")).unwrap_err();
        assert!(matches!(e, Error::DatalogNotHorn { .. }), "{e:?}");
    });
}

// @tg: verifies:: [[openspec:datalog-temporal#Allen relations#Job meets the next job]] {level: integration}
#[test]
fn job_meets_the_next_job() {
    each(|db| {
        assert_fact(db, "a", "job", v("x"), Valid::between(year(2020), year(2022)));
        assert_fact(db, "a", "job", v("y"), Valid::between(year(2022), year(2025)));
        // the records of statements never valid together: `stmt` holds always
        let m = goal(db, "", "stmt(E1, v:a, v:job, v:x), stmt(E2, v:a, v:job, v:y), meets(E1, E2)");
        let o = goal(db, "", "stmt(E1, v:a, v:job, v:x), stmt(E2, v:a, v:job, v:y), overlaps(E1, E2)");
        // as facts they are never valid at one instant
        let f = goal(db, "", "v:job(v:a, v:x) @ E1, v:job(v:a, v:y) @ E2, meets(E1, E2)");
        assert!(f.rows.is_empty());
        assert_eq!(m.rows.len(), 1);
        assert!(o.rows.is_empty());
    });
}

const CONTAGION: &str = "contagious(X) :- v:sick(X, true).\ncontagious(Y) :- past[0d, 3d] contagious(X), v:contact(X, Y).\n";

// @tg: verifies:: [[openspec:datalog-temporal#Windows and recursion through time#Contagion needs a window]] {level: integration}
#[test]
fn contagion_needs_a_window() {
    each(|db| {
        let e = db.now().datalog(&text(CONTAGION), Some("contagious(X)"), &opts()).unwrap_err();
        assert!(matches!(e, Error::DatalogUnboundedRecursion { .. }), "{e:?}");
    });
}

// @tg: verifies:: [[openspec:datalog-temporal#Windows and recursion through time#Contagion within a window]] {level: integration}
#[test]
fn contagion_within_a_window() {
    each(|db| {
        assert_fact(db, "a", "sick", Value::Bool(true), Valid::between(D, 2 * D));
        assert_fact(db, "a", "contact", v("b"), Valid::between(2 * D, 3 * D));
        assert_fact(db, "b", "contact", v("c"), Valid::between(3 * D, 4 * D));
        let mut o = opts();
        o.window = Some(Valid::between(0, 30 * D));
        let r = db.now().datalog(&text(CONTAGION), Some("contagious(X)"), &o).unwrap();
        let names: Vec<_> = r.rows.iter().map(|r| r.values[0].clone()).collect();
        assert_eq!(names, vec![tv("a"), tv("b"), tv("c")]);
        for row in &r.rows {
            assert!(row.valid.iter().all(|w| w.from.unwrap_or(-1) >= 0 && w.to.unwrap_or(i64::MAX) <= 30 * D));
        }
        rows(&r)
    });
}

// @tg: verifies:: [[openspec:datalog-temporal#Negation over time#Unmanaged until a manager arrives]] {level: integration}
#[test]
fn unmanaged_until_a_manager_arrives() {
    each(|db| {
        employee(db, "bob");
        assert_fact(db, "bob", "manager", v("carol"), Valid::from(year(2023)));
        let r = goal(db, UNMANAGED, "unmanaged(X)");
        assert_eq!(r.rows[0].valid, vec![Valid::until(year(2023))]);
        rows(&r)
    });
}

// @tg: verifies:: [[openspec:datalog-temporal#Aggregates over time#Headcount changes with hires]] {level: integration}
#[test]
fn headcount_changes_with_hires() {
    each(|db| {
        tx(db, |t| t.assert(v("acme"), Value::iri(vocab::RDF_TYPE), v("Company"), Valid::from(year(2020))));
        assert_fact(db, "acme", "employs", v("a"), Valid::from(year(2020)));
        assert_fact(db, "acme", "employs", v("b"), Valid::between(year(2022), year(2024)));
        let r = goal(
            db,
            "headcount(C, N) :- rdf:type(C, v:Company), N = count : { v:employs(C, _) }.",
            "headcount(C, N)",
        );
        let got: Vec<(DatalogTerm, Vec<Valid>)> = r.rows.iter().map(|r| (r.values[1].clone(), r.valid.clone())).collect();
        assert_eq!(
            got,
            vec![
                (
                    DatalogTerm::Value(Value::Int(1)),
                    vec![Valid::between(year(2020), year(2022)), Valid::from(year(2024))]
                ),
                (DatalogTerm::Value(Value::Int(2)), vec![Valid::between(year(2022), year(2024))]),
            ]
        );
        rows(&r)
    });
}

// @tg: verifies:: [[openspec:datalog-temporal#Transaction time and the pinned clock#As-of ignores later facts]] {level: integration}
#[test]
fn as_of_ignores_later_facts() {
    each(|db| {
        assert_fact(db, "x", "y", v("z"), Valid::ALWAYS);
        assert_fact(db, "alice", "manager", v("bob"), Valid::ALWAYS);
        let r = db
            .as_of(TimeRef::Tx(1))
            .datalog(&text(REPORTS), Some("reports(v:alice, X)"), &opts())
            .unwrap();
        assert!(r.rows.is_empty());
    });
}

// @tg: verifies:: [[openspec:datalog-temporal#Transaction time and the pinned clock#Reproducible now]] {level: integration}
#[test]
fn reproducible_now() {
    each(|db| {
        for i in 0..3 {
            assert_fact(db, &format!("x{i}"), "y", v("z"), Valid::ALWAYS);
        }
        let inst = db.as_of(TimeRef::Tx(3)).datalog(&text(""), Some("T = now()"), &opts()).unwrap();
        let again = db.as_of(TimeRef::Tx(3)).datalog(&text(""), Some("T = now()"), &opts()).unwrap();
        assert_eq!(rows(&inst), rows(&again));
        let tx3 = db.history().triples(None, None, None).unwrap()[2].t_add;
        let _ = tx3;
        rows(&inst)
    });
}

// @tg: verifies:: [[openspec:datalog-temporal#Atemporal mode#Atemporal combines disjoint facts]] {level: integration}
#[test]
fn atemporal_combines_disjoint_facts() {
    each(|db| {
        assert_fact(db, "alice", "worksAt", v("acme"), Valid::between(year(2020), year(2022)));
        assert_fact(db, "acme", "locatedIn", v("berlin"), Valid::from(year(2024)));
        let mut o = opts();
        o.time = DatalogTime::Atemporal;
        let c = db.now().datalog_reason(&text(WORKS_IN), &o).unwrap().conclusions;
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].valid, Valid::ALWAYS);
        let e = db
            .now()
            .datalog(&text("a(X) :- past[0d, 1d] v:fever(X, true)."), Some("a(X)"), &o)
            .unwrap_err();
        assert!(matches!(e, Error::DatalogTemporalOperatorInAtemporal { .. }), "{e:?}");
    });
}

// ---------------------------------------------------------------- programs

// @tg: verifies:: [[openspec:datalog-programs#Store a program#Store and run]] {level: integration}
#[test]
fn store_and_run() {
    each(|db| {
        chain(db);
        tx(db, |t| t.store_program(v("reporting"), REPORTS));
        let stored = db
            .now()
            .datalog(&DatalogProgram::Stored(v("reporting")), Some("reports(v:alice, X)"), &opts())
            .unwrap();
        assert_eq!(rows(&stored), rows(&goal(db, REPORTS, "reports(v:alice, X)")));
        assert!(stored.program_statement.is_some());
        rows(&stored)
    });
}

// @tg: verifies:: [[openspec:datalog-programs#Store a program#Same text is idempotent]] {level: integration}
#[test]
fn same_text_is_idempotent() {
    each(|db| {
        let a = tx(db, |t| t.store_program(v("reporting"), REPORTS));
        let n = db.history().triples(None, None, None).unwrap().len();
        let b = tx(db, |t| t.store_program(v("reporting"), REPORTS));
        assert_eq!(a, b);
        assert_eq!(db.history().triples(None, None, None).unwrap().len(), n);
    });
}

// @tg: verifies:: [[openspec:datalog-programs#Store a program#Invalid text is refused]] {level: integration}
#[test]
fn invalid_text_is_refused() {
    each(|db| {
        let r = db.transact(TxOptions::default(), |t| {
            t.store_program(v("bad"), "q(X) :- v:a(X, X).\np(X) :- q(X), not r(X).\nr(X) :- q(X), not p(X).")?;
            Ok(())
        });
        assert!(matches!(r, Err(Error::DatalogUnstratifiable { .. })), "{r:?}");
        assert!(db.history().triples(None, None, None).unwrap().is_empty());
    });
}

// @tg: verifies:: [[openspec:datalog-programs#Programs have transaction time#As-of uses the old program]] {level: integration}
#[test]
fn as_of_uses_the_old_program() {
    each(|db| {
        chain(db); // transactions 1 and 2
        tx(db, |t| t.store_program(v("reporting"), "reports(X, Y) :- v:manager(X, Y).")); // 3
        assert_fact(db, "x", "y", v("z"), Valid::ALWAYS); // 4
        tx(db, |t| t.store_program(v("reporting"), REPORTS)); // 5
        let old = db
            .as_of(TimeRef::Tx(4))
            .datalog(&DatalogProgram::Stored(v("reporting")), Some("reports(v:alice, X)"), &opts())
            .unwrap();
        assert_eq!(old.rows.iter().map(|r| r.values.clone()).collect::<Vec<_>>(), vec![vec![tv("bob")]]);
        let now = db
            .now()
            .datalog(&DatalogProgram::Stored(v("reporting")), Some("reports(v:alice, X)"), &opts())
            .unwrap();
        assert_eq!(now.rows.len(), 2);
    });
}

// @tg: verifies:: [[openspec:datalog-programs#Program information#Info of a stored program]] {level: integration}
#[test]
fn info_of_a_stored_program() {
    each(|db| {
        tx(db, |t| t.store_program(v("reporting"), REPORTS));
        let info = db.now().program_info(&DatalogProgram::Stored(v("reporting"))).unwrap();
        assert_eq!(info.analysis.rules, 2);
        let user: Vec<_> = info.analysis.strata.iter().filter(|s| s.relations == ["reports"]).collect();
        assert_eq!(user.len(), 1);
        assert!(user[0].recursive && user[0].monotonic);
        assert_eq!(info.analysis.predicates, vec!["urn:tiramemsu:v:manager".to_string()]);
        let e = db.now().program_info(&DatalogProgram::Stored(v("nothing"))).unwrap_err();
        assert!(matches!(e, Error::UnknownProgram { .. }), "{e:?}");
    });
}

// @tg: verifies:: [[openspec:datalog-programs#Rule language conflict#N3 node refused]] {level: integration}
#[test]
fn n3_node_refused() {
    each(|db| {
        tx(db, |t| t.assert(v("r"), Value::iri(vocab::N3_SOURCE), Value::str("{ ?x a v:A } => { ?x a v:B } ."), Valid::ALWAYS));
        let r = db.transact(TxOptions::default(), |t| {
            t.store_program(v("r"), REPORTS)?;
            Ok(())
        });
        assert!(matches!(r, Err(Error::RuleLanguageConflict { .. })), "{r:?}");
    });
}

#[cfg(feature = "n3")]
#[test]
fn datalog_node_refuses_n3() {
    each(|db| {
        tx(db, |t| t.store_program(v("r"), REPORTS));
        let r = db.transact(TxOptions::default(), |t| {
            t.store_rules(v("r"), "{ ?x a v:A } => { ?x a v:B } .")?;
            Ok(())
        });
        assert!(matches!(r, Err(Error::RuleLanguageConflict { .. })), "{r:?}");
    });
}

// ---------------------------------------------------------------- constraints

const POLICIES: &str = "constraint one_ceo error :- v:role(X, v:CEO), v:role(Y, v:CEO), X != Y.\n\
                        constraint owned warn :- rdf:type(P, v:Project), not v:owner(P, _).\n";

// @tg: verifies:: [[openspec:datalog-constraints#Declare constraints#Two constraints]] {level: integration}
#[test]
fn two_constraints() {
    each(|db| {
        let info = db.now().program_info(&text(POLICIES)).unwrap();
        assert_eq!(
            info.analysis.constraints,
            vec![("one_ceo".to_string(), Severity::Error), ("owned".to_string(), Severity::Warn)]
        );
    });
}

// @tg: verifies:: [[openspec:datalog-constraints#Declare constraints#Duplicate name]] {level: integration}
#[test]
fn duplicate_name() {
    each(|db| {
        let e = db
            .now()
            .program_info(&text("constraint one_ceo warn :- v:a(X, X).\nconstraint one_ceo error :- v:a(X, X)."))
            .unwrap_err();
        assert!(matches!(e, Error::DatalogDuplicateConstraint { .. }), "{e:?}");
    });
}

// @tg: verifies:: [[openspec:datalog-constraints#Check constraints#Two CEOs at once]] {level: integration}
#[test]
fn two_ceos_at_once() {
    each(|db| {
        let a = assert_fact(db, "a", "role", v("CEO"), Valid::between(year(2020), year(2024)));
        let b = assert_fact(db, "b", "role", v("CEO"), Valid::from(year(2023)));
        let r = db.now().check(&text(POLICIES), &["one_ceo".into()], &opts()).unwrap();
        assert_eq!(r.violated.len(), 1);
        let vs = &r.violated[0].violations;
        assert_eq!(vs.len(), 2);
        for viol in vs {
            assert_eq!(viol.valid, vec![Valid::between(year(2023), year(2024))]);
            assert_eq!(viol.eids, vec![a, b]);
        }
        assert_eq!(vs[0].bindings, vec![("X".into(), tv("a")), ("Y".into(), tv("b"))]);
        assert_eq!(vs[1].bindings, vec![("X".into(), tv("b")), ("Y".into(), tv("a"))]);
        vs.len()
    });
}

// @tg: verifies:: [[openspec:datalog-constraints#Check constraints#Successive CEOs are fine]] {level: integration}
#[test]
fn successive_ceos_are_fine() {
    each(|db| {
        assert_fact(db, "a", "role", v("CEO"), Valid::between(year(2020), year(2024)));
        assert_fact(db, "b", "role", v("CEO"), Valid::from(year(2024)));
        let r = db.now().check(&text(POLICIES), &["one_ceo".into()], &opts()).unwrap();
        assert!(r.violated.is_empty());
    });
}

// @tg: verifies:: [[openspec:datalog-constraints#Check constraints#Check as of the past]] {level: integration}
#[test]
fn check_as_of_the_past() {
    each(|db| {
        for i in 0..5 {
            assert_fact(db, &format!("x{i}"), "y", v("z"), Valid::ALWAYS);
        }
        assert_fact(db, "a", "role", v("CEO"), Valid::ALWAYS); // 6
        assert_fact(db, "b", "role", v("CEO"), Valid::ALWAYS); // 7
        assert_eq!(db.now().check(&text(POLICIES), &[], &opts()).unwrap().violated.len(), 1);
        let past = db.as_of(TimeRef::Tx(6)).check(&text(POLICIES), &[], &opts()).unwrap();
        assert!(past.violated.iter().all(|c| c.name != "one_ceo"));
    });
}
