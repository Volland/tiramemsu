//! Property tests of ELPI enforcement (`lat.md/elpi#Two Constraint Entry
//! Points`): on random histories, an enforced program's delta check refuses a
//! transaction exactly when the full check after it finds a violation that the
//! full check before it did not.
#![cfg(feature = "elpi")]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use proptest::prelude::*;
use tiramemsu::*;

fn v(s: &str) -> Value {
    Value::iri(format!("urn:tiramemsu:v:{s}"))
}

/// Two constraints, each with a delta form that covers every way a write can
/// introduce it: a second CEO (seeded by additions) and a project without an
/// owner (seeded by additions of projects and retractions of owners).
const POLICY: &str = r#"
pred owned o:term.
owned P :- tm.triple P (iri "urn:tiramemsu:v:owner") _ _.
tm.violation error "one_ceo" "two CEOs" [E1, E2] :-
  tm.triple X (iri "urn:tiramemsu:v:role") (iri "urn:tiramemsu:v:CEO") E1,
  tm.triple Y (iri "urn:tiramemsu:v:role") (iri "urn:tiramemsu:v:CEO") E2,
  not (X = Y), E1 = eid N1, E2 = eid N2, N1 < N2.
tm.violation error "owned" "project without owner" [E] :-
  tm.triple P (iri "urn:tiramemsu:v:type") (iri "urn:tiramemsu:v:Project") E,
  not (owned P).
tm.violation-delta "one_ceo" "two CEOs" [E1, E2] :-
  tm.added X (iri "urn:tiramemsu:v:role") (iri "urn:tiramemsu:v:CEO") E1,
  tm.triple Y (iri "urn:tiramemsu:v:role") (iri "urn:tiramemsu:v:CEO") E2,
  not (X = Y).
tm.violation-delta "owned" "project without owner" [E] :-
  tm.added P (iri "urn:tiramemsu:v:type") (iri "urn:tiramemsu:v:Project") E,
  not (owned P).
tm.violation-delta "owned" "project without owner" [E] :-
  tm.retracted P (iri "urn:tiramemsu:v:owner") _ _,
  tm.triple P (iri "urn:tiramemsu:v:type") (iri "urn:tiramemsu:v:Project") E,
  not (owned P).
"#;

#[derive(Clone, Debug)]
enum Op {
    Ceo(u8),
    Project(u8),
    Owner(u8),
    /// Retract the n-th live statement of the write predicates, if any.
    Retract(u8),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        (0u8..3).prop_map(Op::Ceo),
        (0u8..3).prop_map(Op::Project),
        (0u8..3).prop_map(Op::Owner),
        (0u8..8).prop_map(Op::Retract),
    ]
}

/// Applies `ops`; a retraction picks the n-th of `live`, the statements of
/// the write predicates by content, and retracts its eid in this database.
fn apply(tx: &mut Tx<'_>, ops: &[Op], live: &[(String, Eid)]) -> Result<()> {
    for o in ops {
        match o {
            Op::Ceo(i) => {
                tx.assert(v(&format!("p{i}")), v("role"), v("CEO"), Valid::ALWAYS)?;
            }
            Op::Project(i) => {
                tx.assert(v(&format!("x{i}")), v("type"), v("Project"), Valid::ALWAYS)?;
            }
            Op::Owner(i) => {
                tx.assert(v(&format!("x{i}")), v("owner"), v("ann"), Valid::ALWAYS)?;
            }
            Op::Retract(n) => {
                if !live.is_empty() {
                    tx.retract(live[*n as usize % live.len()].1)?;
                }
            }
        }
    }
    Ok(())
}

fn open(dir: &tempfile::TempDir, name: &str) -> Db {
    Db::open(
        dir.path().join(name),
        OpenOptions {
            clock: Arc::new(ManualClock::new(1_000)),
            ..OpenOptions::default()
        },
    )
    .unwrap()
}

/// Violations by name and the content of the statements they cite, so two
/// databases with the same history compare equal whatever their eids.
type Found = BTreeSet<(String, Vec<String>)>;

fn content(view: &View<'_>) -> BTreeMap<Eid, String> {
    view.triples(None, None, None)
        .unwrap()
        .into_iter()
        .map(|t| {
            let d = |id| view.decode(id).unwrap().to_string();
            (t.eid, format!("{} {} {}", d(t.s), d(t.p), d(t.o)))
        })
        .collect()
}

fn found(view: &View<'_>, r: &ElpiCheckResult) -> Found {
    let c = content(view);
    r.violations
        .iter()
        .filter(|x| x.severity == ElpiSeverity::Error)
        .map(|x| {
            let mut cited: Vec<String> = x.eids.iter().map(|e| c[e].clone()).collect();
            cited.sort();
            (x.name.clone(), cited)
        })
        .collect()
}

/// The live statements of the write predicates, by content.
fn live(db: &Db) -> Vec<(String, Eid)> {
    let mut out: Vec<(String, Eid)> = content(&db.now())
        .into_iter()
        .filter(|(_, c)| {
            ["role", "type", "owner"]
                .iter()
                .any(|p| c.contains(&format!(":v:{p}>")))
        })
        .map(|(e, c)| (c, e))
        .collect();
    out.sort();
    out
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 24, ..ProptestConfig::default() })]

    // @tg: verifies:: [[openspec:elpi-constraints#Delta checks at commit]] {level: integration}
    #[test]
    fn delta_refuses_exactly_new_violations(txs in prop::collection::vec(prop::collection::vec(op(), 1..4), 1..8)) {
        let dir = tempfile::tempdir().unwrap();
        // `enforced` checks at commit; `shadow` is the same history unenforced,
        // where the full check is the oracle
        let enforced = open(&dir, "e.db");
        let shadow = open(&dir, "s.db");
        let program = ElpiProgram::Text(POLICY.to_string());
        let opts = ElpiOptions::default();
        enforced
            .transact(TxOptions::default(), |tx| {
                tx.store_elpi_program(v("policy"), POLICY)?;
                tx.enforce_elpi(v("policy"), true)?;
                Ok(())
            })
            .unwrap();
        shadow
            .transact(TxOptions::default(), |tx| {
                tx.store_elpi_program(v("policy"), POLICY)?;
                tx.assert(v("policy"), v("note"), Value::str("unenforced"), Valid::ALWAYS)?;
                Ok(())
            })
            .unwrap();
        for ops in &txs {
            let (live_s, live_e) = (live(&shadow), live(&enforced));
            let keys = |l: &[(String, Eid)]| l.iter().map(|(c, _)| c.clone()).collect::<Vec<_>>();
            prop_assert_eq!(keys(&live_s), keys(&live_e));
            let before = {
                let view = shadow.now();
                found(&view, &view.elpi_check(&program, &opts).unwrap())
            };
            let after = shadow
                .with(|tx| apply(tx, ops, &live_s), |view| {
                    Ok(found(view, &view.elpi_check(&program, &opts)?))
                })
                .unwrap();
            let introduces = after.difference(&before).next().is_some();
            let r = enforced.transact(TxOptions::default(), |tx| apply(tx, ops, &live_e));
            match &r {
                Err(Error::ConstraintViolated { .. }) => prop_assert!(introduces, "refused {ops:?} without a new violation"),
                Ok(_) => prop_assert!(!introduces, "committed {ops:?} with new violations {:?}", after.difference(&before).collect::<Vec<_>>()),
                Err(e) => panic!("{e}"),
            }
            if r.is_ok() {
                shadow.transact(TxOptions::default(), |tx| apply(tx, ops, &live_s)).unwrap();
            }
        }
    }
}
