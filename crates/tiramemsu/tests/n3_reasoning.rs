//! Spec `n3-reasoning` (OpenSpec change `add-n3-reasoning`): read-only N3
//! reasoning over a view, on SQLite and, with the `redb` feature, on redb.
#![cfg(feature = "n3")]

mod n3_common;

use n3_common::*;
use tiramemsu::*;

fn socrates(db: &Db) -> Eid {
    db.transact(TxOptions::default(), |tx| {
        tx.assert(v("socrates"), a(), v("Human"), Valid::ALWAYS)?;
        Ok(())
    })
    .unwrap()
    .asserted[0]
}

// @tg: verifies:: [[openspec:n3-reasoning#Reason over a view#Socrates]] {since: "2026-10-28", level: integration}
#[test]
fn socrates_is_mortal_and_nothing_is_written() {
    each(|name, db| {
        socrates(db);
        let before = (db.last_t().unwrap(), db.events_since(0).unwrap().len());
        let r = db
            .now()
            .reason(text(MORTALITY), &ReasonOptions::default())
            .unwrap();
        assert_eq!(r.conclusions.len(), 1, "{name}");
        assert_eq!(
            content(&r.conclusions[0]),
            spo(&v("socrates"), &a(), &v("Mortal"))
        );
        assert_eq!(r.conclusions[0].valid, Valid::ALWAYS);
        assert_eq!(r.status, ReasonStatus::Complete);
        assert_eq!(r.t, db.last_t().unwrap());
        assert_eq!(
            (db.last_t().unwrap(), db.events_since(0).unwrap().len()),
            before,
            "{name}: the store is unchanged"
        );
    });
}

// @tg: verifies:: [[openspec:n3-reasoning#Reason over a view#Parse error]] {since: "2026-10-28", level: integration}
#[test]
fn parse_error_has_a_line() {
    each(|name, db| {
        match db
            .now()
            .reason(text("{ ?x a v:Human } => "), &ReasonOptions::default())
        {
            Err(Error::N3Parse { line, message }) => {
                assert_eq!(line, 1, "{name}: {message}");
            }
            other => panic!("{name}: {other:?}"),
        }
        match db.now().reason(
            text("{ ?x a v:Human }\n=> { ?x a v:Mortal \n"),
            &ReasonOptions::default(),
        ) {
            Err(Error::N3Parse { line, .. }) => assert!(line >= 2, "{name}"),
            other => panic!("{name}: {other:?}"),
        }
    });
}

// @tg: verifies:: [[openspec:n3-reasoning#Reasoning scope and pruning#Pruning skips unused predicates]] {since: "2026-10-28", level: integration}
#[test]
fn pruning_skips_unused_predicates() {
    each(|name, db| {
        db.transact(TxOptions::default(), |tx| {
            for i in 0..100 {
                tx.assert(
                    v(&format!("p{i}")),
                    v("name"),
                    Value::str(format!("person {i}")),
                    Valid::ALWAYS,
                )?;
            }
            for who in ["socrates", "plato", "aristotle"] {
                tx.assert(v(who), a(), v("Human"), Valid::ALWAYS)?;
            }
            Ok(())
        })
        .unwrap();
        let r = db
            .now()
            .reason(text(MORTALITY), &ReasonOptions::default())
            .unwrap();
        assert_eq!((r.loaded, r.pruned), (3, true), "{name}");
        assert_eq!(r.conclusions.len(), 3);
    });
}

// @tg: verifies:: [[openspec:n3-reasoning#Reasoning scope and pruning#Variable predicate loads everything]] {since: "2026-10-28", level: integration}
#[test]
fn variable_predicate_loads_everything() {
    each(|name, db| {
        db.transact(TxOptions::default(), |tx| {
            let e = tx
                .assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?
                .eid();
            tx.assert(e, v("confidence"), Value::Double(0.9), Valid::ALWAYS)?;
            tx.assert(v("socrates"), a(), v("Human"), Valid::ALWAYS)?;
            Ok(())
        })
        .unwrap();
        let all = db.now().triples(None, None, None).unwrap().len();
        let r = db
            .now()
            .reason(
                text("{ ?s ?p ?o } => { ?p a v:Predicate } ."),
                &ReasonOptions::default(),
            )
            .unwrap();
        assert_eq!((r.loaded, r.pruned), (all, false), "{name}");
        // a variable predicate also matches the virtual `tm:content` triples
        // (never conclusions themselves)
        let mut preds: Vec<String> = r.conclusions.iter().map(|c| c.s.to_string()).collect();
        preds.sort();
        assert_eq!(
            preds,
            [
                a().to_string(),
                format!("<{}>", n3_engine::vocab::TM_CONTENT),
                v("confidence").to_string(),
                v("worksAt").to_string()
            ]
        );
    });
}

// @tg: verifies:: [[openspec:n3-reasoning#Reasoning scope and pruning#Graph scope]] {since: "2026-10-28", level: integration}
#[test]
fn graph_scope() {
    each(|name, db| {
        db.transact(TxOptions::default(), |tx| {
            let a1 = tx.assert(v("a"), a(), v("Human"), Valid::ALWAYS)?.eid();
            tx.assert(v("b"), a(), v("Human"), Valid::ALWAYS)?;
            tx.add_to_graph(a1, v("g1"), AssertOpts::default())?;
            Ok(())
        })
        .unwrap();
        let opts = ReasonOptions {
            scope: ReasonScope::Graphs(vec![v("g1")]),
            ..ReasonOptions::default()
        };
        let r = db.now().reason(text(MORTALITY), &opts).unwrap();
        let got: Vec<String> = r.conclusions.iter().map(content).collect();
        assert_eq!(got, [spo(&v("a"), &a(), &v("Mortal"))], "{name}");
        assert_eq!(r.loaded, 1);
    });
}

#[test]
fn seed_scope() {
    each(|name, db| {
        db.transact(TxOptions::default(), |tx| {
            tx.assert(v("a"), v("knows"), v("b"), Valid::ALWAYS)?;
            tx.assert(v("b"), v("knows"), v("c"), Valid::ALWAYS)?;
            tx.assert(v("c"), v("knows"), v("d"), Valid::ALWAYS)?;
            Ok(())
        })
        .unwrap();
        let rule = text("{ ?x v:knows ?y } => { ?y v:knownBy ?x } .");
        for (hops, want) in [(0, 0), (1, 1), (2, 2), (5, 3)] {
            let opts = ReasonOptions {
                scope: ReasonScope::Seeds {
                    nodes: vec![v("a")],
                    hops,
                },
                ..ReasonOptions::default()
            };
            let r = db.now().reason(rule.clone(), &opts).unwrap();
            assert_eq!(
                (r.loaded, r.conclusions.len()),
                (want, want),
                "{name} {hops}"
            );
        }
    });
}

const WORKS_IN: &str = "{ ?p v:worksAt ?c. ?c v:locatedIn ?l } => { ?p v:worksIn ?l } .";

fn work_facts(db: &Db, job: Valid) -> (Eid, Eid) {
    let r = db
        .transact(TxOptions::default(), |tx| {
            tx.assert(v("alice"), v("worksAt"), v("acme"), job)?;
            tx.assert(
                v("acme"),
                v("locatedIn"),
                v("berlin"),
                Valid::from(year(2024)),
            )?;
            Ok(())
        })
        .unwrap();
    (r.asserted[0], r.asserted[1])
}

// @tg: verifies:: [[openspec:n3-reasoning#Pointwise valid time#Facts never simultaneously valid do not combine]] {since: "2026-10-28", level: integration}
#[test]
fn facts_never_simultaneously_valid_do_not_combine() {
    each(|name, db| {
        work_facts(db, Valid::between(year(2020), year(2022)));
        let r = db
            .now()
            .reason(text(WORKS_IN), &ReasonOptions::default())
            .unwrap();
        assert!(r.conclusions.is_empty(), "{name}: {:?}", r.conclusions);
        assert_eq!(r.segments, 4);
        // ignoring valid time, they combine
        let atemporal = ReasonOptions {
            time: TimeMode::Atemporal,
            ..ReasonOptions::default()
        };
        let r = db.now().reason(text(WORKS_IN), &atemporal).unwrap();
        assert_eq!(r.conclusions.len(), 1);
        assert_eq!(r.conclusions[0].valid, Valid::ALWAYS);
    });
}

// @tg: verifies:: [[openspec:n3-reasoning#Pointwise valid time#Overlap gives the intersection]] {since: "2026-10-28", level: integration}
#[test]
fn overlap_gives_the_intersection() {
    each(|name, db| {
        let (job, place) = work_facts(db, Valid::between(year(2020), year(2025)));
        let r = db
            .now()
            .reason(text(WORKS_IN), &ReasonOptions::default())
            .unwrap();
        assert_eq!(r.conclusions.len(), 1, "{name}");
        let c = &r.conclusions[0];
        assert_eq!(content(c), spo(&v("alice"), &v("worksIn"), &v("berlin")));
        assert_eq!(c.valid, Valid::between(year(2024), year(2025)));
        assert_eq!(
            c.proof.premises,
            [Premise::Statement(job), Premise::Statement(place)]
        );
        // a window keeps the conclusion inside it
        let opts = ReasonOptions {
            window: Valid::from(year(2024) + 1000),
            ..ReasonOptions::default()
        };
        let r = db.now().reason(text(WORKS_IN), &opts).unwrap();
        assert_eq!(
            r.conclusions[0].valid,
            Valid::between(year(2024) + 1000, year(2025))
        );
    });
}

// @tg: verifies:: [[openspec:n3-reasoning#Pointwise valid time#Snapshot view]] {since: "2026-10-28", level: integration}
#[test]
fn snapshot_view() {
    each(|name, db| {
        work_facts(db, Valid::between(year(2020), year(2025)));
        let t = date("2024-06-01");
        let r = db
            .now()
            .valid_at(t)
            .reason(text(WORKS_IN), &ReasonOptions::default())
            .unwrap();
        assert_eq!(r.segments, 1, "{name}");
        assert_eq!(r.conclusions.len(), 1);
        assert_eq!(r.conclusions[0].valid, Valid::between(t, t + 1));
        let r = db
            .now()
            .valid_at(year(2023))
            .reason(text(WORKS_IN), &ReasonOptions::default())
            .unwrap();
        assert!(r.conclusions.is_empty());
    });
}

// @tg: verifies:: [[openspec:n3-reasoning#Pointwise valid time#Negation is evaluated per segment]] {since: "2026-10-28", level: integration}
#[test]
fn negation_is_evaluated_per_segment() {
    each(|name, db| {
        db.transact(TxOptions::default(), |tx| {
            tx.assert(v("bob"), a(), v("Employee"), Valid::ALWAYS)?;
            tx.assert(v("bob"), v("manager"), v("carol"), Valid::from(year(2023)))?;
            Ok(())
        })
        .unwrap();
        let r = db
            .now()
            .reason(
                text("{ ?x a v:Employee. ?SCOPE log:notIncludes { ?x v:manager ?m } } => { ?x a v:Unmanaged } ."),
                &ReasonOptions::default(),
            )
            .unwrap();
        let got: Vec<(String, Valid)> = r
            .conclusions
            .iter()
            .map(|c| (content(c), c.valid))
            .collect();
        assert_eq!(
            got,
            [(
                spo(&v("bob"), &a(), &v("Unmanaged")),
                Valid::until(year(2023))
            )],
            "{name}"
        );
    });
}

// @tg: verifies:: [[openspec:n3-reasoning#Pointwise valid time#Segment limit]] {since: "2026-10-28", level: integration}
#[test]
fn segment_limit() {
    each(|name, db| {
        db.transact(TxOptions::default(), |tx| {
            for i in 0..10 {
                tx.assert(
                    v(&format!("h{i}")),
                    a(),
                    v("Human"),
                    Valid::between(year(2000 + i), year(2001 + i)),
                )?;
            }
            Ok(())
        })
        .unwrap();
        let opts = ReasonOptions {
            max_segments: 5,
            ..ReasonOptions::default()
        };
        let r = db.now().reason(text(MORTALITY), &opts).unwrap();
        assert_eq!(
            r.status,
            ReasonStatus::Incomplete {
                limits: vec![ReasonLimit::Segments]
            },
            "{name}"
        );
        assert_eq!(r.segments, 5);
        // sound for the segments that ran: (-inf, 2000) and 2000..2004
        assert_eq!(r.conclusions.len(), 4);
        for c in &r.conclusions {
            assert!(c.valid.to.unwrap() <= year(2004));
        }
    });
}

// @tg: verifies:: [[openspec:n3-reasoning#Transaction time and the pinned clock#As-of reasoning ignores later facts]] {since: "2026-10-28", level: integration}
#[test]
fn as_of_reasoning_ignores_later_facts() {
    each(|name, db| {
        let t1 = db
            .transact(TxOptions::default(), |tx| {
                tx.assert(v("plato"), v("name"), Value::str("Plato"), Valid::ALWAYS)?;
                Ok(())
            })
            .unwrap();
        let t2 = db
            .transact(TxOptions::default(), |tx| {
                tx.assert(v("socrates"), a(), v("Human"), Valid::ALWAYS)?;
                Ok(())
            })
            .unwrap();
        assert_eq!((t1.t, t2.t), (TxId(1), TxId(2)));
        let r = db
            .as_of(TimeRef::Tx(1))
            .reason(text(MORTALITY), &ReasonOptions::default())
            .unwrap();
        assert!(r.conclusions.is_empty(), "{name}");
        assert_eq!((r.t, r.now), (1, t1.instant));
        let r = db
            .as_of(TimeRef::Tx(2))
            .reason(text(MORTALITY), &ReasonOptions::default())
            .unwrap();
        assert_eq!(r.conclusions.len(), 1);
        // an instant resolves to the last transaction at or before it
        let r = db
            .as_of(TimeRef::Instant(t2.instant))
            .reason(text(MORTALITY), &ReasonOptions::default())
            .unwrap();
        assert_eq!((r.t, r.now, r.conclusions.len()), (2, t2.instant, 1));
    });
}

// @tg: verifies:: [[openspec:n3-reasoning#Transaction time and the pinned clock#Reproducible clock]] {since: "2026-10-28", level: integration}
#[test]
fn reproducible_clock() {
    each(|name, db| {
        let mut instants = Vec::new();
        for i in 0..4 {
            let r = db
                .transact(TxOptions::default(), |tx| {
                    tx.assert(v(&format!("n{i}")), v("p"), Value::Int(i), Valid::ALWAYS)?;
                    Ok(())
                })
                .unwrap();
            instants.push(r.instant);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let rules = text("{ \"\" time:localTime ?t } => { v:run v:at ?t } .");
        let run = || {
            db.as_of(TimeRef::Tx(3))
                .reason(rules.clone(), &ReasonOptions::default())
                .unwrap()
        };
        let (first, second) = (run(), run());
        assert_eq!(first.conclusions, second.conclusions, "{name}");
        assert_eq!(first.now, instants[2]);
        match first.conclusions[0].o.as_value() {
            Some(Value::DateTime { ms, .. }) => assert_eq!(*ms, instants[2], "{name}"),
            other => panic!("{name}: {other:?}"),
        }
        // the clock is `tm:run tm:now` too, and a clock-reading rule set says so
        let r = db
            .as_of(TimeRef::Tx(3))
            .reason(
                text("{ tm:run tm:now ?t } => { v:run v:at ?t } ."),
                &ReasonOptions::default(),
            )
            .unwrap();
        assert_eq!(r.conclusions[0].o, first.conclusions[0].o);
    });
}

// @tg: verifies:: [[openspec:n3-reasoning#Statements in N3#Layer on a statement]] {since: "2026-10-28", level: integration}
#[test]
fn layer_on_a_statement() {
    each(|name, db| {
        let r = db
            .transact(TxOptions::default(), |tx| {
                let e1 = tx
                    .assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?
                    .eid();
                tx.assert(e1, v("confidence"), Value::Double(0.9), Valid::ALWAYS)?;
                let e2 = tx
                    .assert(v("bob"), v("worksAt"), v("initech"), Valid::ALWAYS)?
                    .eid();
                tx.assert(e2, v("confidence"), Value::Double(0.5), Valid::ALWAYS)?;
                Ok(())
            })
            .unwrap();
        let (e1, layer) = (r.asserted[0], r.asserted[1]);
        let res = db
            .now()
            .reason(
                text("{ ?e v:confidence ?c. ?c math:greaterThan 0.8. ?e tm:content { ?s ?p ?o } } => { ?s v:trusted ?o } ."),
                &ReasonOptions::default(),
            )
            .unwrap();
        assert_eq!(res.conclusions.len(), 1, "{name}: {:?}", res.conclusions);
        // the variable predicate inside `tm:content` keeps pruning on: the layers
        // and the statements they are about are loaded, not the whole view
        assert!(res.pruned, "{name}");
        assert_eq!(res.loaded, 4);
        let c = &res.conclusions[0];
        assert_eq!(content(c), spo(&v("alice"), &v("trusted"), &v("acme")));
        assert_eq!(
            c.proof.premises,
            [Premise::Statement(layer), Premise::Statement(e1)]
        );
    });
}

// @tg: verifies:: [[openspec:n3-reasoning#Statements in N3#Validity as data]] {since: "2026-10-28", level: integration}
#[test]
fn validity_as_data() {
    each(|name, db| {
        let e1 = db
            .transact(TxOptions::default(), |tx| {
                tx.assert(
                    v("alice"),
                    v("worksAt"),
                    v("acme"),
                    Valid::between(date("2020-01-01"), date("2023-01-01")),
                )?;
                Ok(())
            })
            .unwrap()
            .asserted[0];
        let r = db
            .now()
            .reason(
                text("{ ?e tm:content { ?s v:worksAt ?o }. ?e tm:validFrom ?a. ?e tm:validTo ?b } => { ?s v:from ?a. ?s v:to ?b } ."),
                &ReasonOptions::default(),
            )
            .unwrap();
        let mut got: Vec<(String, String)> = r
            .conclusions
            .iter()
            .map(|c| {
                let lex = match rdf::render(c.o.as_value().unwrap()) {
                    RdfTerm::Literal { lex, datatype, .. } => {
                        assert_eq!(datatype.as_deref(), Some(vocab::XSD_DATETIME));
                        lex
                    }
                    other => panic!("{other:?}"),
                };
                (c.p.to_string(), lex)
            })
            .collect();
        got.sort();
        assert_eq!(
            got,
            [
                (v("from").to_string(), "2020-01-01T00:00:00Z".to_string()),
                (v("to").to_string(), "2023-01-01T00:00:00Z".to_string()),
            ],
            "{name}"
        );
        for c in &r.conclusions {
            assert_eq!(c.proof.premises, [Premise::Statement(e1)]);
            assert_eq!(
                c.valid,
                Valid::between(date("2020-01-01"), date("2023-01-01"))
            );
        }
    });
}

// @tg: verifies:: [[openspec:n3-reasoning#Proofs and completion#Proof lists premise eids]] {since: "2026-10-28", level: integration}
#[test]
fn proof_lists_premise_eids() {
    each(|name, db| {
        let e1 = socrates(db);
        let opts = ReasonOptions {
            proof_n3: true,
            ..ReasonOptions::default()
        };
        let r = db
            .now()
            .reason(
                text(&format!(
                    "{MORTALITY}\n{{ ?x a v:Mortal }} => {{ ?x v:has v:lifespan }} ."
                )),
                &opts,
            )
            .unwrap();
        assert_eq!(r.conclusions.len(), 2, "{name}");
        let p = &r.conclusions[0].proof;
        assert_eq!((p.rule_set.clone(), p.rule), (None, 0));
        assert_eq!(p.premises, [Premise::Statement(e1)]);
        let p = &r.conclusions[1].proof;
        assert_eq!(
            (p.rule, p.premises.clone()),
            (1, vec![Premise::Conclusion(0)])
        );
        let proof = r.proof_n3.unwrap();
        assert!(proof.contains("v:Mortal"), "{proof}");
    });
}

// @tg: verifies:: [[openspec:n3-reasoning#Proofs and completion#Fuse]] {since: "2026-10-28", level: integration}
#[test]
fn fuse_is_a_contradiction() {
    each(|name, db| {
        db.transact(TxOptions::default(), |tx| {
            tx.assert(v("koko"), a(), v("Cat"), Valid::ALWAYS)?;
            tx.assert(v("koko"), a(), v("Dog"), Valid::ALWAYS)?;
            Ok(())
        })
        .unwrap();
        let r = db
            .now()
            .reason(
                text(
                    "{ ?x a v:Pet } => { ?x a v:Animal } .\n{ ?x a v:Cat. ?x a v:Dog } => false .",
                ),
                &ReasonOptions::default(),
            )
            .unwrap();
        let c = r.contradiction.expect(name);
        assert_eq!(c.rule, 1);
        assert_eq!(c.instance.len(), 2);
        assert!(c.instance[0].contains("koko"), "{:?}", c.instance);
    });
}

#[test]
fn reasoning_inside_a_speculation_sees_its_writes() {
    let (_d, db) = {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path().join("s.db"), OpenOptions::default()).unwrap();
        (dir, db)
    };
    let n = db
        .with(
            |tx| {
                tx.assert(v("socrates"), a(), v("Human"), Valid::ALWAYS)?;
                Ok(())
            },
            |view| {
                Ok(view
                    .reason(text(MORTALITY), &ReasonOptions::default())?
                    .conclusions
                    .len())
            },
        )
        .unwrap();
    assert_eq!(n, 1);
    assert!(db.now().triples(None, None, None).unwrap().is_empty());
}
