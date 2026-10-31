//! ELPI (OpenSpec changes `add-elpi-engine`, `add-elpi-programs` and
//! `add-elpi-materialization`) through the facade: one test per scenario of
//! the changes' delta specs, each on SQLite and, with the `redb` feature, on
//! redb, where every backend must observe the same thing.
#![cfg(feature = "elpi")]

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
        dir.path().join(format!("e.{}", backend.name())),
        OpenOptions {
            backend,
            clock: Arc::new(ManualClock::new(1_000)),
            ..OpenOptions::default()
        },
    )
    .unwrap()
}

fn each<R: std::fmt::Debug + PartialEq>(scenario: impl Fn(&Db) -> R) {
    let mut seen: Vec<(Backend, R)> = Vec::new();
    for b in backends() {
        let dir = tempfile::tempdir().unwrap();
        let db = open(&dir, b);
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

fn try_tx(db: &Db, f: impl FnOnce(&mut Tx<'_>) -> Result<()>) -> Result<TxReport> {
    db.transact(TxOptions::default(), f)
}

fn text(t: &str) -> ElpiProgram {
    ElpiProgram::Text(t.to_string())
}

fn values(r: &ElpiQueryResult, var: &str) -> Vec<Value> {
    let mut out: Vec<Value> = r
        .rows
        .iter()
        .filter_map(|row| match row.bindings.get(var) {
            Some(ElpiTerm::Value(v)) => Some(v.clone()),
            _ => None,
        })
        .collect();
    out.sort_by_key(|v| v.to_string());
    out
}

const REPORTS: &str = r#"
pred reports o:term, o:term.
reports X Y :- tm.triple X (iri "urn:tiramemsu:v:manager") Y _.
reports X Z :- tm.triple X (iri "urn:tiramemsu:v:manager") Y _, reports Y Z.
"#;

fn managers(db: &Db) {
    tx(db, |t| {
        t.assert(v("alice"), v("manager"), v("bob"), Valid::ALWAYS)?;
        t.assert(v("bob"), v("manager"), v("carol"), Valid::ALWAYS)?;
        Ok(())
    });
}

// @tg: verifies:: [[openspec:elpi-engine#Goal queries over a view#Transitive closure]] {level: integration}
#[test]
fn transitive_closure() {
    each(|db| {
        managers(db);
        let r = db
            .now()
            .elpi(
                &text(REPORTS),
                r#"reports (iri "urn:tiramemsu:v:alice") Y"#,
                &ElpiOptions::default(),
            )
            .unwrap();
        values(&r, "Y")
    });
}

// @tg: verifies:: [[openspec:elpi-engine#Goal queries over a view#Syntax error]] {level: integration}
#[test]
fn syntax_error() {
    each(|db| {
        let e = db
            .now()
            .elpi(&text("p X :- ."), "p X", &ElpiOptions::default());
        matches!(e, Err(Error::ElpiProgram { .. }))
    });
}

// @tg: verifies:: [[openspec:elpi-engine#Goal queries over a view#Fuel runs out]] {level: integration}
#[test]
fn fuel_runs_out() {
    each(|db| {
        let opts = ElpiOptions {
            fuel: 10_000,
            ..ElpiOptions::default()
        };
        let e = db
            .now()
            .elpi(&text("pred loop.\nloop :- loop."), "loop", &opts);
        matches!(e, Err(Error::ElpiOutOfFuel { .. }))
    });
}

// @tg: verifies:: [[openspec:elpi-engine#Terms cross both ways#Native integers]] {level: integration}
#[test]
fn native_integers() {
    each(|db| {
        tx(db, |t| {
            t.assert(v("a"), v("age"), Value::Int(41), Valid::ALWAYS)?;
            Ok(())
        });
        let r = db
            .now()
            .elpi(
                &text(""),
                r#"tm.triple _ (iri "urn:tiramemsu:v:age") (num N) _, M is N + 1"#,
                &ElpiOptions::default(),
            )
            .unwrap();
        r.rows[0].bindings.get("M").cloned()
    });
}

// @tg: verifies:: [[openspec:elpi-engine#Terms cross both ways#Bound arguments are lookups]] {level: integration}
#[test]
fn bound_arguments_are_lookups() {
    each(|db| {
        managers(db);
        let r = db
            .now()
            .elpi(
                &text(""),
                r#"tm.triple (iri "urn:tiramemsu:v:nobody") P O _"#,
                &ElpiOptions::default(),
            )
            .unwrap();
        (r.rows.len(), r.cost.rows_read)
    });
}

// @tg: verifies:: [[openspec:elpi-engine#Valid time of a run#A closed fact is not read]] {level: integration}
#[test]
fn closed_fact_is_not_read() {
    each(|db| {
        tx(db, |t| {
            t.assert(
                v("a"),
                v("role"),
                v("CEO"),
                Valid {
                    from: Some(100),
                    to: Some(500),
                },
            )?;
            Ok(())
        });
        let q = r#"tm.triple X (iri "urn:tiramemsu:v:role") _ _"#;
        let at = |d| ElpiOptions {
            valid_at: Some(d),
            ..ElpiOptions::default()
        };
        let now = db.now();
        (
            now.elpi(&text(""), q, &at(200)).unwrap().rows.len(),
            now.elpi(&text(""), q, &at(600)).unwrap().rows.len(),
        )
    });
}

const GRAND: &str = r#"
tm.conclude (triple X (iri "urn:tiramemsu:v:grandparent") Z) [E1, E2] :-
  tm.triple X (iri "urn:tiramemsu:v:parent") Y E1,
  tm.triple Y (iri "urn:tiramemsu:v:parent") Z E2.
"#;

fn parents(db: &Db, a: Valid, b: Valid) -> (Eid, Eid) {
    tx(db, |t| {
        let e1 = t.assert(v("a"), v("parent"), v("b"), a)?.eid();
        let e2 = t.assert(v("b"), v("parent"), v("c"), b)?.eid();
        Ok((e1, e2))
    })
}

fn conclude(db: &Db, program: &str) -> Result<ElpiConcludeResult> {
    db.now()
        .elpi_conclude(&text(program), &ElpiOptions::default())
}

// @tg: verifies:: [[openspec:elpi-engine#Conclusions carry stated support#Support is the premises]] {level: integration}
#[test]
fn support_is_the_premises() {
    each(|db| {
        let (e1, e2) = parents(db, Valid::ALWAYS, Valid::ALWAYS);
        let r = conclude(db, GRAND).unwrap();
        assert_eq!(r.conclusions.len(), 1);
        assert_eq!(r.conclusions[0].support, vec![e1, e2]);
        (r.conclusions[0].s.clone(), r.conclusions[0].o.clone())
    });
}

// @tg: verifies:: [[openspec:elpi-engine#Conclusions carry stated support#Forged support]] {level: integration}
#[test]
fn forged_support() {
    each(|db| {
        parents(db, Valid::ALWAYS, Valid::ALWAYS);
        let forged = r#"
tm.conclude (triple X (iri "urn:tiramemsu:v:p") X) [eid 999] :-
  tm.triple X (iri "urn:tiramemsu:v:parent") _ _.
"#;
        matches!(conclude(db, forged), Err(Error::ElpiRun { .. }))
    });
}

// @tg: verifies:: [[openspec:elpi-engine#Conclusions carry stated support#Intervals intersect]] {level: integration}
#[test]
fn intervals_intersect() {
    each(|db| {
        parents(
            db,
            Valid {
                from: Some(2020),
                to: Some(2025),
            },
            Valid {
                from: Some(2022),
                to: None,
            },
        );
        conclude(db, GRAND).unwrap().conclusions[0].valid
    });
}

// @tg: verifies:: [[openspec:elpi-engine#Conclusions carry stated support#Negation is labelled]] {level: integration}
#[test]
fn negation_is_labelled() {
    each(|db| {
        managers(db);
        let p = r#"
tm.conclude (triple X (iri "urn:tiramemsu:v:top") (bool "true")) [E] :-
  tm.triple _ (iri "urn:tiramemsu:v:manager") X E,
  not (tm.triple X (iri "urn:tiramemsu:v:manager") _ _).
"#;
        let r = conclude(db, p).unwrap();
        (
            r.non_monotonic,
            r.conclusions
                .iter()
                .map(|c| c.s.clone())
                .collect::<Vec<_>>(),
        )
    });
}

const ONE_CEO: &str = r#"
tm.violation error "one_ceo" "two CEOs" [E1, E2] :-
  tm.triple X (iri "urn:tiramemsu:v:role") (iri "urn:tiramemsu:v:CEO") E1,
  tm.triple Y (iri "urn:tiramemsu:v:role") (iri "urn:tiramemsu:v:CEO") E2,
  not (X = Y), E1 = eid N1, E2 = eid N2, N1 < N2.
tm.violation-delta "one_ceo" "two CEOs" [E1, E2] :-
  tm.added X (iri "urn:tiramemsu:v:role") (iri "urn:tiramemsu:v:CEO") E1,
  tm.triple Y (iri "urn:tiramemsu:v:role") (iri "urn:tiramemsu:v:CEO") E2,
  not (X = Y).
"#;

// @tg: verifies:: [[openspec:elpi-engine#Full constraint check#Two CEOs]] {level: integration}
#[test]
fn two_ceos() {
    each(|db| {
        tx(db, |t| {
            t.assert(v("a"), v("role"), v("CEO"), Valid::ALWAYS)?;
            t.assert(v("b"), v("role"), v("CEO"), Valid::ALWAYS)?;
            Ok(())
        });
        let r = db
            .now()
            .elpi_check(&text(ONE_CEO), &ElpiOptions::default())
            .unwrap();
        r.violations
            .iter()
            .map(|x| (x.severity, x.name.clone(), x.eids.len()))
            .collect::<Vec<_>>()
    });
}

// @tg: verifies:: [[openspec:elpi-programs#Store an ELPI program#Store and supersede]] {level: integration}
#[test]
fn store_and_supersede() {
    each(|db| {
        managers(db);
        let first = tx(db, |t| t.store_elpi_program(v("p"), REPORTS));
        let t1 = db.summary().unwrap().transactions;
        let second = tx(db, |t| {
            t.store_elpi_program(v("p"), &format!("{REPORTS}\n% edited\n"))
        });
        assert_ne!(first, second);
        let old = db
            .as_of(TimeRef::Tx(t1))
            .elpi_program_info(&v("p"))
            .unwrap();
        (
            old.statement == first,
            db.now().elpi_program_info(&v("p")).unwrap().statement == second,
        )
    });
}

// @tg: verifies:: [[openspec:elpi-programs#Store an ELPI program#Invalid text writes nothing]] {level: integration}
#[test]
fn invalid_text_writes_nothing() {
    each(|db| {
        let e = try_tx(db, |t| t.store_elpi_program(v("p"), "p X :- .").map(|_| ()));
        (
            matches!(e, Err(Error::ElpiProgram { .. })),
            db.now().elpi_program_info(&v("p")).is_err(),
        )
    });
}

// @tg: verifies:: [[openspec:elpi-programs#Rule language conflict#Datalog node refused]] {level: integration}
#[test]
#[cfg(feature = "datalog")]
fn datalog_node_refused() {
    each(|db| {
        tx(db, |t| t.store_program(v("p"), "r(X) :- v:manager(X, _)."));
        let e = try_tx(db, |t| t.store_elpi_program(v("p"), REPORTS).map(|_| ()));
        matches!(e, Err(Error::RuleLanguageConflict { .. }))
    });
}

// @tg: verifies:: [[openspec:elpi-programs#Includes#Library included]] {level: integration}
#[test]
fn library_included() {
    each(|db| {
        managers(db);
        tx(db, |t| {
            t.store_elpi_program(v("lib"), REPORTS)?;
            t.store_elpi_program(
                v("app"),
                "accumulate <urn:tiramemsu:v:lib>.\npred boss o:term.\nboss Y :- reports (iri \"urn:tiramemsu:v:alice\") Y.\n",
            )
        });
        let r = db
            .now()
            .elpi(
                &ElpiProgram::Stored(v("app")),
                "boss Y",
                &ElpiOptions::default(),
            )
            .unwrap();
        values(&r, "Y")
    });
}

// @tg: verifies:: [[openspec:elpi-programs#Includes#Cycle]] {level: integration}
#[test]
fn include_cycle() {
    each(|db| {
        tx(db, |t| t.store_elpi_program(v("b"), "pred q.\nq.\n"));
        tx(db, |t| {
            t.store_elpi_program(v("a"), "accumulate <urn:tiramemsu:v:b>.\n")
        });
        let e = try_tx(db, |t| {
            t.store_elpi_program(v("b"), "accumulate <urn:tiramemsu:v:a>.\npred q.\nq.\n")
                .map(|_| ())
        });
        matches!(e, Err(Error::ProgramCycle { .. }))
    });
}

// @tg: verifies:: [[openspec:elpi-programs#Program information#Closure listed]] {level: integration}
#[test]
fn closure_listed() {
    each(|db| {
        let lib = tx(db, |t| t.store_elpi_program(v("lib"), REPORTS));
        tx(db, |t| {
            t.store_elpi_program(v("app"), "accumulate <urn:tiramemsu:v:lib>.\n")
        });
        let info = db.now().elpi_program_info(&v("app")).unwrap();
        info.includes == vec![(v("lib"), lib)]
    });
}

fn ceo(db: &Db, who: &str) -> Result<TxReport> {
    try_tx(db, |t| {
        t.assert(v(who), v("role"), v("CEO"), Valid::ALWAYS)?;
        Ok(())
    })
}

// @tg: verifies:: [[openspec:elpi-constraints#Enforce an ELPI program#Enforce over violating data]] {level: integration}
#[test]
fn enforce_over_violating_data() {
    each(|db| {
        ceo(db, "a").unwrap();
        ceo(db, "b").unwrap();
        tx(db, |t| t.store_elpi_program(v("policy"), ONE_CEO));
        let e = try_tx(db, |t| t.enforce_elpi(v("policy"), true).map(|_| ()));
        (
            matches!(e, Err(Error::ConstraintViolated { .. })),
            db.now().elpi_program_info(&v("policy")).unwrap().enforced,
        )
    });
}

// @tg: verifies:: [[openspec:elpi-constraints#Enforce an ELPI program#Unknown program]] {level: integration}
#[test]
fn enforce_unknown_program() {
    each(|db| {
        let e = try_tx(db, |t| t.enforce_elpi(v("none"), true).map(|_| ()));
        matches!(e, Err(Error::UnknownProgram { .. }))
    });
}

fn enforced(db: &Db) {
    ceo(db, "a").unwrap();
    tx(db, |t| {
        t.store_elpi_program(v("policy"), ONE_CEO)?;
        t.enforce_elpi(v("policy"), true)
    });
}

// @tg: verifies:: [[openspec:elpi-constraints#Delta checks at commit#Second CEO refused]] {level: integration}
#[test]
fn second_ceo_refused() {
    each(|db| {
        enforced(db);
        match ceo(db, "b") {
            Err(Error::ConstraintViolated {
                constraint,
                violations,
                ..
            }) => (constraint, violations[0].eids.len()),
            other => panic!("{other:?}"),
        }
    });
}

// @tg: verifies:: [[openspec:elpi-constraints#Delta checks at commit#Unrelated write commits]] {level: integration}
#[test]
fn unrelated_write_commits() {
    each(|db| {
        enforced(db);
        try_tx(db, |t| {
            t.assert(v("a"), v("name"), Value::str("Ann"), Valid::ALWAYS)?;
            Ok(())
        })
        .is_ok()
    });
}

// @tg: verifies:: [[openspec:elpi-constraints#Delta checks at commit#Retraction seeds a check]] {level: integration}
#[test]
fn retraction_seeds_a_check() {
    each(|db| {
        let owner = tx(db, |t| {
            t.assert(v("x"), v("type"), v("Project"), Valid::ALWAYS)?;
            Ok(t.assert(v("x"), v("owner"), v("ann"), Valid::ALWAYS)?.eid())
        });
        let p = r#"
tm.violation-delta "owned" "project without owner" [E] :-
  tm.retracted P (iri "urn:tiramemsu:v:owner") _ _,
  tm.triple P (iri "urn:tiramemsu:v:type") (iri "urn:tiramemsu:v:Project") E,
  not (tm.triple P (iri "urn:tiramemsu:v:owner") _ _).
"#;
        tx(db, |t| {
            t.store_elpi_program(v("owned"), p)?;
            t.enforce_elpi(v("owned"), true)
        });
        let e = try_tx(db, |t| t.retract(owner).map(|_| ()));
        matches!(e, Err(Error::ConstraintViolated { .. }))
    });
}

// @tg: verifies:: [[openspec:elpi-constraints#ELPI constraint budget#Looping delta form]] {level: integration}
#[test]
fn looping_delta_form() {
    each(|db| {
        let p = "pred spin.\nspin :- spin.\ntm.violation-delta \"spin\" \"never\" [] :- tm.added _ _ _ _, spin.\n";
        tx(db, |t| {
            t.store_elpi_program(v("spin"), p)?;
            t.enforce_elpi(v("spin"), true)
        });
        let e = ceo(db, "a");
        matches!(e, Err(Error::ConstraintBudgetExceeded { .. }))
    });
}

const GRAND_STORED: &str = GRAND;

fn grand(db: &Db) -> (Eid, Eid, Eid) {
    let (e1, e2) = parents(db, Valid::ALWAYS, Valid::ALWAYS);
    let src = tx(db, |t| t.store_elpi_program(v("g"), GRAND_STORED));
    (e1, e2, src)
}

fn live_grandparents(db: &Db) -> usize {
    db.now()
        .elpi(
            &text(""),
            r#"tm.triple _ (iri "urn:tiramemsu:v:grandparent") _ _"#,
            &ElpiOptions::default(),
        )
        .unwrap()
        .rows
        .len()
}

// @tg: verifies:: [[openspec:elpi-materialization#Materialize an ELPI program#Grandparents]] {level: integration}
#[test]
fn grandparents_materialized() {
    each(|db| {
        let (e1, e2, src) = grand(db);
        let r = db
            .materialize_elpi(&v("g"), &MaterializeElpiOptions::default())
            .unwrap();
        assert_eq!(r.derived.len(), 1);
        let deps = db.now().dependents(e1).unwrap();
        (
            live_grandparents(db),
            deps.contains(&r.derived[0]),
            db.now().dependents(e2).unwrap().contains(&r.derived[0]),
            db.now().dependents(src).unwrap().contains(&r.derived[0]),
        )
    });
}

// @tg: verifies:: [[openspec:elpi-materialization#Materialize an ELPI program#Idempotent]] {level: integration}
#[test]
fn materialize_idempotent() {
    each(|db| {
        grand(db);
        db.materialize_elpi(&v("g"), &MaterializeElpiOptions::default())
            .unwrap();
        let r = db
            .materialize_elpi(&v("g"), &MaterializeElpiOptions::default())
            .unwrap();
        (r.derived.len(), r.existing.len(), r.reconciled.len())
    });
}

// @tg: verifies:: [[openspec:elpi-materialization#Conclusions fall with premises and programs#Premise retracted]] {level: integration}
#[test]
fn premise_retracted() {
    each(|db| {
        let (_, e2, _) = grand(db);
        db.materialize_elpi(&v("g"), &MaterializeElpiOptions::default())
            .unwrap();
        tx(db, |t| t.retract(e2));
        live_grandparents(db)
    });
}

// @tg: verifies:: [[openspec:elpi-materialization#Conclusions fall with premises and programs#Library edited]] {level: integration}
#[test]
fn library_edited() {
    each(|db| {
        parents(db, Valid::ALWAYS, Valid::ALWAYS);
        tx(db, |t| {
            t.store_elpi_program(
                v("lib"),
                "pred parent o:term, o:term, o:stmt.\nparent X Y E :- tm.triple X (iri \"urn:tiramemsu:v:parent\") Y E.\n",
            )?;
            t.store_elpi_program(
                v("app"),
                "accumulate <urn:tiramemsu:v:lib>.\ntm.conclude (triple X (iri \"urn:tiramemsu:v:grandparent\") Z) [E1, E2] :- parent X Y E1, parent Y Z E2.\n",
            )
        });
        db.materialize_elpi(&v("app"), &MaterializeElpiOptions::default())
            .unwrap();
        let before = live_grandparents(db);
        tx(db, |t| {
            t.store_elpi_program(
                v("lib"),
                "pred parent o:term, o:term, o:stmt.\nparent X Y E :- tm.triple X (iri \"urn:tiramemsu:v:parent\") Y E. % v2\n",
            )
        });
        (before, live_grandparents(db))
    });
}

// @tg: verifies:: [[openspec:elpi-materialization#Reconcile every run#Negation reconciled]] {level: integration}
#[test]
fn negation_reconciled() {
    each(|db| {
        tx(db, |t| {
            t.assert(v("x"), v("type"), v("Person"), Valid::ALWAYS)?;
            Ok(())
        });
        let p = r#"
tm.conclude (triple X (iri "urn:tiramemsu:v:unmanaged") (bool "true")) [E] :-
  tm.triple X (iri "urn:tiramemsu:v:type") (iri "urn:tiramemsu:v:Person") E,
  not (tm.triple X (iri "urn:tiramemsu:v:manager") _ _).
"#;
        tx(db, |t| t.store_elpi_program(v("u"), p));
        let first = db
            .materialize_elpi(&v("u"), &MaterializeElpiOptions::default())
            .unwrap();
        tx(db, |t| {
            t.assert(v("x"), v("manager"), v("boss"), Valid::ALWAYS)?;
            Ok(())
        });
        let second = db
            .materialize_elpi(&v("u"), &MaterializeElpiOptions::default())
            .unwrap();
        (
            first.derived.len(),
            second.reconciled == first.derived,
            !first.needs_rerun_after_writes.is_empty(),
        )
    });
}

// @tg: verifies:: [[openspec:elpi-materialization#Own conclusions are not input#No self-support]] {level: integration}
#[test]
fn no_self_support() {
    each(|db| {
        grand(db);
        tx(db, |t| {
            t.store_elpi_program(
                v("g"),
                &format!("{GRAND}\ntm.conclude (triple X (iri \"urn:tiramemsu:v:again\") Z) [E] :- tm.triple X (iri \"urn:tiramemsu:v:grandparent\") Z E.\n"),
            )
        });
        let r = db
            .materialize_elpi(&v("g"), &MaterializeElpiOptions::default())
            .unwrap();
        let again = db
            .materialize_elpi(&v("g"), &MaterializeElpiOptions::default())
            .unwrap();
        (r.derived.len(), again.derived.len())
    });
}

/// The closure of a pseudo-random graph as Datalog computes it and as ELPI
/// computes it must be the same set of pairs.
// @tg: verifies:: [[openspec:elpi-engine#Goal queries over a view]] {level: integration}
#[test]
#[cfg(feature = "datalog")]
fn datalog_oracle_agrees() {
    each(|db| {
        tx(db, |t| {
            let mut x: u64 = 7;
            for _ in 0..40 {
                x = x
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                let (a, b) = ((x >> 33) % 12, (x >> 17) % 12);
                t.assert(
                    v(&format!("n{a}")),
                    v("manager"),
                    v(&format!("n{b}")),
                    Valid::ALWAYS,
                )?;
            }
            Ok(())
        });
        let dl = db
            .now()
            .datalog(
                &DatalogProgram::Text(
                    "reports(X, Y) :- v:manager(X, Y).\nreports(X, Z) :- reports(X, Y), v:manager(Y, Z).".into(),
                ),
                Some("reports(X, Y)"),
                &DatalogOptions::default(),
            )
            .unwrap();
        let mut want: Vec<(String, String)> = dl
            .rows
            .iter()
            .map(|r| (format!("{:?}", r.values[0]), format!("{:?}", r.values[1])))
            .collect();
        want.sort();
        want.dedup();
        // a tabled closure: ELPI's depth-first search would loop on cycles
        let program = r#"
pred reach i:term, i:list term, o:term.
reach X _ Y :- tm.triple X (iri "urn:tiramemsu:v:manager") Y _.
reach X Seen Z :- tm.triple X (iri "urn:tiramemsu:v:manager") Y _,
  not (std.mem Seen Y), reach Y [Y | Seen] Z.
pred reports o:term, o:term.
reports X Y :- tm.triple X (iri "urn:tiramemsu:v:manager") _ _, reach X [X] Y.
"#;
        let r = db
            .now()
            .elpi(&text(program), "reports X Y", &ElpiOptions::default())
            .unwrap();
        let mut got: Vec<(String, String)> = r
            .rows
            .iter()
            .map(|row| {
                let g = |k: &str| match row.bindings.get(k) {
                    Some(ElpiTerm::Value(v)) => format!("{:?}", DatalogTerm::Value(v.clone())),
                    other => format!("{other:?}"),
                };
                (g("X"), g("Y"))
            })
            .collect();
        got.sort();
        got.dedup();
        assert_eq!(got, want);
        got.len()
    });
}
