//! Materialized Datalog programs (OpenSpec change `add-datalog-materialization`):
//! one test per scenario, on SQLite and redb, plus a grounding property.
#![cfg(feature = "datalog")]

use std::sync::Arc;

use tiramemsu::*;

const D: i64 = 86_400_000;

fn v(s: &str) -> Value {
    Value::iri(format!("urn:tiramemsu:v:{s}"))
}

fn year(y: i64) -> i64 {
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

fn each<R: std::fmt::Debug + PartialEq>(scenario: impl Fn(&Db) -> R) {
    let mut seen: Vec<(Backend, R)> = Vec::new();
    for b in backends() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(
            dir.path().join(format!("m.{}", b.name())),
            OpenOptions {
                backend: b,
                clock: Arc::new(ManualClock::new(1_000)),
                ..OpenOptions::default()
            },
        )
        .unwrap();
        seen.push((b, scenario(&db)));
    }
    for (b, r) in &seen[1..] {
        assert_eq!(
            r,
            &seen[0].1,
            "{} differs from {}",
            b.name(),
            seen[0].0.name()
        );
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

fn fact(db: &Db, s: &str, p: &str, o: Value, valid: Valid) -> Eid {
    tx(db, |t| Ok(t.assert(v(s), v(p), o, valid)?.eid()))
}

const REACH: &str = "v:reportsTo(X, Y) :- v:manager(X, Y).\nv:reportsTo(X, Z) :- v:reportsTo(X, Y), v:manager(Y, Z).\n";

fn store(db: &Db, name: &str, text: &str) {
    tx(db, |t| t.store_program(v(name), text));
}

fn run(db: &Db, name: &str) -> MaterializeProgramReport {
    db.materialize_program(&v(name), &MaterializeProgramOptions::default())
        .unwrap()
}

fn chain(db: &Db) -> (Eid, Eid) {
    (
        fact(db, "alice", "manager", v("bob"), Valid::ALWAYS),
        fact(db, "bob", "manager", v("carol"), Valid::ALWAYS),
    )
}

/// The live statements of predicate `p` as `(s, o, valid)` strings, sorted.
fn live_of(db: &Db, p: &str) -> Vec<(String, String, Valid)> {
    let view = db.now();
    let Some(pid) = view.encode(&v(p)).unwrap() else {
        return Vec::new();
    };
    let mut out: Vec<(String, String, Valid)> = view
        .triples(None, Some(pid), None)
        .unwrap()
        .iter()
        .map(|t| {
            (
                view.decode(t.s).unwrap().to_string(),
                view.decode(t.o).unwrap().to_string(),
                Valid {
                    from: t.v_from,
                    to: t.v_to,
                },
            )
        })
        .collect();
    out.sort_by(|a, b| (&a.0, &a.1, a.2.from).cmp(&(&b.0, &b.1, b.2.from)));
    out
}

/// The eids a statement was derived from.
fn support_of(db: &Db, d: Eid) -> Vec<Eid> {
    let view = db.now();
    let df = view
        .encode(&Value::iri(vocab::SYS_DERIVED_FROM))
        .unwrap()
        .unwrap();
    let mut out: Vec<Eid> = view
        .triples(Some(d.oid()), Some(df), None)
        .unwrap()
        .iter()
        .filter_map(|t| Eid::from_oid(t.o))
        .collect();
    out.sort();
    out
}

// @tg: verifies:: [[openspec:datalog-materialization#Materialize a program#Materialized chain]] {level: integration}
#[test]
fn materialized_chain() {
    each(|db| {
        let (_, e2) = chain(db);
        store(db, "reporting", REACH);
        let r = run(db, "reporting");
        assert_eq!(r.derived.len(), 3);
        assert_eq!(live_of(db, "reportsTo").len(), 3);
        // (alice reportsTo carol) rests on (alice reportsTo bob), e2 and R
        let view = db.now();
        let carol = view.encode(&v("carol")).unwrap().unwrap();
        let top = view
            .triples(None, view.encode(&v("reportsTo")).unwrap(), Some(carol))
            .unwrap();
        let alice = top
            .iter()
            .find(|t| view.decode(t.s).unwrap() == v("alice"))
            .unwrap();
        let sup = support_of(db, alice.eid);
        assert!(sup.contains(&e2) && sup.contains(&r.program_statement));
        assert_eq!(sup.len(), 3);
        // the labels
        let by = view
            .triples(
                Some(alice.eid.oid()),
                view.encode(&Value::iri(vocab::DL_DERIVED_BY)).unwrap(),
                None,
            )
            .unwrap();
        assert_eq!(by.len(), 1);
        live_of(db, "reportsTo")
    });
}

// @tg: verifies:: [[openspec:datalog-materialization#Materialize a program#Re-running is a no-op]] {level: integration}
#[test]
fn re_running_is_a_no_op() {
    each(|db| {
        chain(db);
        store(db, "reporting", REACH);
        let first = run(db, "reporting");
        let again = run(db, "reporting");
        assert_eq!(again.existing.len(), first.derived.len());
        assert!(again.derived.is_empty());
        // only the transaction's `dl:materialized` label is new
        assert_eq!(again.commit.unwrap().asserted.len(), 1);
    });
}

// @tg: verifies:: [[openspec:datalog-materialization#Materialize a program#Dry run]] {level: integration}
#[test]
fn dry_run() {
    each(|db| {
        chain(db);
        store(db, "reporting", REACH);
        let before = db.history().triples(None, None, None).unwrap().len();
        let r = db
            .materialize_program(
                &v("reporting"),
                &MaterializeProgramOptions {
                    dry_run: true,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(r.derived.len(), 3);
        assert!(r.dry_run);
        assert_eq!(
            db.history().triples(None, None, None).unwrap().len(),
            before
        );
    });
}

// @tg: verifies:: [[openspec:datalog-materialization#Materialize a program#Unstorable relation]] {level: integration}
#[test]
fn unstorable_relation() {
    each(|db| {
        fact(db, "a", "name", Value::str("Ada"), Valid::ALWAYS);
        fact(db, "a", "manager", v("b"), Valid::ALWAYS);
        store(
            db,
            "p",
            "reach(X, Y) :- v:manager(X, Y).\nv:labelled(N, X) :- v:name(X, N).\nv:knows(X, Y) :- reach(X, Y).\n",
        );
        let r = run(db, "p");
        // the plain relation is not stored and not unstorable; the literal subject is
        assert_eq!(r.unstorable.len(), 1);
        assert_eq!(r.unstorable[0].p, v("labelled"));
        assert_eq!(live_of(db, "knows").len(), 1);
        // `knows` rests on the statement behind `reach`, not on a stored `reach`
        let view = db.now();
        let k = view
            .triples(None, view.encode(&v("knows")).unwrap(), None)
            .unwrap()[0]
            .eid;
        assert_eq!(support_of(db, k).len(), 2);
    });
}

// @tg: verifies:: [[openspec:datalog-materialization#Conclusions fall with premises and programs#Retracting a premise]] {level: integration}
#[test]
fn retracting_a_premise() {
    each(|db| {
        let (_, e2) = chain(db);
        store(db, "reporting", REACH);
        run(db, "reporting");
        let r = db
            .transact(TxOptions::default(), |t| {
                t.retract(e2)?;
                Ok(())
            })
            .unwrap();
        assert!(
            r.retracted
                .iter()
                .filter(|(_, k)| *k == RetKind::Cascade)
                .count()
                >= 2
        );
        assert_eq!(live_of(db, "reportsTo").len(), 1);
    });
}

// @tg: verifies:: [[openspec:datalog-materialization#Conclusions fall with premises and programs#Editing the program]] {level: integration}
#[test]
fn editing_the_program() {
    each(|db| {
        chain(db);
        store(db, "reporting", REACH);
        run(db, "reporting");
        let r = db
            .transact(TxOptions::default(), |t| {
                t.store_program(v("reporting"), "v:reportsTo(X, Y) :- v:manager(X, Y).")?;
                Ok(())
            })
            .unwrap();
        assert!(r.retracted.iter().any(|(_, k)| *k == RetKind::Supersede));
        assert!(live_of(db, "reportsTo").is_empty());
        run(db, "reporting");
        assert_eq!(live_of(db, "reportsTo").len(), 2);
    });
}

// @tg: verifies:: [[openspec:datalog-materialization#Own conclusions are not input#Layered programs]] {level: integration}
#[test]
fn layered_programs() {
    each(|db| {
        chain(db);
        store(db, "a", REACH);
        store(
            db,
            "b",
            "v:senior(X, Y) :- v:reportsTo(X, Y), v:manager(Y, _).",
        );
        let a = run(db, "a");
        let b = run(db, "b");
        assert_eq!(b.derived.len(), 1, "alice reports to bob who manages carol");
        let s = b.derived[0];
        let sup = support_of(db, s);
        // B's conclusion rests on one of A's conclusions
        let a_all: Vec<Eid> = a.derived.clone();
        assert!(sup.iter().any(|e| a_all.contains(e)));
        let again = run(db, "a");
        assert!(again.derived.is_empty() && again.existing.len() == a.derived.len());
    });
}

const UNMANAGED: &str = "v:unmanaged(X, true) :- rdf:type(X, v:Employee), not v:manager(X, _).\n";

fn employee(db: &Db, who: &str) {
    tx(db, |t| {
        t.assert(
            v(who),
            Value::iri(vocab::RDF_TYPE),
            v("Employee"),
            Valid::ALWAYS,
        )?;
        Ok(())
    });
}

// @tg: verifies:: [[openspec:datalog-materialization#Reconcile non-monotonic strata#Unmanaged becomes managed]] {level: integration}
#[test]
fn unmanaged_becomes_managed() {
    each(|db| {
        employee(db, "bob");
        store(db, "p", UNMANAGED);
        let first = run(db, "p");
        assert_eq!(first.derived.len(), 1);
        assert!(!first.needs_rerun_after_writes.is_empty());
        fact(db, "bob", "manager", v("carol"), Valid::ALWAYS);
        let second = run(db, "p");
        assert_eq!(second.reconciled.len(), 1);
        assert!(live_of(db, "unmanaged").is_empty());
        let view = db.now();
        let t = Value::Tx(second.commit.as_ref().unwrap().t);
        let tid = view.encode(&t).unwrap().unwrap();
        let count = view
            .triples(
                Some(tid),
                view.encode(&Value::iri(vocab::DL_RECONCILED_COUNT))
                    .unwrap(),
                None,
            )
            .unwrap();
        assert_eq!(count.len(), 1);
        assert_eq!(view.decode(count[0].o).unwrap(), Value::Int(1));
        assert_eq!(
            second.needs_rerun_after_writes,
            vec!["urn:tiramemsu:v:unmanaged".to_string()]
        );
    });
}

// @tg: verifies:: [[openspec:datalog-materialization#Reconcile non-monotonic strata#Aggregates track headcount]] {level: integration}
#[test]
fn aggregates_track_headcount() {
    each(|db| {
        tx(db, |t| {
            t.assert(
                v("acme"),
                Value::iri(vocab::RDF_TYPE),
                v("Company"),
                Valid::ALWAYS,
            )?;
            Ok(())
        });
        fact(db, "acme", "employs", v("a"), Valid::ALWAYS);
        store(
            db,
            "p",
            "v:headcount(C, N) :- rdf:type(C, v:Company), N = count : { v:employs(C, _) }.",
        );
        run(db, "p");
        assert_eq!(
            live_of(db, "headcount")
                .iter()
                .map(|x| x.1.clone())
                .collect::<Vec<_>>(),
            vec![Value::Int(1).to_string()]
        );
        fact(db, "acme", "employs", v("b"), Valid::ALWAYS);
        run(db, "p");
        assert_eq!(
            live_of(db, "headcount")
                .iter()
                .map(|x| x.1.clone())
                .collect::<Vec<_>>(),
            vec![Value::Int(2).to_string()]
        );
    });
}

// @tg: verifies:: [[openspec:datalog-materialization#Reconcile non-monotonic strata#Monotonic strata are not reconciled]] {level: integration}
#[test]
fn monotonic_strata_are_not_reconciled() {
    each(|db| {
        let (_, e2) = chain(db);
        store(db, "reporting", REACH);
        run(db, "reporting");
        tx(db, |t| t.retract(e2).map(|_| ()));
        let r = run(db, "reporting");
        assert!(r.reconciled.is_empty());
        assert!(r.needs_rerun_after_writes.is_empty());
    });
}

// @tg: verifies:: [[openspec:datalog-materialization#Valid time of materialized conclusions#Temporal conclusion]] {level: integration}
#[test]
fn temporal_conclusion() {
    each(|db| {
        let e = fact(
            db,
            "p",
            "fever",
            Value::Bool(true),
            Valid::between(10 * D, 11 * D),
        );
        store(
            db,
            "p",
            "v:alert(X, true) :- past[0d, 3d] v:fever(X, true).",
        );
        let r = run(db, "p");
        let live = live_of(db, "alert");
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].2, Valid::between(10 * D, 14 * D));
        let sup = support_of(db, r.derived[0]);
        assert!(sup.contains(&e) && sup.contains(&r.program_statement));
    });
}

// @tg: verifies:: [[openspec:datalog-materialization#Valid time of materialized conclusions#Widening across windows]] {level: integration}
#[test]
fn widening_across_windows() {
    each(|db| {
        fact(
            db,
            "alice",
            "worksAt",
            v("acme"),
            Valid::between(year(2020), year(2024)),
        );
        fact(
            db,
            "acme",
            "locatedIn",
            v("berlin"),
            Valid::between(year(2020), year(2024)),
        );
        store(
            db,
            "p",
            "v:worksIn(P, L) :- v:worksAt(P, C), v:locatedIn(C, L).",
        );
        let first = db
            .materialize_program(
                &v("p"),
                &MaterializeProgramOptions {
                    window: Some(Valid::between(year(2020), year(2022))),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(
            live_of(db, "worksIn")[0].2,
            Valid::between(year(2020), year(2022))
        );
        let second = run(db, "p");
        assert_eq!(second.widened.len(), 1);
        assert_eq!(second.widened[0].0, first.derived[0]);
        assert_eq!(
            live_of(db, "worksIn")[0].2,
            Valid::between(year(2020), year(2024))
        );
    });
}

// @tg: verifies:: [[openspec:datalog-materialization#Valid time of materialized conclusions#Require complete]] {level: integration}
#[test]
fn require_complete() {
    each(|db| {
        chain(db);
        store(db, "reporting", REACH);
        let mut o = MaterializeProgramOptions {
            require_complete: true,
            ..Default::default()
        };
        o.limits.max_facts = Some(1);
        let r = db.materialize_program(&v("reporting"), &o);
        assert!(matches!(r, Err(Error::DatalogIncomplete { .. })), "{r:?}");
        assert!(live_of(db, "reportsTo").is_empty());
    });
}

// @tg: verifies:: [[openspec:datalog-materialization#Snapshot then commit#Premise retracted during evaluation]] {level: integration}
#[test]
fn premise_retracted_during_evaluation() {
    each(|db| {
        let (e1, _) = chain(db);
        fact(db, "dan", "manager", v("erin"), Valid::ALWAYS);
        store(db, "reporting", REACH);
        let r = db
            .materialize_program_between(
                &v("reporting"),
                &MaterializeProgramOptions::default(),
                |db| {
                    db.transact(TxOptions::default(), |t| {
                        t.retract(e1)?;
                        Ok(())
                    })
                    .map(|_| ())
                },
            )
            .unwrap();
        // the conclusions resting on alice's edge are skipped, dan's is written
        assert!(!r.stale_support.is_empty());
        let rest = live_of(db, "reportsTo");
        assert!(rest.iter().any(|x| x.0.contains("dan")));
        assert!(rest.iter().all(|x| !x.0.contains("alice")));
    });
}

// @tg: verifies:: [[openspec:datalog-materialization#Invented nodes#Stable invented manager]] {level: integration}
#[test]
fn stable_invented_manager() {
    each(|db| {
        employee(db, "bob");
        store(
            db,
            "p",
            "exists M: v:assignedManager(X, M) :- rdf:type(X, v:Employee), not v:manager(X, _).",
        );
        let first = run(db, "p");
        assert_eq!(first.derived.len(), 1);
        assert_eq!(first.skolems.len(), 1);
        let a = live_of(db, "assignedManager");
        let second = run(db, "p");
        assert!(second.derived.is_empty());
        assert_eq!(second.skolems, first.skolems);
        assert_eq!(live_of(db, "assignedManager"), a);
    });
}

mod props {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig { cases: 12, ..ProptestConfig::default() })]

        // grounding and agreement: after any retractions, a materialization of a
        // monotonic program leaves exactly what `datalog_reason` concludes, and every
        // live conclusion rests on live support
        #[test]
        fn live_conclusions_equal_reasoning(edges in proptest::collection::vec((0u8..6, 0u8..6), 1..14), kill in proptest::collection::vec(0usize..14, 0..5)) {
            let dir = tempfile::tempdir().unwrap();
            let db = Db::open(dir.path().join("p.db"), OpenOptions { clock: Arc::new(ManualClock::new(1_000)), ..OpenOptions::default() }).unwrap();
            let mut eids = Vec::new();
            for (a, b) in &edges {
                eids.push(fact(&db, &format!("n{a}"), "manager", v(&format!("n{b}")), Valid::ALWAYS));
            }
            store(&db, "reporting", REACH);
            run(&db, "reporting");
            for k in kill {
                if let Some(e) = eids.get(k) {
                    let _ = db.transact(TxOptions::default(), |t| { t.retract(*e)?; Ok(()) });
                }
            }
            // every live conclusion's support is live
            let view = db.now();
            let df = view.encode(&Value::iri(vocab::SYS_DERIVED_FROM)).unwrap();
            if let Some(df) = df {
                for l in view.triples(None, Some(df), None).unwrap() {
                    let target = Eid::from_oid(l.o).unwrap();
                    prop_assert!(view.triples(None, None, None).unwrap().iter().any(|t| t.eid == target));
                }
            }
            run(&db, "reporting");
            let want: Vec<(String, String)> = {
                let mut x: Vec<_> = db.now().datalog_reason(&DatalogProgram::Text(REACH.into()), &DatalogOptions { ..Default::default() }).unwrap()
                    .conclusions.iter().map(|c| (c.s.to_string(), c.o.to_string())).collect();
                x.sort();
                x
            };
            // the reasoned view sees the materialized statements as stored, so compare
            // against a fresh database holding only the surviving edges
            let fresh = tempfile::tempdir().unwrap();
            let db2 = Db::open(fresh.path().join("q.db"), OpenOptions::default()).unwrap();
            for t in db.now().triples(None, view.encode(&v("manager")).unwrap(), None).unwrap() {
                let (s, o) = (view.decode(t.s).unwrap(), view.decode(t.o).unwrap());
                tx(&db2, |tx| tx.assert(s, v("manager"), o, Valid::ALWAYS).map(|_| ()));
            }
            let mut fresh_want: Vec<(String, String)> = db2.now().datalog_reason(&DatalogProgram::Text(REACH.into()), &DatalogOptions::default()).unwrap()
                .conclusions.iter().map(|c| (c.s.to_string(), c.o.to_string())).filter(|(_, _)| true).collect();
            fresh_want.sort();
            let live: Vec<(String, String)> = live_of(&db, "reportsTo").into_iter().map(|x| (x.0, x.1)).collect();
            let mut live_sorted: Vec<(String, String)> = live;
            live_sorted.sort();
            live_sorted.dedup();
            // the manager edges are concluded too (the program defines reportsTo only)
            let reports_only: Vec<(String, String)> = fresh_want.into_iter().collect();
            prop_assert_eq!(live_sorted, reports_only);
            let _ = want;
        }
    }
}
