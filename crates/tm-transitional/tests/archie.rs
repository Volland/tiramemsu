//! The running example of Rönnbäck's paper: Archie's beard, with exact results.
use std::collections::BTreeMap;
use std::sync::Arc;

use tiramemsu::{Db, ManualClock, OpenOptions, Value};
use tm_transitional::{Assertion, Dset, Posit, Step, TrError, Transitional};

const DAY: i64 = 86_400_000;
const FRIDAY: i64 = 5 * DAY;
const SATURDAY: i64 = 6 * DAY;
const SUNDAY: i64 = 7 * DAY;
const END_OF_SUNDAY: i64 = SUNDAY + DAY - 1;
const T1000: i64 = 600; // appearance times in minutes after midnight
const T1001: i64 = 601;
const T1002: i64 = 602;
const T1003: i64 = 603;

struct Archie {
    _dir: tempfile::TempDir,
    db: Db,
    clock: Arc<ManualClock>,
    p: [String; 4],
    names: BTreeMap<i64, String>, // seq -> a1.. name
}

fn posit(value: &str, t: i64) -> Posit {
    Posit {
        dset: Dset::new(&[("A", "has beard")]).unwrap(),
        value: Value::str(value),
        t_app: t,
    }
}

impl Archie {
    fn new() -> Archie {
        let dir = tempfile::tempdir().unwrap();
        let clock = Arc::new(ManualClock::new(FRIDAY));
        let db = Db::open(
            dir.path().join("archie.db"),
            OpenOptions {
                clock: clock.clone(),
                ..OpenOptions::default()
            },
        )
        .unwrap();
        let ps = [
            posit("fluffy red", T1000),   // p1
            posit("shaved clean", T1002), // p2
            posit("shaved clean", T1000), // p3
            posit("fluffy red", T1002),   // p4
        ];
        let p = [ps[0].id(), ps[1].id(), ps[2].id(), ps[3].id()];
        Transitional::new(&db)
            .write(ps.into_iter().map(Step::Posit).collect())
            .unwrap();
        Archie {
            _dir: dir,
            db,
            clock,
            p,
            names: BTreeMap::new(),
        }
    }

    fn tr(&self) -> Transitional<'_> {
        Transitional::new(&self.db)
    }

    /// Writes one assertion at the given day and names it.
    fn assert(
        &mut self,
        name: &str,
        day: i64,
        who: &str,
        p: usize,
        alpha: f64,
    ) -> Result<(), TrError> {
        self.clock.set(day);
        let w = self.tr().write(vec![Step::Assert {
            positor: who.into(),
            posit: self.p[p - 1].clone(),
            alpha,
        }])?;
        self.names.insert(w[0].seq, name.into());
        Ok(())
    }

    fn effect(&self, t_assert: i64, t_app: i64) -> Vec<String> {
        let r = self.tr().in_effect(t_assert, t_app).unwrap();
        r.iter().map(|a| self.names[&a.seq].clone()).collect()
    }

    /// a1..a5 of the paper, in the order the paper lists them.
    fn story() -> Archie {
        let mut a = Archie::new();
        a.assert("a1", FRIDAY, "D", 2, 1.0).unwrap();
        a.assert("a2", FRIDAY, "C", 1, 0.75).unwrap();
        a.assert("a3", FRIDAY, "C", 3, -1.0).unwrap();
        a.assert("a4", SUNDAY, "D", 2, 0.0).unwrap();
        a.assert("a5", SUNDAY, "D", 4, 0.25).unwrap();
        a
    }
}

// @tg: verifies:: [[tests#Transitional Modeling#Archie Information In Effect]] {since: "2026-11-22", level: unit}
#[test]
fn archie_information_in_effect() {
    let a = Archie::story();
    assert_eq!(a.effect(SATURDAY + DAY / 2, T1001), ["a2", "a3"]);
    assert_eq!(a.effect(SATURDAY + DAY / 2, T1003), ["a1", "a2", "a3"]);
    assert_eq!(a.effect(END_OF_SUNDAY, T1003), ["a2", "a3", "a5"]);
}

// @tg: verifies:: [[tests#Transitional Modeling#Boundary Rule]] {since: "2026-11-22", level: unit}
#[test]
fn donna_may_hold_a_positive_and_a_negative_opinion_but_not_two_that_exceed_one() {
    // a6 = !(D, p2, -0.75, Sunday): 0.25 + (1 - 0.75) = 0.5, accepted
    let mut a = Archie::story();
    a.assert("a6", SUNDAY, "D", 2, -0.75).unwrap();
    assert_eq!(a.effect(END_OF_SUNDAY, T1003), ["a2", "a3", "a5", "a6"]);

    // 0.25 on p4 and 0.9 on p2 sum to 1.15: rejected, and nothing is written
    let mut b = Archie::story();
    let before = b.tr().assertions().unwrap().len();
    let err = b.assert("x", SUNDAY, "D", 2, 0.9).unwrap_err();
    assert!(matches!(err, TrError::Rejected(_)), "{err}");
    assert_eq!(b.tr().assertions().unwrap().len(), before);
}

// @tg: verifies:: [[tests#Transitional Modeling#Reassertion And Restatement]] {since: "2026-11-22", level: unit}
#[test]
fn reassertion_and_restatement_are_stored_and_the_latest_is_in_effect() {
    // Reassertion: Charlie says a2 again on Sunday, same posit and same alpha.
    let mut a = Archie::story();
    a.assert("a2again", SUNDAY, "C", 1, 0.75).unwrap();
    let all = a.tr().assertions().unwrap();
    assert_eq!(all.len(), 6, "both a2 and its reassertion are stored");
    assert_eq!(a.effect(END_OF_SUNDAY, T1003), ["a3", "a5", "a2again"]);
    // a2 itself is still what the store said on Saturday
    assert_eq!(a.effect(SATURDAY + DAY / 2, T1001), ["a2", "a3"]);

    // Restatement: Charlie gives the same alpha to the same value at a later
    // appearance time (p4, fluffy red at 10:02), on Friday's clock.
    let mut b = Archie::story();
    b.assert("a7", FRIDAY + 1, "C", 4, 0.75).unwrap();
    assert_eq!(b.tr().assertions().unwrap().len(), 6);
    // at 10:01 the restatement has not appeared yet, so nothing changes
    assert_eq!(b.effect(END_OF_SUNDAY, T1001), ["a2", "a3"]);
    // at 10:03 the latest appearance time per positor and dset wins: Charlie
    // is now only the restatement
    assert_eq!(b.effect(END_OF_SUNDAY, T1003), ["a5", "a7"]);
}

// @tg: verifies:: [[tests#Transitional Modeling#Correction Is Atomic]] {since: "2026-11-22", level: unit}
#[test]
fn a_correction_retracts_and_restates_in_one_transaction() {
    let a = Archie::story();
    a.clock.set(SUNDAY + 1);
    let before = a.tr().assertions().unwrap().len();
    // Charlie corrects "fluffy red at 10:00" to "grey at 10:00"
    let w = a
        .tr()
        .correct("C", &a.p[0], posit("grey", T1000), 0.75)
        .unwrap();
    assert_eq!(w.len(), 2);
    assert_eq!(w[0].t_assert, w[1].t_assert, "one transaction, one instant");
    assert_eq!(a.tr().assertions().unwrap().len(), before + 2);
    let now: Vec<&Assertion> = w.iter().collect();
    let in_effect = a.tr().in_effect(END_OF_SUNDAY, T1000).unwrap();
    assert!(
        in_effect.iter().any(|x| x.seq == now[1].seq),
        "grey is in effect"
    );
    assert!(
        !in_effect.iter().any(|x| x.posit == a.p[0]),
        "fluffy red is not"
    );
}
