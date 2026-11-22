//! The worked example of the article `article/sanctions-audit/` (sanctions
//! screening at a payments firm): every program, query and claim the article
//! makes is checked here, one test per section of the article.
//!
//! Story (all instants UTC, 2026): a registry feed writes the ownership graph
//! in January; payment pay1042 to Halcyon Shipping is screened clear on 3
//! March; the designation of Marten Vale takes effect on 1 March but reaches
//! the system on 5 March; a second payment is refused at commit on 20 March;
//! a misread filing blocks Harbor Freight on 2 April and is corrected; the
//! screening policy moves from "more than 50%" to "50% or more" on 4 May.
#![cfg(feature = "elpi")]

use std::sync::Arc;

use tiramemsu::*;

fn v(s: &str) -> Value {
    Value::iri(format!("urn:tiramemsu:v:{s}"))
}

fn ms(d: &str) -> i64 {
    value::parse_datetime(d).unwrap().0
}

fn one_instant(d: &str) -> Valid {
    Valid::between(ms(d), ms(d) + 1)
}

/// The screening library, version 1: designated parties, and anything more
/// than 50% owned by a blocked party, are blocked. Each predicate threads the
/// statements it used, so a decision can cite them.
const SCREENING_V1: &str = r#"
pred designated o:term, o:list stmt.
designated X [E] :- tm.triple X (iri "urn:tiramemsu:v:designatedBy") _ E.

pred owns o:term, o:term, o:int, o:list stmt.
owns X Y P [E1, E2] :-
  tm.triple X (iri "urn:tiramemsu:v:owns") Y E1,
  tm.triple (stmtv E1) (iri "urn:tiramemsu:v:share") (num P) E2.

pred controlling o:int.
controlling P :- P > 50.

pred blocked o:term, o:list stmt.
blocked X Es :- designated X Es.
blocked Y Es :- owns X Y P Own, controlling P, blocked X Up, std.append Own Up Es.
"#;

/// Version 2 changes one line: 50% or more.
const SCREENING_V2: &str = r#"
pred designated o:term, o:list stmt.
designated X [E] :- tm.triple X (iri "urn:tiramemsu:v:designatedBy") _ E.

pred owns o:term, o:term, o:int, o:list stmt.
owns X Y P [E1, E2] :-
  tm.triple X (iri "urn:tiramemsu:v:owns") Y E1,
  tm.triple (stmtv E1) (iri "urn:tiramemsu:v:share") (num P) E2.

pred controlling o:int.
controlling P :- P >= 50.

pred blocked o:term, o:list stmt.
blocked X Es :- designated X Es.
blocked Y Es :- owns X Y P Own, controlling P, blocked X Up, std.append Own Up Es.
"#;

/// The commit guard: a payment to a blocked party does not commit.
const GUARD: &str = r#"
accumulate <urn:tiramemsu:v:screening>.
tm.violation-delta "sanctions" "payment to a blocked party" [E | Es] :-
  tm.added _ (iri "urn:tiramemsu:v:paidTo") B E, blocked B Es.
"#;

/// The look-back: past payments to parties we now know were blocked then.
const LOOKBACK: &str = r#"
accumulate <urn:tiramemsu:v:screening>.
tm.violation warn "lookback" "paid a party that was blocked at the time" [E | Es] :-
  tm.triple _ (iri "urn:tiramemsu:v:paidTo") B E, blocked B Es.
"#;

/// Blocked status remembered as derived beliefs.
const BLOCKLIST: &str = r#"
accumulate <urn:tiramemsu:v:screening>.
tm.conclude (triple X (iri "urn:tiramemsu:v:blocked") (bool "true")) Es :- blocked X Es.
"#;

struct Bank {
    _dir: tempfile::TempDir,
    db: Db,
    clock: Arc<ManualClock>,
}

impl Bank {
    fn at(&self, d: &str) {
        self.clock.set(ms(d));
    }

    fn tx<R>(&self, f: impl FnOnce(&mut Tx<'_>) -> Result<R>) -> R {
        let mut out = None;
        self.db
            .transact(TxOptions::default(), |t| {
                out = Some(f(t)?);
                Ok(())
            })
            .unwrap();
        out.unwrap()
    }

    fn try_tx(&self, f: impl FnOnce(&mut Tx<'_>) -> Result<()>) -> Result<TxReport> {
        self.db.transact(TxOptions::default(), f)
    }

    fn sparql(&self, view: &View<'_>, q: &str) -> Vec<Vec<String>> {
        match view.sparql(q).unwrap_or_else(|e| panic!("{q}: {e}")) {
            SparqlResult::Solutions(s) => s
                .rows
                .iter()
                .map(|r| {
                    r.iter()
                        .map(|c| c.as_ref().map(|v| v.to_string()).unwrap_or_default())
                        .collect()
                })
                .collect(),
            other => panic!("not solutions: {other:?}"),
        }
    }
}

fn author(t: &mut Tx<'_>, who: &str, source: &str) -> Result<()> {
    t.meta(Value::iri(vocab::SYS_AUTHOR), v(who))?;
    t.meta(Value::iri(vocab::SYS_SOURCE), Value::str(source))?;
    Ok(())
}

/// Owner `x` holds `share` percent of `y` from `from`.
fn stake(t: &mut Tx<'_>, x: &str, y: &str, share: i64, from: &str) -> Result<Eid> {
    let e = t
        .assert(v(x), v("owns"), v(y), Valid::from(ms(from)))?
        .eid();
    t.assert(e, v("share"), Value::Int(share), Valid::from(ms(from)))?;
    Ok(e)
}

/// January: the registry feed writes the ownership graph and compliance
/// stores the policy, the guard and the blocklist program.
fn bank() -> Bank {
    let dir = tempfile::tempdir().unwrap();
    let clock = Arc::new(ManualClock::new(ms("2026-01-12T09:00:00Z")));
    let db = Db::open(
        dir.path().join("bank.db"),
        OpenOptions {
            clock: clock.clone(),
            ..OpenOptions::default()
        },
    )
    .unwrap();
    let b = Bank {
        _dir: dir,
        db,
        clock,
    };
    b.tx(|t| {
        author(t, "registry-feed", "filing-7731")?;
        stake(t, "vale", "brightwater", 55, "2025-06-01T00:00:00Z")?;
        stake(t, "brightwater", "halcyon", 60, "2025-09-15T00:00:00Z")?;
        stake(t, "vale", "corvid", 50, "2025-11-01T00:00:00Z")?;
        Ok(())
    });
    b.at("2026-01-20T09:00:00Z");
    b.tx(|t| {
        t.meta(Value::iri(vocab::SYS_AUTHOR), v("compliance"))?;
        t.meta(
            Value::iri(vocab::SYS_REASON),
            Value::str("screening policy v1"),
        )?;
        t.store_elpi_program(v("screening"), SCREENING_V1)?;
        t.store_elpi_program(v("guard"), GUARD)?;
        t.store_elpi_program(v("lookback"), LOOKBACK)?;
        t.store_elpi_program(v("blocklist"), BLOCKLIST)?;
        t.enforce_elpi(v("guard"), true)
    });
    b
}

fn screen(view: &View<'_>, who: &str, at: &str) -> ElpiQueryResult {
    view.elpi(
        &ElpiProgram::Stored(v("screening")),
        &format!(r#"blocked (iri "urn:tiramemsu:v:{who}") Es"#),
        &ElpiOptions {
            valid_at: Some(ms(at)),
            ..ElpiOptions::default()
        },
    )
    .unwrap()
}

/// 3 March: pay1042 is screened, found clear and released, in one
/// transaction that says who released it and why. Returns the payment
/// statement and the transaction.
fn release(b: &Bank, pay: &str, to: &str, at: &str) -> (Eid, u64) {
    b.at(at);
    let clear = screen(&b.db.now(), to, at).rows.is_empty();
    assert!(clear, "{to} should screen clear at {at}");
    let mut eid = None;
    let r =
        b.db.transact(TxOptions::default(), |t| {
            t.meta(Value::iri(vocab::SYS_AUTHOR), v("payments"))?;
            t.meta(Value::iri(vocab::SYS_REASON), Value::str("screened clear"))?;
            eid = Some(t.assert(v(pay), v("paidTo"), v(to), one_instant(at))?.eid());
            Ok(())
        })
        .unwrap();
    (eid.unwrap(), r.t.0)
}

/// 5 March: the designation, in force since 1 March, arrives.
fn designation_arrives(b: &Bank) -> Eid {
    b.at("2026-03-05T08:00:00Z");
    b.tx(|t| {
        author(t, "sanctions-feed", "eu-list-2026-041")?;
        Ok(t.assert(
            v("vale"),
            v("designatedBy"),
            v("eu-list"),
            Valid::from(ms("2026-03-01T00:00:00Z")),
        )?
        .eid())
    })
}

fn rows_text(r: &ElpiQueryResult, var: &str) -> Vec<String> {
    r.rows
        .iter()
        .map(|row| match row.bindings.get(var) {
            Some(ElpiTerm::Value(x)) => x.to_string(),
            Some(other) => format!("{other:?}"),
            None => String::new(),
        })
        .collect()
}

// Section "The decision record": the release says who and why, and the view
// as of its transaction holds the exact rule text that screened it.
#[test]
fn decision_record_and_the_rule_in_force() {
    let b = bank();
    let (pay, t) = release(&b, "pay1042", "halcyon", "2026-03-03T10:00:00Z");
    let q = format!(
        "SELECT ?who ?why ?when WHERE {{ {} tm:txAdded ?t ; tm:addedAt ?when . \
           ?t sys:author ?who ; sys:reason ?why }}",
        Value::Stmt(pay)
    );
    let rows = b.sparql(&b.db.now(), &q);
    println!("decision record: {rows:?}");
    assert_eq!(rows.len(), 1);
    let rule =
        b.db.as_of(TimeRef::Tx(t))
            .elpi_program_info(&v("screening"))
            .unwrap();
    assert_eq!(rule.text, SCREENING_V1);
}

// Section "Two clocks, three answers": the same question on three views.
#[test]
fn two_clocks_three_answers() {
    let b = bank();
    release(&b, "pay1042", "halcyon", "2026-03-03T10:00:00Z");
    let d = designation_arrives(&b);

    // what we knew when we decided: clear
    let then = b.db.as_of(TimeRef::Instant(ms("2026-03-03T10:00:00Z")));
    assert!(screen(&then, "halcyon", "2026-03-03T10:00:00Z")
        .rows
        .is_empty());

    // what we know now about that day: blocked, with the chain of evidence
    let now = screen(&b.db.now(), "halcyon", "2026-03-03T10:00:00Z");
    assert_eq!(now.rows.len(), 1);
    // e3, e4: Vale owns 55% of Brightwater; e5, e6: Brightwater owns 60% of
    // Halcyon; then the designation
    assert_eq!(d, Eid::new(21));
    match now.rows[0].bindings.get("Es") {
        Some(ElpiTerm::Other(es)) => assert_eq!(es, "[eid 5, eid 6, eid 3, eid 4, eid 21]"),
        other => panic!("{other:?}"),
    }

    // what we know now about February: clear, the designation was not in force
    assert!(screen(&b.db.now(), "halcyon", "2026-02-15T00:00:00Z")
        .rows
        .is_empty());
}

// Section "When did we learn it?": recorded four days after it took effect.
#[test]
fn when_we_learned_it() {
    let b = bank();
    designation_arrives(&b);
    let rows = b.sparql(
        &b.db.now(),
        "SELECT ?who ?from ?added WHERE { ?who v:designatedBy ?list ~ ?r . \
           ?r tm:validFrom ?from ; tm:addedAt ?added FILTER(?added > ?from) }",
    );
    println!("late facts: {rows:?}");
    assert_eq!(rows.len(), 1);
    let c =
        b.db.now()
            .cypher(
                "MATCH (x)-[r:designatedBy]->(l) \
             RETURN x, (r.addedAt.epochMillis - r.validFrom.epochMillis) / 3600000 AS hoursLate, \
                    tiramemsu.transaction(r).`sys:source` AS source",
                &CypherParams::default(),
            )
            .unwrap();
    println!("cypher: {:?}", c.rows);
    assert_eq!(c.rows.len(), 1);
    assert_eq!(c.rows[0][1], CypherValue::Integer(104));
    assert_eq!(c.rows[0][2], CypherValue::String("eu-list-2026-041".into()));
}

// Section "The look-back": the warn check finds pay1042, and only once the
// designation is known.
#[test]
fn lookback_finds_the_payment() {
    let b = bank();
    let (pay, _) = release(&b, "pay1042", "halcyon", "2026-03-03T10:00:00Z");
    release(&b, "pay0977", "halcyon", "2026-02-10T10:00:00Z");
    let check = |view: &View<'_>| {
        view.elpi_check(&ElpiProgram::Stored(v("lookback")), &ElpiOptions::default())
            .unwrap()
            .violations
    };
    assert!(check(&b.db.now()).is_empty());
    designation_arrives(&b);
    let found = check(&b.db.now());
    println!("look-back: {found:?}");
    assert_eq!(found.len(), 1, "only the March payment, not February's");
    assert!(found[0].eids.contains(&pay));
    assert_eq!(found[0].valid, one_instant("2026-03-03T10:00:00Z"));
    // a view as of before the list arrived still finds nothing
    let before = b.db.as_of(TimeRef::Instant(ms("2026-03-04T00:00:00Z")));
    assert!(check(&before).is_empty());
}

// Section "The guard": a second payment to Halcyon is refused at commit and
// leaves nothing behind; a payment to an unrelated party commits.
#[test]
fn guard_refuses_at_commit() {
    let b = bank();
    designation_arrives(&b);
    b.at("2026-03-20T11:00:00Z");
    let before = b.db.last_t().unwrap();
    let e = b.try_tx(|t| {
        t.assert(
            v("pay1077"),
            v("paidTo"),
            v("halcyon"),
            one_instant("2026-03-20T11:00:00Z"),
        )?;
        Ok(())
    });
    match &e {
        Err(Error::ConstraintViolated {
            constraint,
            violations,
            ..
        }) => println!("refused: {constraint} {violations:?}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(b.db.last_t().unwrap(), before);
    b.try_tx(|t| {
        t.assert(
            v("pay1078"),
            v("paidTo"),
            v("northwind"),
            one_instant("2026-03-20T11:05:00Z"),
        )?;
        Ok(())
    })
    .unwrap();
}

// Section "What if": exposure if Brightwater itself were listed, without a write.
#[test]
fn what_if_brightwater_were_listed() {
    let b = bank();
    let before = b.db.last_t().unwrap();
    let r =
        b.db.now()
            .elpi(
                &ElpiProgram::Stored(v("screening")),
                r#"designated (iri "urn:tiramemsu:v:brightwater") [] => blocked C _"#,
                &ElpiOptions {
                    valid_at: Some(ms("2026-03-10T00:00:00Z")),
                    ..ElpiOptions::default()
                },
            )
            .unwrap();
    let c = rows_text(&r, "C");
    println!("exposure: {c:?}");
    assert_eq!(
        c,
        vec![v("brightwater").to_string(), v("halcyon").to_string()]
    );
    assert_eq!(b.db.last_t().unwrap(), before);
}

// Section "Test the rule, not the data": properties of each policy version
// for any owner and any company.
#[test]
fn rule_properties_per_version() {
    let b = bank();
    let prop = |program: &str, goal: &str| {
        !b.db
            .now()
            .elpi(
                &ElpiProgram::Text(program.into()),
                goal,
                &ElpiOptions::default(),
            )
            .unwrap()
            .rows
            .is_empty()
    };
    let majority = r#"pi x\ pi y\ (designated x [], owns x y 51 []) => blocked y _"#;
    let half = r#"pi x\ pi y\ (designated x [], owns x y 50 []) => blocked y _"#;
    let chain =
        r#"pi x\ pi y\ pi z\ (designated x [], owns x y 80 [], owns y z 80 []) => blocked z _"#;
    assert!(prop(SCREENING_V1, majority));
    assert!(!prop(SCREENING_V1, half));
    assert!(prop(SCREENING_V1, chain));
    assert!(prop(SCREENING_V2, majority));
    assert!(prop(SCREENING_V2, half));
}

// Section "Debugging a false positive".
#[test]
fn debugging_a_false_positive() {
    let b = bank();
    designation_arrives(&b);
    // 28 March: an extraction agent misreads a filing of Vale Logistics Ltd
    b.at("2026-03-28T16:00:00Z");
    let wrong = b.tx(|t| {
        author(t, "registry-extractor", "filing-9132")?;
        t.meta(v("modelVersion"), Value::str("2.3"))?;
        stake(t, "vale", "harbor", 70, "2026-01-01T00:00:00Z")
    });
    b.at("2026-03-30T09:00:00Z");
    let mat =
        b.db.materialize_elpi(&v("blocklist"), &MaterializeElpiOptions::default())
            .unwrap();
    println!("materialized: {:?}", mat.derived);

    // 2 April: a payment to Harbor Freight is refused
    b.at("2026-04-02T10:00:00Z");
    let e = b.try_tx(|t| {
        t.assert(
            v("pay1090"),
            v("paidTo"),
            v("harbor"),
            one_instant("2026-04-02T10:00:00Z"),
        )?;
        Ok(())
    });
    let cited = match e {
        Err(Error::ConstraintViolated { violations, .. }) => violations[0].eids.clone(),
        other => panic!("{other:?}"),
    };
    println!("cited: {cited:?}");
    assert!(cited.contains(&wrong));

    // who wrote the cited ownership, from what, with which model
    let prov = b.sparql(
        &b.db.now(),
        "SELECT ?who ?src ?model WHERE { v:vale v:owns v:harbor ~ ?r . ?r tm:txAdded ?t . \
           ?t sys:author ?who ; sys:source ?src ; v:modelVersion ?model }",
    );
    assert_eq!(
        prov,
        vec![vec![
            v("registry-extractor").to_string(),
            Value::str("filing-9132").to_string(),
            Value::str("2.3").to_string(),
        ]]
    );

    // everything that model version wrote
    let blast = b.sparql(
        &b.db.now(),
        "SELECT ?s ?p ?o WHERE { ?s ?p ?o ~ ?r . ?r tm:txAdded ?t . \
           ?t v:modelVersion \"2.3\" FILTER(?s != ?t) }",
    );
    println!("blast radius: {blast:?}");

    // what correcting it would take with it
    let deps = b.db.now().dependents(wrong).unwrap();
    println!("dependents: {deps:?}");
    assert!(deps.iter().any(|d| mat.derived.contains(d)));

    // the fix: the owner was Vale Logistics, a different party
    b.at("2026-04-02T14:00:00Z");
    b.tx(|t| {
        t.meta(Value::iri(vocab::SYS_AUTHOR), v("analyst-kim"))?;
        t.meta(
            Value::iri(vocab::SYS_REASON),
            Value::str("entity resolution: Vale Logistics Ltd is not Marten Vale"),
        )?;
        t.retract(wrong)?;
        stake(t, "vale-logistics", "harbor", 70, "2026-01-01T00:00:00Z")?;
        Ok(())
    });
    // the derived belief fell with its premise
    let blocked = b.sparql(&b.db.now(), "SELECT ?x WHERE { ?x v:blocked true }");
    println!("blocked now: {blocked:?}");
    // the payment now commits
    b.at("2026-04-02T14:05:00Z");
    b.try_tx(|t| {
        t.assert(
            v("pay1090"),
            v("paidTo"),
            v("harbor"),
            one_instant("2026-04-02T14:05:00Z"),
        )?;
        Ok(())
    })
    .unwrap();
    // and the refusal can be replayed as of the moment it happened
    let then = b.db.as_of(TimeRef::Instant(ms("2026-04-02T10:00:00Z")));
    assert_eq!(
        screen(&then, "harbor", "2026-04-02T10:00:00Z").rows.len(),
        1
    );
    assert!(screen(&b.db.now(), "harbor", "2026-04-02T10:00:00Z")
        .rows
        .is_empty());
    // why was it retracted?
    let why = b.sparql(
        &b.db.history(),
        "SELECT ?x ?y ?who ?why WHERE { ?x v:owns ?y ~ ?r . ?r tm:txRetracted ?t . \
           ?t sys:author ?who ; sys:reason ?why }",
    );
    println!("why: {why:?}");
    assert_eq!(why.len(), 1);
}

// Section "The rule changed": v1 to v2, and which rule decided what.
#[test]
fn policy_change_is_auditable() {
    let b = bank();
    let (pay1042, _) = release(&b, "pay1042", "halcyon", "2026-03-03T10:00:00Z");
    designation_arrives(&b);
    // 20 April: Corvid is 50% owned by Vale; under v1 that is not control
    let (pay1100, t1100) = release(&b, "pay1100", "corvid", "2026-04-20T10:00:00Z");
    b.at("2026-04-21T09:00:00Z");
    b.db.materialize_elpi(&v("blocklist"), &MaterializeElpiOptions::default())
        .unwrap();
    let blocked = |view: &View<'_>| -> Vec<String> {
        let mut r: Vec<String> = b
            .sparql(view, "SELECT ?x WHERE { ?x v:blocked true }")
            .into_iter()
            .map(|r| r[0].clone())
            .collect();
        r.sort();
        r
    };
    assert_eq!(blocked(&b.db.now()).len(), 3);

    // 4 May: compliance adopts "50% or more"
    b.at("2026-05-04T09:00:00Z");
    b.tx(|t| {
        t.meta(Value::iri(vocab::SYS_AUTHOR), v("compliance"))?;
        t.meta(
            Value::iri(vocab::SYS_REASON),
            Value::str("policy v2: 50% or more, for USD clearing"),
        )?;
        t.store_elpi_program(v("screening"), SCREENING_V2)?;
        Ok(())
    });
    // the conclusions of v1 fell with the rule they rested on
    assert!(blocked(&b.db.now()).is_empty());
    b.at("2026-05-04T09:10:00Z");
    b.db.materialize_elpi(&v("blocklist"), &MaterializeElpiOptions::default())
        .unwrap();
    let now = blocked(&b.db.now());
    println!("blocked under v2: {now:?}");
    assert_eq!(now.len(), 4);

    // what changed between 1 May and now
    let diff = b.sparql(
        &b.db.now(),
        "SELECT ?x WHERE { ?x v:blocked true . \
           OPTIONAL { SERVICE <urn:tiramemsu:tm:asOf/2026-05-01T00:00:00Z> { ?x v:blocked ?then } } \
           FILTER(!BOUND(?then)) }",
    );
    println!("newly blocked: {diff:?}");
    assert_eq!(diff, vec![vec![v("corvid").to_string()]]);

    // which rule screened pay1100: the one live as of its transaction
    let v1 =
        b.db.as_of(TimeRef::Tx(t1100))
            .elpi_program_info(&v("screening"))
            .unwrap();
    let v2 = b.db.now().elpi_program_info(&v("screening")).unwrap();
    assert_eq!(v1.text, SCREENING_V1);
    assert_eq!(v2.text, SCREENING_V2);
    let then = b.db.as_of(TimeRef::Instant(ms("2026-04-20T10:00:00Z")));

    // today's facts under the old rule: still clear, so the change is the rule
    let old_rule =
        b.db.now()
            .elpi(
                &ElpiProgram::Text(v1.text.clone()),
                r#"blocked (iri "urn:tiramemsu:v:corvid") Es"#,
                &ElpiOptions {
                    valid_at: Some(ms("2026-04-20T10:00:00Z")),
                    ..ElpiOptions::default()
                },
            )
            .unwrap();
    assert!(old_rule.rows.is_empty());
    // and the new rule on what we knew on 20 April already blocks
    let new_rule_then = then
        .elpi(
            &ElpiProgram::Text(v2.text.clone()),
            r#"blocked (iri "urn:tiramemsu:v:corvid") Es"#,
            &ElpiOptions {
                valid_at: Some(ms("2026-04-20T10:00:00Z")),
                ..ElpiOptions::default()
            },
        )
        .unwrap();
    assert_eq!(new_rule_then.rows.len(), 1);

    // replay: the decision as taken (v1 text, facts known then) is clear;
    // today's rule on that day's facts blocks
    assert!(screen(&then, "corvid", "2026-04-20T10:00:00Z")
        .rows
        .is_empty());
    assert_eq!(
        screen(&b.db.now(), "corvid", "2026-04-20T10:00:00Z")
            .rows
            .len(),
        1
    );

    // the look-back now flags both payments, each for its own reason
    let found =
        b.db.now()
            .elpi_check(&ElpiProgram::Stored(v("lookback")), &ElpiOptions::default())
            .unwrap()
            .violations;
    assert_eq!(found.len(), 2);
    assert!(found.iter().any(|x| x.eids.contains(&pay1042)));
    assert!(found.iter().any(|x| x.eids.contains(&pay1100)));

    // the policy's own history: who changed it and why
    let lineage = b.sparql(
        &b.db.history(),
        "SELECT ?who ?why WHERE { v:screening <urn:tiramemsu:elpi:source> ?text ~ ?r . \
           ?r tm:txAdded ?t . ?t sys:author ?who ; sys:reason ?why } ORDER BY ?why",
    );
    println!("policy history: {lineage:?}");
    assert_eq!(lineage.len(), 2);
}

// Section "Limits": a statement-valued annotation follows a supersede, so a
// citation of a rule version by its statement moves to the new version in
// the current view; history keeps the original.
#[test]
fn citation_by_statement_follows_supersede() {
    let b = bank();
    let v1 =
        b.db.now()
            .elpi_program_info(&v("screening"))
            .unwrap()
            .statement;
    b.at("2026-03-03T10:00:00Z");
    b.tx(|t| {
        t.meta(v("screenedWith"), v1)?;
        t.assert(
            v("pay1042"),
            v("paidTo"),
            v("halcyon"),
            one_instant("2026-03-03T10:00:00Z"),
        )?;
        Ok(())
    });
    b.at("2026-05-04T09:00:00Z");
    b.tx(|t| t.store_elpi_program(v("screening"), SCREENING_V2));
    let v2 =
        b.db.now()
            .elpi_program_info(&v("screening"))
            .unwrap()
            .statement;
    let q = "SELECT ?policy WHERE { ?t v:screenedWith ?policy }";
    assert_eq!(
        b.sparql(&b.db.now(), q),
        vec![vec![Value::Stmt(v2).to_string()]]
    );
    let mut all = b.sparql(&b.db.history(), q);
    all.sort();
    let mut want = vec![
        vec![Value::Stmt(v1).to_string()],
        vec![Value::Stmt(v2).to_string()],
    ];
    want.sort();
    assert_eq!(all, want);
}
