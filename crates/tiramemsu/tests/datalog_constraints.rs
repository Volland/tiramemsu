//! Datalog constraints enforced at commit (OpenSpec change
//! `add-datalog-constraints`): one test per scenario, on SQLite and redb.
#![cfg(feature = "datalog")]

use std::sync::Arc;

use tiramemsu::*;

const H: i64 = 3_600_000;

fn v(s: &str) -> Value {
    Value::iri(format!("urn:tiramemsu:v:{s}"))
}

fn year(y: i64) -> i64 {
    let days = |y: i64| {
        let y1 = y - 1;
        365 * (y1 - 1969) + (y1 / 4 - 1969 / 4) - (y1 / 100 - 1969 / 100) + (y1 / 400 - 1969 / 400)
    };
    days(y) * 86_400_000
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
            dir.path().join(format!("c.{}", b.name())),
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

fn tx(db: &Db, f: impl FnOnce(&mut Tx<'_>) -> Result<()>) -> Result<TxReport> {
    db.transact(TxOptions::default(), f)
}

const ONE_CEO: &str = "constraint one_ceo error :- v:role(X, v:CEO), v:role(Y, v:CEO), X != Y.\n";

fn enforce(db: &Db, text: &str) -> Result<TxReport> {
    tx(db, |t| {
        t.store_program(v("policies"), text)?;
        t.enforce(v("policies"), true)?;
        Ok(())
    })
}

fn ceo(db: &Db, who: &str, valid: Valid) -> Result<TxReport> {
    tx(db, |t| {
        t.assert(v(who), v("role"), v("CEO"), valid)?;
        Ok(())
    })
}

fn live(db: &Db) -> usize {
    db.now().triples(None, None, None).unwrap().len()
}

// @tg: verifies:: [[openspec:datalog-constraints#Enforce a program#Enforce clean data]] {level: integration}
#[test]
fn enforce_clean_data() {
    each(|db| {
        ceo(db, "a", Valid::ALWAYS).unwrap();
        enforce(db, ONE_CEO).unwrap();
        let marks = db
            .now()
            .triples(
                None,
                Some(
                    db.now()
                        .encode(&Value::iri(vocab::DL_ENFORCED))
                        .unwrap()
                        .unwrap(),
                ),
                None,
            )
            .unwrap();
        assert_eq!(marks.len(), 1);
    });
}

// @tg: verifies:: [[openspec:datalog-constraints#Enforce a program#Enforce over violating data]] {level: integration}
#[test]
fn enforce_over_violating_data() {
    each(|db| {
        ceo(db, "a", Valid::ALWAYS).unwrap();
        ceo(db, "b", Valid::ALWAYS).unwrap();
        let n = live(db);
        match enforce(db, ONE_CEO) {
            Err(Error::ConstraintViolated {
                constraint,
                violations,
                ..
            }) => {
                assert_eq!(constraint, "one_ceo");
                assert_eq!(violations.len(), 2);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(live(db), n);
    });
}

// @tg: verifies:: [[openspec:datalog-constraints#Enforce a program#Supersede re-checks]] {level: integration}
#[test]
fn supersede_re_checks() {
    each(|db| {
        ceo(db, "a", Valid::ALWAYS).unwrap();
        assert_fact(db, "a", "title", v("boss"));
        assert_fact(db, "b", "title", v("boss"));
        enforce(db, ONE_CEO).unwrap();
        let r = tx(db, |t| {
            t.store_program(v("policies"), &format!("{ONE_CEO}constraint one_boss error :- v:title(X, v:boss), v:title(Y, v:boss), X != Y.\n"))?;
            Ok(())
        });
        assert!(matches!(r, Err(Error::ConstraintViolated { .. })), "{r:?}");
    });
}

fn assert_fact(db: &Db, s: &str, p: &str, o: Value) {
    tx(db, |t| {
        t.assert(v(s), v(p), o, Valid::ALWAYS)?;
        Ok(())
    })
    .unwrap();
}

// @tg: verifies:: [[openspec:datalog-constraints#Error constraints are checked at commit#Second CEO refused]] {level: integration}
#[test]
fn second_ceo_refused() {
    each(|db| {
        ceo(db, "a", Valid::ALWAYS).unwrap();
        enforce(db, ONE_CEO).unwrap();
        let n = live(db);
        match ceo(db, "b", Valid::from(year(2025))) {
            Err(Error::ConstraintViolated {
                constraint,
                violations,
                ..
            }) => {
                assert_eq!(constraint, "one_ceo");
                assert!(violations
                    .iter()
                    .all(|x| x.valid == vec![Valid::from(year(2025))]));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(live(db), n);
    });
}

// @tg: verifies:: [[openspec:datalog-constraints#Error constraints are checked at commit#Successor accepted]] {level: integration}
#[test]
fn successor_accepted() {
    each(|db| {
        let a = ceo(db, "a", Valid::ALWAYS).unwrap().asserted[0];
        enforce(db, ONE_CEO).unwrap();
        tx(db, |t| {
            t.supersede(
                a,
                Patch {
                    v_to: Some(Some(year(2025))),
                    ..Patch::default()
                },
            )?;
            t.assert(v("b"), v("role"), v("CEO"), Valid::from(year(2025)))?;
            Ok(())
        })
        .unwrap();
    });
}

// @tg: verifies:: [[openspec:datalog-constraints#Error constraints are checked at commit#Negation reacts to retraction]] {level: integration}
#[test]
fn negation_reacts_to_retraction() {
    each(|db| {
        let owner = tx(db, |t| {
            t.assert(
                v("p1"),
                Value::iri(vocab::RDF_TYPE),
                v("Project"),
                Valid::ALWAYS,
            )?;
            t.assert(v("p1"), v("owner"), v("ann"), Valid::ALWAYS)?;
            Ok(())
        })
        .unwrap()
        .asserted[1];
        enforce(
            db,
            "constraint owned error :- rdf:type(P, v:Project), not v:owner(P, _).\n",
        )
        .unwrap();
        let r = tx(db, |t| {
            t.retract(owner)?;
            Ok(())
        });
        assert!(
            matches!(&r, Err(Error::ConstraintViolated { constraint, .. }) if constraint == "owned"),
            "{r:?}"
        );
    });
}

// @tg: verifies:: [[openspec:datalog-constraints#Error constraints are checked at commit#Warn reports without failing]] {level: integration}
#[test]
fn warn_reports_without_failing() {
    each(|db| {
        let owner = tx(db, |t| {
            t.assert(
                v("p1"),
                Value::iri(vocab::RDF_TYPE),
                v("Project"),
                Valid::ALWAYS,
            )?;
            t.assert(v("p1"), v("owner"), v("ann"), Valid::ALWAYS)?;
            Ok(())
        })
        .unwrap()
        .asserted[1];
        enforce(
            db,
            "constraint owned warn :- rdf:type(P, v:Project), not v:owner(P, _).\n",
        )
        .unwrap();
        let r = tx(db, |t| {
            t.retract(owner)?;
            Ok(())
        })
        .unwrap();
        assert_eq!(r.constraint_violations.len(), 1);
        assert_eq!(r.constraint_violations[0].constraint, "owned");
        assert_eq!(r.constraint_violations[0].severity, "warn");
        r.constraint_checks
    });
}

// @tg: verifies:: [[openspec:datalog-constraints#Error constraints are checked at commit#Metric window]] {level: integration}
#[test]
fn metric_window() {
    each(|db| {
        tx(db, |t| {
            t.assert(
                v("pat"),
                v("dose"),
                Value::Int(1),
                Valid::between(10 * H, 10 * H + 1),
            )?;
            Ok(())
        })
        .unwrap();
        enforce(
            db,
            "constraint cooldown error :- v:dose(P, _), past[1ms, 6h] v:dose(P, _).\n",
        )
        .unwrap();
        let r = tx(db, |t| {
            t.assert(
                v("pat"),
                v("dose"),
                Value::Int(2),
                Valid::between(12 * H, 12 * H + 1),
            )?;
            Ok(())
        });
        assert!(
            matches!(&r, Err(Error::ConstraintViolated { constraint, .. }) if constraint == "cooldown"),
            "{r:?}"
        );
        // eight hours later is fine
        tx(db, |t| {
            t.assert(
                v("pat"),
                v("dose"),
                Value::Int(3),
                Valid::between(18 * H, 18 * H + 1),
            )?;
            Ok(())
        })
        .unwrap();
    });
}

// @tg: verifies:: [[openspec:datalog-constraints#Checks read only the change#Unrelated write skips the check]] {level: integration}
#[test]
fn unrelated_write_skips_the_check() {
    each(|db| {
        enforce(db, ONE_CEO).unwrap();
        let r = tx(db, |t| {
            t.assert(v("a"), v("name"), Value::str("Ada"), Valid::ALWAYS)?;
            Ok(())
        })
        .unwrap();
        assert_eq!(r.constraint_checks, 0);
        assert_eq!(r.seed_rows, 0);
    });
}

// @tg: verifies:: [[openspec:datalog-constraints#Checks read only the change#Seed rows follow the write]] {level: integration}
#[test]
fn seed_rows_follow_the_write() {
    each(|db| {
        tx(db, |t| {
            for i in 0..100_000 {
                t.assert(
                    v(&format!("p{i}")),
                    v("role"),
                    v(&format!("r{}", i % 50)),
                    Valid::ALWAYS,
                )?;
            }
            Ok(())
        })
        .unwrap();
        enforce(db, ONE_CEO).unwrap();
        let r = ceo(db, "boss", Valid::ALWAYS).unwrap();
        assert_eq!(r.constraint_checks, 1);
        assert_eq!(
            r.seed_rows, 2,
            "one seed row per positive atom of the constraint"
        );
        r.seed_rows
    });
}

// @tg: verifies:: [[openspec:datalog-constraints#Enforceable constraints#Recursive constraint not enforceable]] {level: integration}
#[test]
fn recursive_constraint_not_enforceable() {
    each(|db| {
        let r = enforce(
            db,
            "reports(X, Y) :- v:manager(X, Y).\nreports(X, Z) :- reports(X, Y), v:manager(Y, Z).\nconstraint no_cycle error :- reports(X, X).\n",
        );
        assert!(
            matches!(&r, Err(Error::ConstraintNotEnforceable { constraint, .. }) if constraint == "no_cycle"),
            "{r:?}"
        );
        // still checkable on demand
        let c = db
            .now()
            .check(
                &DatalogProgram::Text("reports(X, Y) :- v:manager(X, Y).\nconstraint no_cycle error :- reports(X, X).\n".into()),
                &[],
                &DatalogOptions::default(),
            )
            .unwrap();
        assert!(c.violated.is_empty());
    });
}

// @tg: verifies:: [[openspec:datalog-constraints#Enforceable constraints#Non-recursive relation inlined]] {level: integration}
#[test]
fn non_recursive_relation_inlined() {
    each(|db| {
        ceo(db, "a", Valid::ALWAYS).unwrap();
        enforce(
            db,
            "ceo(X) :- v:role(X, v:CEO).\nconstraint one_ceo error :- ceo(X), ceo(Y), X != Y.\n",
        )
        .unwrap();
        let r = ceo(db, "b", Valid::ALWAYS);
        assert!(matches!(r, Err(Error::ConstraintViolated { .. })), "{r:?}");
    });
}

// @tg: verifies:: [[openspec:datalog-constraints#Constraint check budget#Budget fails closed]] {level: integration}
#[test]
fn budget_fails_closed() {
    each(|db| {
        enforce(db, ONE_CEO).unwrap();
        let n = live(db);
        let r = db.transact(
            TxOptions {
                constraint_budget: Some(ConstraintBudget {
                    timeout_ms: 1_000,
                    max_seed_rows: 10,
                }),
                ..TxOptions::default()
            },
            |t| {
                // 20 rows that match the constraint's atoms: 40 seed rows
                for i in 0..20 {
                    t.assert(v(&format!("x{i}")), v("role"), v("CEO"), Valid::ALWAYS)?;
                }
                Ok(())
            },
        );
        assert!(
            matches!(r, Err(Error::ConstraintBudgetExceeded { .. })),
            "{r:?}"
        );
        assert_eq!(live(db), n);
    });
}

// @tg: verifies:: [[openspec:datalog-constraints#Previews report violations#Dry run lists the violation]] {level: integration}
#[test]
fn dry_run_lists_the_violation() {
    each(|db| {
        ceo(db, "a", Valid::ALWAYS).unwrap();
        enforce(db, ONE_CEO).unwrap();
        let n = live(db);
        let r = db
            .transact(
                TxOptions {
                    dry_run: true,
                    ..TxOptions::default()
                },
                |t| {
                    t.assert(v("b"), v("role"), v("CEO"), Valid::ALWAYS)?;
                    Ok(())
                },
            )
            .unwrap();
        assert!(r
            .constraint_violations
            .iter()
            .any(|x| x.constraint == "one_ceo" && x.severity == "error"));
        assert_eq!(live(db), n);
        // speculation fails as a commit would
        let s = db.with(
            |t| {
                t.assert(v("b"), v("role"), v("CEO"), Valid::ALWAYS)?;
                Ok(())
            },
            |view| view.triples(None, None, None).map(|x| x.len()),
        );
        assert!(matches!(s, Err(Error::ConstraintViolated { .. })), "{s:?}");
    });
}

// @tg: verifies:: [[openspec:datalog-constraints#Same verdict on every backend#Random histories agree]] {level: integration}
#[test]
fn random_histories_agree() {
    for seed in [1u64, 2, 3, 4] {
        each(|db| {
            enforce(db, "constraint one_ceo error :- v:role(X, v:CEO), v:role(Y, v:CEO), X != Y.\nconstraint owned warn :- rdf:type(P, v:Project), not v:owner(P, _).\n").unwrap();
            let mut x = seed;
            let mut next = |m: u64| {
                x = x
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                (x >> 33) % m
            };
            let mut verdicts = Vec::new();
            let mut eids: Vec<Eid> = Vec::new();
            for _ in 0..25 {
                let kind = next(4);
                let who = format!("n{}", next(4));
                let r = tx(db, |t| {
                    match kind {
                        0 => {
                            let f = year(2020 + next(6) as i64);
                            t.assert(
                                v(&who),
                                v("role"),
                                v("CEO"),
                                Valid::between(f, f + year(1971)),
                            )?;
                        }
                        1 => {
                            t.assert(
                                v(&who),
                                Value::iri(vocab::RDF_TYPE),
                                v("Project"),
                                Valid::ALWAYS,
                            )?;
                        }
                        2 => {
                            t.assert(v(&who), v("owner"), v("ann"), Valid::ALWAYS)?;
                        }
                        _ => {
                            if let Some(e) = eids.get(next(eids.len().max(1) as u64) as usize) {
                                t.retract(*e)?;
                            }
                        }
                    }
                    Ok(())
                });
                match r {
                    Ok(rep) => {
                        eids.extend(rep.asserted.iter().copied());
                        verdicts.push(format!(
                            "ok {} {}",
                            rep.constraint_checks,
                            rep.constraint_violations.len()
                        ));
                    }
                    Err(e) => verdicts.push(format!("{e}")),
                }
            }
            verdicts
        });
    }
}

/// The `error` violations of a full check, by bindings, with their instants.
fn full_errors(
    view: &View<'_>,
    text: &str,
) -> Vec<(String, tiramemsu::datalog_engine::interval::IvSet)> {
    use tiramemsu::datalog_engine::interval::IvSet;
    let r = view
        .check(
            &DatalogProgram::Text(text.into()),
            &[],
            &DatalogOptions::default(),
        )
        .unwrap();
    let mut out: Vec<(String, IvSet)> = Vec::new();
    for c in r.violated.iter().filter(|c| c.severity == Severity::Error) {
        for x in &c.violations {
            let key = format!("{} {:?}", c.name, x.bindings);
            let iv = x
                .valid
                .iter()
                .fold(IvSet::empty(), |a, v| a.union(&IvSet::of_valid(*v)));
            match out.iter_mut().find(|(k, _)| *k == key) {
                Some((_, s)) => *s = s.union(&iv),
                None => out.push((key, iv)),
            }
        }
    }
    out
}

// add-datalog-constraints task 4.2: a transaction is refused by the seeded check
// exactly when a full check after it finds an `error` violation (or part of one)
// that a full check before it did not
#[test]
fn seeded_check_equals_full_check() {
    let text = "constraint one_ceo error :- v:role(X, v:CEO), v:role(Y, v:CEO), X != Y.\n\
                constraint owned error :- rdf:type(P, v:Project), not v:owner(P, _).\n";
    let mut refusals = 0;
    for seed in 1u64..7 {
        let dir = tempfile::tempdir().unwrap();
        let open = |n: &str| {
            Db::open(
                dir.path().join(n),
                OpenOptions {
                    clock: Arc::new(ManualClock::new(1_000)),
                    ..OpenOptions::default()
                },
            )
            .unwrap()
        };
        let (enforced, mirror) = (open("e.db"), open("m.db"));
        enforce(&enforced, text).unwrap();
        let mut x = seed;
        let mut next = |m: u64| {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (x >> 33) % m
        };
        let mut live: Vec<(String, String, String)> = Vec::new();
        for _ in 0..30 {
            let op = next(5);
            let who = format!("n{}", next(4));
            let f = year(2020 + next(5) as i64);
            let to = f + year(1970 + 1 + next(3) as i64) - year(1970);
            let pick = (!live.is_empty()).then(|| live[next(live.len() as u64) as usize].clone());
            let apply = |t: &mut Tx<'_>| -> Result<()> {
                match op {
                    0 | 1 => {
                        t.assert(v(&who), v("role"), v("CEO"), Valid::between(f, to))?;
                    }
                    2 => {
                        t.assert(
                            v(&who),
                            Value::iri(vocab::RDF_TYPE),
                            v("Project"),
                            Valid::ALWAYS,
                        )?;
                    }
                    3 => {
                        t.assert(v(&who), v("owner"), v("ann"), Valid::ALWAYS)?;
                    }
                    _ => {
                        if let Some((s, p, o)) = &pick {
                            let ps = if p == "type" {
                                Value::iri(vocab::RDF_TYPE)
                            } else {
                                v(p)
                            };
                            let (s, p, o) = (t.lookup(&v(s))?, t.lookup(&ps)?, t.lookup(&v(o))?);
                            if let (Some(s), Some(p), Some(o)) = (s, p, o) {
                                t.retract_matching(Some(s), Some(p), Some(o))?;
                            }
                        }
                    }
                }
                Ok(())
            };
            let before = full_errors(&mirror.now(), text);
            let after = mirror
                .with(|t| apply(t), |view| Ok(full_errors(view, text)))
                .unwrap();
            let new = after.iter().any(|(k, iv)| {
                before
                    .iter()
                    .find(|(b, _)| b == k)
                    .is_none_or(|(_, biv)| !biv.covers(iv))
            });
            let refused = matches!(
                enforced.transact(TxOptions::default(), apply),
                Err(Error::ConstraintViolated { .. })
            );
            assert_eq!(
                refused, new,
                "seed {seed}: op {op} on {who}; before {before:?}, after {after:?}"
            );
            refusals += refused as usize;
            if !refused {
                mirror.transact(TxOptions::default(), apply).unwrap();
                match op {
                    0 | 1 => live.push((who.clone(), "role".into(), "CEO".into())),
                    2 => live.push((who.clone(), "type".into(), "Project".into())),
                    3 => live.push((who.clone(), "owner".into(), "ann".into())),
                    _ => {}
                }
            }
        }
    }
    assert!(refusals > 3, "only {refusals} refusals");
}
