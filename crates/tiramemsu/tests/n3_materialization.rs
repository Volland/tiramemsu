//! Spec `n3-materialization` (OpenSpec change `add-n3-reasoning`):
//! `Db::materialize` writes the conclusions of a stored rule set as derived
//! beliefs supported by their premises and the rule-set statement. Every
//! scenario runs on SQLite and, with the `redb` feature, on redb.
#![cfg(feature = "n3")]

mod n3_common;

use std::collections::BTreeSet;

use n3_common::*;
use tiramemsu::n3_engine::vocab::{N3_DERIVED_BY, N3_RULE, N3_SKOLEM};
use tiramemsu::*;

fn store(db: &Db, rule_set: &str, n3: &str) -> (Eid, TxReport) {
    let mut eid = None;
    let report = db
        .transact(TxOptions::default(), |tx| {
            eid = Some(tx.store_rules(v(rule_set), n3)?);
            Ok(())
        })
        .unwrap();
    (eid.unwrap(), report)
}

fn fact(db: &Db, s: &str, p: Value, o: &str, valid: Valid) -> Eid {
    let mut eid = None;
    db.transact(TxOptions::default(), |tx| {
        eid = Some(tx.create(v(s), p, v(o), valid)?);
        Ok(())
    })
    .unwrap();
    eid.unwrap()
}

fn human(db: &Db, s: &str) -> Eid {
    fact(db, s, a(), "Human", Valid::ALWAYS)
}

fn materialize(db: &Db, rule_set: &str) -> MaterializeReport {
    db.materialize(&v(rule_set), &MaterializeOptions::default())
        .unwrap()
}

fn id(db: &Db, value: &Value) -> ObjectId {
    db.now().encode(value).unwrap().expect("interned")
}

/// The live statements `(s p o)`.
fn live(db: &Db, s: &Value, p: &Value, o: &Value) -> Vec<Triple> {
    let now = db.now();
    match (
        now.encode(s).unwrap(),
        now.encode(p).unwrap(),
        now.encode(o).unwrap(),
    ) {
        (Some(s), Some(p), Some(o)) => now.triples(Some(s), Some(p), Some(o)).unwrap(),
        _ => Vec::new(),
    }
}

/// The live supports of `d`.
fn supports(db: &Db, d: Eid) -> BTreeSet<Eid> {
    let df = id(db, &Value::iri(vocab::SYS_DERIVED_FROM));
    db.now()
        .triples(Some(d.oid()), Some(df), None)
        .unwrap()
        .iter()
        .map(|t| Eid::from_oid(t.o).expect("a statement"))
        .collect()
}

/// The objects of the live `(d p ?)` layers.
fn layer(db: &Db, d: Eid, p: &str) -> Vec<Value> {
    let p = id(db, &Value::iri(p));
    db.now()
        .triples(Some(d.oid()), Some(p), None)
        .unwrap()
        .iter()
        .map(|t| db.now().decode(t.o).unwrap())
        .collect()
}

fn is_live(db: &Db, e: Eid) -> bool {
    db.now().dependents(e).unwrap().first() == Some(&e)
}

fn valid_of(t: &Triple) -> Valid {
    Valid {
        from: t.v_from,
        to: t.v_to,
    }
}

// @tg: verifies:: [[openspec:n3-materialization#Materialize a rule set#Materialized conclusion]] {since: "2026-10-28", level: integration}
#[test]
fn materialized_conclusion() {
    each(|name, db| {
        let e1 = human(db, "socrates");
        let (r, _) = store(db, "mortality", MORTALITY);
        let t0 = db.last_t().unwrap();
        let report = materialize(db, "mortality");
        assert_eq!(report.rule_set, v("mortality"), "{name}");
        assert_eq!(report.rules_statement, r);
        assert_eq!(report.reasoned_at, t0);
        assert_eq!(report.status, ReasonStatus::Complete);
        assert!(!report.dry_run);
        let rows = live(db, &v("socrates"), &a(), &v("Mortal"));
        assert_eq!(rows.len(), 1);
        let d = rows[0].eid;
        assert_eq!(report.derived, [d]);
        assert!(report.existing.is_empty() && report.widened.is_empty());
        assert_eq!(supports(db, d), BTreeSet::from([e1, r]));
        assert_eq!(layer(db, d, N3_DERIVED_BY), [v("mortality")]);
        assert_eq!(layer(db, d, N3_RULE), [Value::Int(0)]);
        // the conclusion, two support links and two labels
        assert_eq!(report.commit.unwrap().asserted.len(), 5);
    });
}

// @tg: verifies:: [[openspec:n3-materialization#Materialize a rule set#Re-running is a no-op]] {since: "2026-10-28", level: integration}
#[test]
fn re_running_is_a_no_op() {
    each(|name, db| {
        human(db, "socrates");
        store(db, "mortality", MORTALITY);
        let first = materialize(db, "mortality");
        let second = materialize(db, "mortality");
        assert_eq!(second.existing, first.derived, "{name}");
        assert!(second.derived.is_empty() && second.widened.is_empty());
        let commit = second.commit.unwrap();
        assert!(commit.asserted.is_empty() && commit.retracted.is_empty());
        assert!(commit.superseded.is_empty());
    });
}

// @tg: verifies:: [[openspec:n3-materialization#Materialize a rule set#Dry run]] {since: "2026-10-28", level: integration}
#[test]
fn dry_run() {
    each(|name, db| {
        human(db, "socrates");
        store(db, "mortality", MORTALITY);
        let before = db.history().triples(None, None, None).unwrap();
        let last = db.last_t().unwrap();
        let opts = MaterializeOptions {
            dry_run: true,
            ..MaterializeOptions::default()
        };
        let report = db.materialize(&v("mortality"), &opts).unwrap();
        assert!(report.dry_run, "{name}");
        assert_eq!(report.derived.len(), 1);
        assert_eq!(report.commit.unwrap().asserted.len(), 5);
        assert_eq!(db.history().triples(None, None, None).unwrap(), before);
        assert_eq!(db.last_t().unwrap(), last);
        assert!(live(db, &v("socrates"), &a(), &v("Mortal")).is_empty());
    });
}

// @tg: verifies:: [[openspec:n3-materialization#Conclusions fall with premises and rules#Retracting a premise]] {since: "2026-10-28", level: integration}
#[test]
fn retracting_a_premise() {
    each(|name, db| {
        let e1 = human(db, "socrates");
        store(db, "mortality", MORTALITY);
        let d = materialize(db, "mortality").derived[0];
        let report = db
            .transact(TxOptions::default(), |tx| tx.retract(e1).map(|_| ()))
            .unwrap();
        assert!(
            report.retracted.contains(&(d, RetKind::Cascade)),
            "{name}: {report:?}"
        );
        assert!(!is_live(db, d));
    });
}

// @tg: verifies:: [[openspec:n3-materialization#Conclusions fall with premises and rules#Editing the rules]] {since: "2026-10-28", level: integration}
#[test]
fn editing_the_rules() {
    each(|name, db| {
        human(db, "socrates");
        store(db, "mortality", MORTALITY);
        let d = materialize(db, "mortality").derived[0];
        let (r2, report) = store(db, "mortality", "{ ?x a v:Human } => { ?x a v:Immortal } .");
        assert!(
            report.retracted.contains(&(d, RetKind::Supersede)),
            "{name}: {report:?}"
        );
        assert!(!is_live(db, d));
        let again = materialize(db, "mortality");
        assert_eq!(again.rules_statement, r2);
        assert_eq!(again.derived.len(), 1);
        assert!(live(db, &v("socrates"), &a(), &v("Mortal")).is_empty());
        let rows = live(db, &v("socrates"), &a(), &v("Immortal"));
        assert_eq!(rows.len(), 1);
        assert!(supports(db, rows[0].eid).contains(&r2));
    });
}

// @tg: verifies:: [[openspec:n3-materialization#Own conclusions are not input#Layered rule sets]] {since: "2026-10-28", level: integration}
#[test]
fn layered_rule_sets() {
    each(|name, db| {
        human(db, "socrates");
        store(db, "mortality", MORTALITY);
        let (rb, _) = store(
            db,
            "lifespan",
            "{ ?x a v:Mortal } => { ?x v:has v:lifespan } .",
        );
        let da = materialize(db, "mortality").derived[0];
        let b = materialize(db, "lifespan");
        assert_eq!(b.derived.len(), 1, "{name}");
        assert_eq!(supports(db, b.derived[0]), BTreeSet::from([da, rb]));
        let again = materialize(db, "mortality");
        assert_eq!(again.existing, [da]);
        let commit = again.commit.unwrap();
        assert!(commit.asserted.is_empty() && commit.retracted.is_empty());
        assert!(is_live(db, b.derived[0]));
    });
}

/// Materializing a refused rule set writes nothing.
fn refused(db: &Db, rule_set: &str) -> Error {
    let before = db.history().triples(None, None, None).unwrap();
    let last = db.last_t().unwrap();
    let err = db
        .materialize(&v(rule_set), &MaterializeOptions::default())
        .unwrap_err();
    assert_eq!(db.history().triples(None, None, None).unwrap(), before);
    assert_eq!(db.last_t().unwrap(), last);
    err
}

// @tg: verifies:: [[openspec:n3-materialization#Only monotonic rule sets materialize#Negation refused]] {since: "2026-10-28", level: integration}
#[test]
fn negation_refused() {
    each(|name, db| {
        fact(db, "bob", a(), "Employee", Valid::ALWAYS);
        store(
            db,
            "unmanaged",
            "{ ?x a v:Employee. ?SCOPE log:notIncludes { ?x v:manager ?m } } => { ?x a v:Unmanaged } .",
        );
        match refused(db, "unmanaged") {
            Error::N3NotMaterializable { rule, builtin } => {
                assert_eq!((rule, builtin.as_str()), (0, "log:notIncludes"), "{name}")
            }
            other => panic!("{name}: {other:?}"),
        }
        // the clock and untraced premises are refused the same way
        store(
            db,
            "clock",
            "{ ?x a v:Employee } => { ?x a v:Staff } .\n{ ?x a v:Employee. \"\" time:localTime ?t } => { ?x v:seen ?t } .",
        );
        assert!(matches!(
            refused(db, "clock"),
            Error::N3NotMaterializable { rule: 1, ref builtin } if builtin == "time:localTime"
        ));
        store(
            db,
            "doubling",
            "{ ?x v:items ?l. (?l v:double) list:map ?m } => { ?x v:doubled ?m } .",
        );
        assert!(matches!(
            refused(db, "doubling"),
            Error::N3NotMaterializable { rule: 0, ref builtin } if builtin == "list:map"
        ));
        // and a rule set that was never stored is unknown
        assert!(matches!(
            refused(db, "nothing"),
            Error::UnknownRuleSet { .. }
        ));
    });
}

// @tg: verifies:: [[openspec:n3-materialization#Only monotonic rule sets materialize#Contradiction aborts]] {since: "2026-10-28", level: integration}
#[test]
fn contradiction_aborts() {
    each(|name, db| {
        human(db, "socrates");
        fact(db, "socrates", a(), "Robot", Valid::ALWAYS);
        store(
            db,
            "fused",
            "{ ?x a v:Human } => { ?x a v:Mortal } .\n{ ?x a v:Human. ?x a v:Robot } => false .",
        );
        match refused(db, "fused") {
            Error::N3Contradiction { rule } => assert_eq!(rule, 1, "{name}"),
            other => panic!("{name}: {other:?}"),
        }
    });
}

// @tg: verifies:: [[openspec:n3-materialization#Valid time of materialized conclusions#Two episodes from different support]] {since: "2026-10-28", level: integration}
#[test]
fn two_episodes_from_different_support() {
    each(|name, db| {
        let ta = fact(
            db,
            "alice",
            v("team"),
            "a",
            Valid::between(year(2020), year(2022)),
        );
        let tb = fact(
            db,
            "alice",
            v("team"),
            "b",
            Valid::between(year(2022), year(2024)),
        );
        let la = fact(db, "a", v("locatedIn"), "berlin", Valid::ALWAYS);
        let lb = fact(db, "b", v("locatedIn"), "berlin", Valid::ALWAYS);
        let (r, _) = store(
            db,
            "workplace",
            "{ ?p v:team ?t. ?t v:locatedIn ?l } => { ?p v:worksIn ?l } .",
        );
        let report = materialize(db, "workplace");
        assert_eq!(report.derived.len(), 2, "{name}");
        let rows = live(db, &v("alice"), &v("worksIn"), &v("berlin"));
        let got: Vec<(Valid, BTreeSet<Eid>)> = rows
            .iter()
            .map(|t| (valid_of(t), supports(db, t.eid)))
            .collect();
        assert_eq!(
            got,
            [
                (
                    Valid::between(year(2020), year(2022)),
                    BTreeSet::from([ta, la, r])
                ),
                (
                    Valid::between(year(2022), year(2024)),
                    BTreeSet::from([tb, lb, r])
                ),
            ]
        );
    });
}

// @tg: verifies:: [[openspec:n3-materialization#Valid time of materialized conclusions#Widening across windows]] {since: "2026-10-28", level: integration}
#[test]
fn widening_across_windows() {
    each(|name, db| {
        let e1 = fact(
            db,
            "alice",
            v("badge"),
            "berlin",
            Valid::between(year(2020), year(2024)),
        );
        let (r, _) = store(
            db,
            "workplace",
            "{ ?p v:badge ?l } => { ?p v:worksIn ?l } .",
        );
        let first = db
            .materialize(
                &v("workplace"),
                &MaterializeOptions {
                    window: Valid::between(year(2020), year(2022)),
                    ..MaterializeOptions::default()
                },
            )
            .unwrap();
        let d = first.derived[0];
        let rows = live(db, &v("alice"), &v("worksIn"), &v("berlin"));
        assert_eq!(
            valid_of(&rows[0]),
            Valid::between(year(2020), year(2022)),
            "{name}"
        );
        let second = materialize(db, "workplace");
        assert!(second.derived.is_empty() && second.existing.is_empty());
        assert_eq!(second.widened.len(), 1);
        let (old, new) = second.widened[0];
        assert_eq!(old, d);
        assert!(!is_live(db, d));
        let rows = live(db, &v("alice"), &v("worksIn"), &v("berlin"));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].eid, new);
        assert_eq!(valid_of(&rows[0]), Valid::between(year(2020), year(2024)));
        // the same support and labels, replayed onto the new statement
        assert_eq!(supports(db, new), BTreeSet::from([e1, r]));
        assert_eq!(layer(db, new, N3_DERIVED_BY), [v("workplace")]);
        assert_eq!(layer(db, new, N3_RULE), [Value::Int(0)]);
        // and a third run finds it
        assert_eq!(materialize(db, "workplace").existing, [new]);
    });
}

// @tg: verifies:: [[openspec:n3-materialization#Valid time of materialized conclusions#Require complete]] {since: "2026-10-28", level: integration}
#[test]
fn require_complete() {
    each(|name, db| {
        for i in 0..6 {
            fact(
                db,
                &format!("p{i}"),
                a(),
                "Human",
                Valid::between(year(2000 + 2 * i), year(2001 + 2 * i)),
            );
        }
        store(db, "mortality", MORTALITY);
        let opts = MaterializeOptions {
            max_segments: 3,
            require_complete: true,
            ..MaterializeOptions::default()
        };
        let before = db.history().triples(None, None, None).unwrap();
        let err = db.materialize(&v("mortality"), &opts).unwrap_err();
        match err {
            Error::N3Incomplete { limits, .. } => assert_eq!(limits, ["Segments"], "{name}"),
            other => panic!("{name}: {other:?}"),
        }
        assert_eq!(db.history().triples(None, None, None).unwrap(), before);
        // without it the sound part is written and the status says incomplete
        let report = db
            .materialize(
                &v("mortality"),
                &MaterializeOptions {
                    require_complete: false,
                    ..opts
                },
            )
            .unwrap();
        assert_eq!(
            report.status,
            ReasonStatus::Incomplete {
                limits: vec![ReasonLimit::Segments]
            }
        );
        assert_eq!(report.derived.len(), 1);
    });
}

// @tg: verifies:: [[openspec:n3-materialization#Snapshot then commit#Premise retracted during reasoning]] {since: "2026-10-28", level: integration}
#[test]
fn premise_retracted_during_reasoning() {
    each(|name, db| {
        let e1 = human(db, "socrates");
        let e2 = human(db, "plato");
        store(
            db,
            "mortality",
            "{ ?x a v:Human } => { ?x a v:Mortal } . { ?x a v:Mortal } => { ?x v:has v:lifespan } .",
        );
        let report = db
            .materialize_between(&v("mortality"), &MaterializeOptions::default(), |db| {
                db.transact(TxOptions::default(), |tx| tx.retract(e1).map(|_| ()))
                    .map(|_| ())
            })
            .unwrap();
        // the conclusion on e1 and the one resting on it
        assert_eq!(report.stale_support.len(), 2, "{name}: {report:?}");
        assert_eq!(
            report.stale_support[0].proof.premises,
            [Premise::Statement(e1)]
        );
        assert_eq!(report.stale_support[1].s, N3Term::Value(v("socrates")));
        assert_eq!(report.derived.len(), 2);
        assert!(live(db, &v("socrates"), &a(), &v("Mortal")).is_empty());
        let plato = live(db, &v("plato"), &a(), &v("Mortal"));
        assert_eq!(plato.len(), 1);
        assert!(supports(db, plato[0].eid).contains(&e2));
        assert_eq!(live(db, &v("plato"), &v("has"), &v("lifespan")).len(), 1);
    });
}

// @tg: verifies:: [[openspec:n3-materialization#Skolem nodes#Stable invented node]] {since: "2026-10-28", level: integration}
#[test]
fn stable_invented_node() {
    each(|name, db| {
        let e1 = fact(db, "bob", a(), "Employee", Valid::ALWAYS);
        let (r, _) = store(
            db,
            "managers",
            "{ ?x a v:Employee } => { ?x v:manager _:m } .",
        );
        let first = materialize(db, "managers");
        assert_eq!(first.skolems.len(), 1, "{name}");
        let node = first.skolems[0];
        let bob = id(db, &v("bob"));
        let manager = id(db, &v("manager"));
        let rows = db.now().triples(Some(bob), Some(manager), None).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].o, node);
        assert_eq!(supports(db, rows[0].eid), BTreeSet::from([e1, r]));
        // the node's key is derived from the same support
        let skolem = id(db, &Value::iri(N3_SKOLEM));
        let keys = db.now().triples(Some(node), Some(skolem), None).unwrap();
        assert_eq!(keys.len(), 1);
        assert_eq!(supports(db, keys[0].eid), BTreeSet::from([e1, r]));
        let second = materialize(db, "managers");
        assert_eq!(second.skolems, [node]);
        assert_eq!(second.existing, [rows[0].eid]);
        assert!(second.commit.unwrap().asserted.is_empty());
    });
}
