//! Derived beliefs (OpenSpec change `add-derived-beliefs`) through the facade:
//! `Tx::derive`, `Tx::upsert_derived`, the support cascade, supersede, dependents
//! and bundles. Every scenario runs on SQLite and, with the `redb` feature, on
//! redb, and both must end with the same rows.

use std::collections::BTreeSet;
use std::sync::Arc;

use tiramemsu::*;

fn v(s: &str) -> Value {
    Value::iri(format!("urn:tiramemsu:v:{s}"))
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

/// Runs `scenario` on a fresh database of every backend and checks that all of
/// them end with the same statement table.
fn each(scenario: impl Fn(&Db)) {
    let mut tables = Vec::new();
    for b in backends() {
        let dir = tempfile::tempdir().unwrap();
        let db = open(&dir, b);
        scenario(&db);
        tables.push((b, db.history().triples(None, None, None).unwrap()));
    }
    for (b, t) in &tables[1..] {
        assert_eq!(
            t,
            &tables[0].1,
            "{} differs from {}",
            b.name(),
            tables[0].0.name()
        );
    }
}

/// One transaction; returns its report and what the body returned.
fn run<R>(db: &Db, f: impl FnOnce(&mut Tx<'_>) -> Result<R>) -> Result<(TxReport, R)> {
    let mut out = None;
    let report = db.transact(TxOptions::default(), |tx| {
        out = Some(f(tx)?);
        Ok(())
    })?;
    Ok((report, out.expect("the body ran")))
}

fn assert_one(db: &Db, s: &str, p: &str, o: Value) -> Eid {
    run(db, |tx| Ok(tx.assert(v(s), v(p), o, Valid::ALWAYS)?.eid()))
        .unwrap()
        .1
}

fn derive(db: &Db, s: &str, p: &str, o: Value, support: &[Eid]) -> Asserted {
    run(db, |tx| tx.derive(v(s), v(p), o, Valid::ALWAYS, support))
        .unwrap()
        .1
}

fn retract(db: &Db, e: Eid) -> TxReport {
    db.transact(TxOptions::default(), |tx| tx.retract(e).map(|_| ()))
        .unwrap()
}

/// The supports of `d` through its live links, with the link eids.
fn links(db: &Db, d: Eid) -> Vec<(Eid, Eid)> {
    let now = db.now();
    let Some(df) = now.encode(&Value::iri(vocab::SYS_DERIVED_FROM)).unwrap() else {
        return Vec::new();
    };
    now.triples(Some(d.oid()), Some(df), None)
        .unwrap()
        .iter()
        .map(|t| (Eid::from_oid(t.o).expect("a statement"), t.eid))
        .collect()
}

fn live(db: &Db, e: Eid) -> bool {
    db.now().dependents(e).unwrap().first() == Some(&e)
}

/// The eids a report retracted, with their kinds.
fn retracted(r: &TxReport) -> BTreeSet<(Eid, RetKind)> {
    r.retracted.iter().copied().collect()
}

// @tg: verifies:: [[openspec:derived-beliefs#Derive a belief with support#Summary with two supports]] {since: "2026-10-28", level: integration}
#[test]
fn summary_with_two_supports() {
    each(|db| {
        let e1 = assert_one(db, "alice", "ordered", v("tea"));
        let e2 = assert_one(db, "alice", "ordered", v("tea2"));
        let (r, d) = run(db, |tx| {
            tx.derive(v("alice"), v("prefers"), v("tea"), Valid::ALWAYS, &[e2, e1])
        })
        .unwrap();
        assert!(d.is_new());
        let d = d.eid();
        let ls = links(db, d);
        assert_eq!(ls.iter().map(|(e, _)| *e).collect::<Vec<_>>(), [e1, e2]);
        // the belief, then its links in support order
        assert_eq!(r.asserted, [d, ls[0].1, ls[1].1]);
    });
}

// @tg: verifies:: [[openspec:derived-beliefs#Derive a belief with support#Retracted support refused]] {since: "2026-10-28", level: integration}
#[test]
fn retracted_or_missing_support_is_refused() {
    each(|db| {
        let e1 = assert_one(db, "alice", "ordered", v("tea"));
        retract(db, e1);
        let before = db.history().triples(None, None, None).unwrap();
        let err = run(db, |tx| {
            tx.derive(v("alice"), v("prefers"), v("tea"), Valid::ALWAYS, &[e1])
        })
        .unwrap_err();
        assert!(matches!(err, Error::NotLive(e) if e == e1), "{err}");
        let err = run(db, |tx| {
            tx.derive(v("alice"), v("prefers"), v("tea"), Valid::ALWAYS, &[])
        })
        .unwrap_err();
        assert!(matches!(err, Error::InvalidTerm { .. }), "{err}");
        // a failed derive leaves no trace
        assert_eq!(db.history().triples(None, None, None).unwrap(), before);
    });
}

// @tg: verifies:: [[openspec:derived-beliefs#Derive a belief with support#Direct link refused]] {since: "2026-10-28", level: integration}
#[test]
fn direct_support_link_is_refused() {
    each(|db| {
        let e1 = assert_one(db, "alice", "ordered", v("tea"));
        let d = derive(db, "alice", "prefers", v("tea"), &[e1]).eid();
        let link = Value::iri(vocab::SYS_DERIVED_FROM);
        for create in [false, true] {
            let err = run(db, |tx| {
                if create {
                    tx.create(d, link.clone(), e1, Valid::ALWAYS).map(|_| ())
                } else {
                    tx.assert(d, link.clone(), e1, Valid::ALWAYS).map(|_| ())
                }
            })
            .unwrap_err();
            assert!(
                matches!(&err, Error::ReservedNamespace(p) if p == vocab::SYS_DERIVED_FROM),
                "{err}"
            );
        }
        // nor can a link be corrected
        let l = links(db, d)[0].1;
        let err = run(db, |tx| tx.supersede(l, Patch::object(v("x")))).unwrap_err();
        assert!(matches!(err, Error::ReservedNamespace(_)), "{err}");
    });
}

// @tg: verifies:: [[openspec:derived-beliefs#Derivation idempotency#Re-deriving is a no-op]] {since: "2026-10-28", level: integration}
// @tg: verifies:: [[tests#Derived Beliefs#Derivations Are Idempotent Per Support Set]] {since: "2026-10-28", level: integration}
#[test]
fn re_deriving_is_a_no_op() {
    each(|db| {
        let e1 = assert_one(db, "alice", "ordered", v("tea"));
        let e2 = assert_one(db, "alice", "ordered", v("tea2"));
        let d = derive(db, "alice", "prefers", v("tea"), &[e1, e2]).eid();
        let (r, again) = run(db, |tx| {
            tx.derive(
                v("alice"),
                v("prefers"),
                v("tea"),
                Valid::ALWAYS,
                &[e2, e1, e2],
            )
        })
        .unwrap();
        assert_eq!(again, Asserted::Existing(d));
        assert!(r.asserted.is_empty());
        assert_eq!(r.existing, [d]);
        assert_eq!(links(db, d).len(), 2);
    });
}

// @tg: verifies:: [[openspec:derived-beliefs#Derivation idempotency#Independent fact stays independent]] {since: "2026-10-28", level: integration}
#[test]
fn an_independent_fact_stays_independent() {
    each(|db| {
        let e9 = assert_one(db, "alice", "prefers", v("tea"));
        let e1 = assert_one(db, "alice", "ordered", v("tea"));
        let d = derive(db, "alice", "prefers", v("tea"), &[e1]);
        assert!(d.is_new());
        assert_ne!(d.eid(), e9);
        assert!(links(db, e9).is_empty());
        let r = retract(db, e1);
        assert!(retracted(&r).contains(&(d.eid(), RetKind::Cascade)));
        assert!(live(db, e9));
        assert!(!live(db, d.eid()));
    });
}

// @tg: verifies:: [[openspec:derived-beliefs#Support cascade#Losing one support]] {since: "2026-10-28", level: integration}
// @tg: verifies:: [[tests#Derived Beliefs#Support Retraction Retracts The Belief]] {since: "2026-10-28", level: integration}
#[test]
fn losing_one_support_retracts_the_belief_and_its_layers() {
    each(|db| {
        let e1 = assert_one(db, "alice", "ordered", v("tea"));
        let e2 = assert_one(db, "alice", "ordered", v("tea2"));
        let d = derive(db, "alice", "prefers", v("tea"), &[e1, e2]).eid();
        let conf = run(db, |tx| {
            Ok(tx
                .assert(d, v("confidence"), Value::Double(0.7), Valid::ALWAYS)?
                .eid())
        })
        .unwrap()
        .1;
        let ls = links(db, d);
        let r = retract(db, e1);
        let mut want = BTreeSet::from([(e1, RetKind::Explicit)]);
        for e in [ls[0].1, ls[1].1, d, conf] {
            want.insert((e, RetKind::Cascade));
        }
        assert_eq!(retracted(&r), want);
        assert!(live(db, e2));
    });
}

// @tg: verifies:: [[openspec:derived-beliefs#Support cascade#Parallel derivations]] {since: "2026-10-28", level: integration}
#[test]
fn parallel_derivations_are_disjunctive() {
    each(|db| {
        let e1 = assert_one(db, "alice", "ordered", v("tea"));
        let e3 = assert_one(db, "alice", "said", Value::str("I like tea"));
        let d1 = derive(db, "alice", "prefers", v("tea"), &[e1]).eid();
        let d2 = derive(db, "alice", "prefers", v("tea"), &[e3]);
        assert!(d2.is_new());
        assert_ne!(d1, d2.eid());
        retract(db, e1);
        assert!(!live(db, d1));
        assert!(live(db, d2.eid()));
        #[cfg(feature = "sparql")]
        assert!(matches!(
            db.now().sparql("ASK { v:alice v:prefers v:tea }").unwrap(),
            SparqlResult::Boolean(true)
        ));
    });
}

// @tg: verifies:: [[openspec:derived-beliefs#Support cascade#Stale saved answer]] {since: "2026-10-28", level: integration}
// @tg: verifies:: [[tests#Derived Beliefs#Derived Answers Go Stale]] {since: "2026-10-28", level: integration}
#[test]
#[cfg(all(feature = "sparql", feature = "cypher"))]
fn a_saved_answer_citing_a_derived_belief_goes_stale() {
    // saved answers are SQL records: SQLite files only
    let dir = tempfile::tempdir().unwrap();
    let db = &open(&dir, Backend::Sqlite);
    {
        let e1 = assert_one(db, "alice", "ordered", v("tea"));
        let d = derive(db, "alice", "prefers", v("tea"), &[e1]).eid();
        let a = db
            .save_answer(
                "taste",
                &SavedQuery::sparql("SELECT ?o WHERE { v:alice v:prefers ?o }"),
            )
            .unwrap();
        assert_eq!(a.dependencies, [d]);
        retract(db, e1);
        let marks = db.check_saved_answers().unwrap();
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].status, AnswerStatus::Stale);
        assert_eq!(marks[0].cause, InvalidationCause::SupportRetracted);
        let ev = marks[0].event.unwrap();
        assert_eq!(
            (ev.eid, ev.op, ev.kind),
            (d, Op::Retract, Some(RetKind::Cascade))
        );
    }
}

// @tg: verifies:: [[openspec:derived-beliefs#Supersede and derived statements#Correcting evidence retracts the summary]] {since: "2026-10-28", level: integration}
// @tg: verifies:: [[tests#Derived Beliefs#Supersede Drops Derived Beliefs]] {since: "2026-10-28", level: integration}
#[test]
fn correcting_evidence_retracts_the_summary() {
    each(|db| {
        let e1 = assert_one(db, "alice", "ordered", v("tea"));
        let d = derive(db, "alice", "prefers", v("tea"), &[e1]).eid();
        let note = run(db, |tx| {
            Ok(tx
                .assert(d, v("note"), Value::str("from orders"), Valid::ALWAYS)?
                .eid())
        })
        .unwrap()
        .1;
        let l = links(db, d)[0].1;
        let r = db
            .transact(TxOptions::default(), |tx| {
                tx.supersede(e1, Patch::object(v("coffee"))).map(|_| ())
            })
            .unwrap();
        // the evidence is replayed, the summary and its layers are not
        assert_eq!(r.superseded.len(), 1);
        let (old, new) = r.superseded[0];
        assert_eq!(old, e1);
        assert!(live(db, new));
        let want: BTreeSet<_> = [e1, l, d, note]
            .into_iter()
            .map(|e| (e, RetKind::Supersede))
            .collect();
        assert_eq!(retracted(&r), want);
        let now = db.now();
        let (s, p, o) = (
            now.encode(&v("alice")).unwrap(),
            now.encode(&v("prefers")).unwrap(),
            now.encode(&v("tea")).unwrap(),
        );
        assert!(now.triples(s, p, o).unwrap().is_empty());
    });
}

// @tg: verifies:: [[openspec:derived-beliefs#Supersede and derived statements#Correcting the summary keeps its support]] {since: "2026-10-28", level: integration}
#[test]
fn correcting_the_summary_keeps_its_support() {
    each(|db| {
        let e1 = assert_one(db, "alice", "ordered", v("tea"));
        let d = derive(db, "alice", "prefers", v("tea"), &[e1]).eid();
        let r = db
            .transact(TxOptions::default(), |tx| {
                let patch = Patch {
                    v_from: Some(Some(10)),
                    ..Patch::default()
                };
                tx.supersede(d, patch).map(|_| ())
            })
            .unwrap();
        let new = r.superseded.iter().find(|(o, _)| *o == d).unwrap().1;
        assert!(live(db, new));
        assert_eq!(
            links(db, new).iter().map(|(e, _)| *e).collect::<Vec<_>>(),
            [e1]
        );
        // the new belief still falls with its support
        let r = retract(db, e1);
        assert!(retracted(&r).contains(&(new, RetKind::Cascade)));
    });
}

// @tg: verifies:: [[openspec:derived-beliefs#Widening a derivation#Wider interval keeps support]] {since: "2026-10-28", level: integration}
#[test]
fn widening_keeps_support() {
    each(|db| {
        let e1 = assert_one(db, "alice", "badge", Value::str("berlin office"));
        let rule = assert_one(db, "rules", "has", Value::str("badge implies works in"));
        let (_, d) = run(db, |tx| {
            tx.derive(
                v("alice"),
                v("worksIn"),
                v("berlin"),
                Valid::between(2020, 2022),
                &[e1, rule],
            )
        })
        .unwrap();
        let d = d.eid();
        let r = db
            .transact(TxOptions::default(), |tx| {
                let patch = Patch {
                    v_to: Some(Some(2024)),
                    ..Patch::default()
                };
                tx.supersede(d, patch).map(|_| ())
            })
            .unwrap();
        let new = r.superseded.iter().find(|(o, _)| *o == d).unwrap().1;
        assert!(retracted(&r).contains(&(d, RetKind::Supersede)));
        let row = db
            .now()
            .triples(None, None, None)
            .unwrap()
            .into_iter()
            .find(|t| t.eid == new)
            .unwrap();
        assert_eq!((row.v_from, row.v_to), (Some(2020), Some(2024)));
        assert_eq!(
            links(db, new)
                .iter()
                .map(|(e, _)| *e)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([e1, rule])
        );
    });
}

// @tg: verifies:: [[openspec:derived-beliefs#Derivations travel in bundles#Bundle round trip]] {since: "2026-10-28", level: integration}
// @tg: verifies:: [[tests#Derived Beliefs#Derivations Round Trip In Bundles]] {since: "2026-10-28", level: integration}
#[test]
fn a_derived_belief_round_trips_in_a_bundle() {
    for from in backends() {
        let dir = tempfile::tempdir().unwrap();
        let src = open(&dir, from);
        let e1 = assert_one(&src, "alice", "ordered", v("tea"));
        let e2 = assert_one(&src, "alice", "ordered", v("tea2"));
        let d = derive(&src, "alice", "prefers", v("tea"), &[e1, e2]).eid();
        let bundle = src.now().bundle(d).unwrap();
        // the belief, its two links and both supports
        assert_eq!(bundle.statements.len(), 5);
        for to in backends() {
            let dir = tempfile::tempdir().unwrap();
            let dst = open(&dir, to);
            let (r, imported) = run(&dst, |tx| tx.import_bundle(&bundle)).unwrap();
            assert_eq!(r.asserted.len(), 5);
            let root = imported.root;
            let sup: BTreeSet<Eid> = links(&dst, root).iter().map(|(e, _)| *e).collect();
            let want: BTreeSet<Eid> = imported
                .statements
                .iter()
                .filter(|s| {
                    let st = &bundle.statements[s.local as usize];
                    st.p == v("ordered")
                })
                .map(|s| s.eid)
                .collect();
            assert_eq!(sup, want);
            // a second import changes nothing
            let (again, rep) = run(&dst, |tx| tx.import_bundle(&bundle)).unwrap();
            assert!(again.asserted.is_empty(), "{again:?}");
            assert_eq!(rep.root, root);
            assert!(rep.statements.iter().all(|s| !s.new));
            // and the imported belief falls with an imported support
            let r = retract(&dst, *want.iter().next().unwrap());
            assert!(retracted(&r).contains(&(root, RetKind::Cascade)));
        }
    }
}

// @tg: verifies:: [[openspec:derived-beliefs#Derived upsert#Re-running reuses the node]] {since: "2026-10-28", level: integration}
// @tg: verifies:: [[tests#Derived Beliefs#Derived Keys Are Reused]] {since: "2026-10-28", level: integration}
#[test]
fn re_running_a_derived_upsert_reuses_the_node() {
    each(|db| {
        let e1 = assert_one(db, "alice", "ordered", v("tea"));
        let rule = assert_one(db, "rules", "has", Value::str("r1"));
        let key = Value::str("r/1/abc");
        let (_, n) = run(db, |tx| {
            tx.upsert_derived(v("skolem"), key.clone(), &[e1, rule])
        })
        .unwrap();
        let (r, again) = run(db, |tx| {
            tx.upsert_derived(v("skolem"), key.clone(), &[rule, e1])
        })
        .unwrap();
        assert_eq!(again, n);
        assert!(r.asserted.is_empty() && r.retracted.is_empty());
    });
}

// @tg: verifies:: [[openspec:derived-beliefs#Derived upsert#The key falls with its support]] {since: "2026-10-28", level: integration}
#[test]
fn a_derived_key_falls_with_its_support() {
    each(|db| {
        let e1 = assert_one(db, "alice", "ordered", v("tea"));
        let e2 = assert_one(db, "alice", "ordered", v("tea2"));
        let key = Value::str("r/1/abc");
        let (r, n) = run(db, |tx| tx.upsert_derived(v("skolem"), key.clone(), &[e1])).unwrap();
        let key_eid = r.asserted[0];
        let r = retract(db, e1);
        assert!(retracted(&r).contains(&(key_eid, RetKind::Cascade)));
        let (_, n2) = run(db, |tx| tx.upsert_derived(v("skolem"), key.clone(), &[e2])).unwrap();
        assert_ne!(n2, n);
    });
}

// @tg: verifies:: [[openspec:derived-beliefs#Derived upsert#An asserted key is not reused]] {since: "2026-10-28", level: integration}
#[test]
fn an_asserted_key_is_not_reused() {
    each(|db| {
        let e1 = assert_one(db, "alice", "ordered", v("tea"));
        let (_, n0) = run(db, |tx| {
            let n0 = tx.new_node()?;
            tx.assert(n0, v("skolem"), Value::str("k"), Valid::ALWAYS)?;
            Ok(n0)
        })
        .unwrap();
        let (_, n1) = run(db, |tx| {
            tx.upsert_derived(v("skolem"), Value::str("k"), &[e1])
        })
        .unwrap();
        assert_ne!(n1, n0);
        // and a key derived from other support is not reused either
        let e2 = assert_one(db, "alice", "ordered", v("tea2"));
        let (_, n2) = run(db, |tx| {
            tx.upsert_derived(v("skolem"), Value::str("k"), &[e2])
        })
        .unwrap();
        assert!(n2 != n0 && n2 != n1);
    });
}

// @tg: verifies:: [[openspec:retraction-cascade#Cascade over subject and object positions#Derived belief follows its support]] {since: "2026-10-28", level: integration}
#[test]
fn a_derived_belief_follows_its_support_in_one_transaction() {
    each(|db| {
        let e1 = assert_one(db, "alice", "ordered", v("tea"));
        let d = derive(db, "alice", "prefers", v("tea"), &[e1]).eid();
        let l = links(db, d)[0].1;
        let r = retract(db, e1);
        let rows = db.history().triples(None, None, None).unwrap();
        for e in [e1, l, d] {
            let row = rows.iter().find(|t| t.eid == e).unwrap();
            assert_eq!(row.t_ret, Some(r.t), "{e:?}");
        }
    });
}

// @tg: verifies:: [[openspec:statement-dependents#Dependents of a statement#Derived beliefs are dependents]] {since: "2026-10-28", level: integration}
// @tg: verifies:: [[tests#Derived Beliefs#Dependents Include Derived Beliefs]] {since: "2026-10-28", level: integration}
#[test]
fn derived_beliefs_are_dependents() {
    each(|db| {
        let e1 = assert_one(db, "alice", "ordered", v("tea"));
        let e2 = assert_one(db, "alice", "ordered", v("tea2"));
        let d = derive(db, "alice", "prefers", v("tea"), &[e1, e2]).eid();
        let ls = links(db, d);
        // e1, then D and its link to e1 in eid order, then D's other link
        assert_eq!(db.now().dependents(e1).unwrap(), [e1, d, ls[0].1, ls[1].1]);
        // the dry run retracts the same statements
        let dry = db
            .transact(
                TxOptions {
                    dry_run: true,
                    ..TxOptions::default()
                },
                |tx| tx.retract(e1).map(|_| ()),
            )
            .unwrap();
        let cascade: BTreeSet<Eid> = dry.retracted.iter().map(|(e, _)| *e).collect();
        assert_eq!(cascade, BTreeSet::from([e1, d, ls[0].1, ls[1].1]));
        // the past still knows what stood on e1
        let t = db
            .history()
            .triples(None, None, None)
            .unwrap()
            .last()
            .unwrap()
            .t_add;
        retract(db, e1);
        assert!(db.now().dependents(e1).unwrap().is_empty());
        assert_eq!(
            db.as_of(TimeRef::Tx(t.0)).dependents(e1).unwrap(),
            [e1, d, ls[0].1, ls[1].1]
        );
    });
}

// @tg: verifies:: [[tests#Derived Beliefs#Support Cascade Counts Toward The Limit]] {since: "2026-10-28", level: integration}
#[test]
fn the_cascade_limit_counts_derived_statements() {
    each(|db| {
        let e1 = assert_one(db, "alice", "ordered", v("tea"));
        derive(db, "alice", "prefers", v("tea"), &[e1]);
        // e1, its link and the belief: three statements
        let limited = |max_cascade| {
            db.transact(
                TxOptions {
                    max_cascade,
                    ..TxOptions::default()
                },
                |tx| tx.retract(e1).map(|_| ()),
            )
        };
        let err = limited(2).unwrap_err();
        assert!(matches!(err, Error::CascadeLimitExceeded { .. }), "{err}");
        assert_eq!(limited(3).unwrap().retracted.len(), 3);
    });
}
