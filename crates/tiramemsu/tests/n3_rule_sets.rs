//! Spec `n3-rule-sets` (OpenSpec change `add-n3-reasoning`): rule sets stored
//! as statements, superseded on edit, read as of a past transaction, and
//! classified for materialization.
#![cfg(feature = "n3")]

mod n3_common;

use n3_common::*;
use tiramemsu::*;

const IMMORTALITY: &str = "{ ?x a v:Human } => { ?x a v:Immortal } .";

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

// @tg: verifies:: [[openspec:n3-rule-sets#Store a rule set#Store and reason]] {since: "2026-10-28", level: integration}
#[test]
fn store_and_reason() {
    each(|name, db| {
        db.transact(TxOptions::default(), |tx| {
            tx.assert(v("socrates"), a(), v("Human"), Valid::ALWAYS)?;
            Ok(())
        })
        .unwrap();
        let (r_eid, _) = store(db, "mortality", MORTALITY);
        let stored = db
            .now()
            .reason(Rules::Stored(v("mortality")), &ReasonOptions::default())
            .unwrap();
        let direct = db
            .now()
            .reason(text(MORTALITY), &ReasonOptions::default())
            .unwrap();
        let contents = |r: &ReasonResult| r.conclusions.iter().map(content).collect::<Vec<_>>();
        assert_eq!(contents(&stored), contents(&direct), "{name}");
        assert_eq!(stored.conclusions.len(), 1);
        assert_eq!(stored.rule_set, Some(v("mortality")));
        assert_eq!(stored.rule_set_statement, Some(r_eid));
        assert_eq!(stored.conclusions[0].proof.rule_set, Some(v("mortality")));
        // the rule-set statement is `(rs n3:source "<text>")`
        let src = db
            .now()
            .encode(&Value::iri(n3_engine::vocab::N3_SOURCE))
            .unwrap()
            .unwrap();
        let rows = db.now().triples(None, Some(src), None).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].eid, r_eid);
        assert_eq!(db.now().decode(rows[0].o).unwrap(), Value::str(MORTALITY));
    });
}

// @tg: verifies:: [[openspec:n3-rule-sets#Store a rule set#Re-storing the same text]] {since: "2026-10-28", level: integration}
#[test]
fn re_storing_the_same_text() {
    each(|name, db| {
        let (first, _) = store(db, "mortality", MORTALITY);
        let (second, report) = store(db, "mortality", MORTALITY);
        assert_eq!(first, second, "{name}");
        assert!(
            report.asserted.is_empty()
                && report.superseded.is_empty()
                && report.retracted.is_empty()
        );
        // other text supersedes the live statement
        let (third, report) = store(db, "mortality", IMMORTALITY);
        assert_ne!(third, first);
        assert_eq!(report.superseded, [(first, third)]);
    });
}

// @tg: verifies:: [[openspec:n3-rule-sets#Store a rule set#Unknown rule set]] {since: "2026-10-28", level: integration}
#[test]
fn unknown_rule_set() {
    each(|name, db| {
        match db
            .now()
            .reason(Rules::Stored(v("nothing")), &ReasonOptions::default())
        {
            Err(Error::UnknownRuleSet { rule_set }) => {
                assert!(rule_set.contains("nothing"), "{name}")
            }
            other => panic!("{name}: {other:?}"),
        }
        assert!(matches!(
            db.now().rule_set_info(&v("nothing")),
            Err(Error::UnknownRuleSet { .. })
        ));
    });
}

#[test]
fn text_that_does_not_parse_leaves_no_trace() {
    each(|name, db| {
        let r = db.transact(TxOptions::default(), |tx| {
            tx.store_rules(v("broken"), "{ ?x a v:Human } => ")?;
            Ok(())
        });
        assert!(
            matches!(r, Err(Error::N3Parse { line: 1, .. })),
            "{name}: {r:?}"
        );
        assert_eq!(db.last_t().unwrap(), 0);
        assert!(db.now().triples(None, None, None).unwrap().is_empty());
    });
}

// @tg: verifies:: [[openspec:n3-rule-sets#Rule sets have transaction time#Rules as they were]] {since: "2026-10-28", level: integration}
#[test]
fn rules_as_they_were() {
    each(|name, db| {
        let t1 = db
            .transact(TxOptions::default(), |tx| {
                tx.assert(v("socrates"), a(), v("Human"), Valid::ALWAYS)?;
                Ok(())
            })
            .unwrap();
        let (_, t2) = store(db, "rs", MORTALITY);
        for i in 3..5 {
            db.transact(TxOptions::default(), |tx| {
                tx.assert(v(&format!("x{i}")), v("p"), Value::Int(i), Valid::ALWAYS)?;
                Ok(())
            })
            .unwrap();
        }
        let (_, t5) = store(db, "rs", IMMORTALITY);
        assert_eq!((t1.t, t2.t, t5.t), (TxId(1), TxId(2), TxId(5)), "{name}");
        let run = |view: View<'_>| -> Vec<String> {
            view.reason(Rules::Stored(v("rs")), &ReasonOptions::default())
                .unwrap()
                .conclusions
                .iter()
                .map(content)
                .collect()
        };
        assert_eq!(
            run(db.as_of(TimeRef::Tx(3))),
            [spo(&v("socrates"), &a(), &v("Mortal"))]
        );
        assert_eq!(run(db.now()), [spo(&v("socrates"), &a(), &v("Immortal"))]);
        // before it was stored the rule set is unknown
        assert!(matches!(
            db.as_of(TimeRef::Tx(1))
                .reason(Rules::Stored(v("rs")), &ReasonOptions::default()),
            Err(Error::UnknownRuleSet { .. })
        ));
        // the history keeps every version
        let src = db
            .now()
            .encode(&Value::iri(n3_engine::vocab::N3_SOURCE))
            .unwrap()
            .unwrap();
        assert_eq!(
            db.history().triples(None, Some(src), None).unwrap().len(),
            2
        );
    });
}

// @tg: verifies:: [[openspec:n3-rule-sets#Monotonicity check#Negation is detected]] {since: "2026-10-28", level: integration}
#[test]
fn negation_is_detected() {
    each(|name, db| {
        store(
            db,
            "unmanaged",
            "{ ?x a v:Employee. ?SCOPE log:notIncludes { ?x v:manager ?m } } => { ?x a v:Unmanaged } .",
        );
        let info = db.now().rule_set_info(&v("unmanaged")).unwrap();
        assert_eq!(
            info.monotonicity,
            Monotonicity::NonMonotonic {
                rule: 0,
                builtin: "log:notIncludes".into()
            },
            "{name}"
        );
        assert_eq!(info.rules, 1);
        assert_eq!(
            info.predicates,
            [vocab::RDF_TYPE.to_string(), format!("{}manager", vocab::V)]
        );
        store(
            db,
            "clock",
            "{ ?x a v:Human } => { ?x a v:Mortal } .\n{ tm:run tm:now ?t } => { v:run v:at ?t } .",
        );
        assert_eq!(
            db.now().rule_set_info(&v("clock")).unwrap().monotonicity,
            Monotonicity::ClockDependent {
                rule: 1,
                builtin: "tm:now".into()
            }
        );
        store(db, "mortality", MORTALITY);
        assert_eq!(
            db.now()
                .rule_set_info(&v("mortality"))
                .unwrap()
                .monotonicity,
            Monotonicity::Monotonic
        );
    });
}
