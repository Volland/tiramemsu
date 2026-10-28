//! Property tests of Datalog (OpenSpec change `add-datalog-engine`, tasks 8.3
//! and 8.4): demand rewriting, provenance, parallelism and the access choice
//! never change the rows, and Horn programs agree with N3.
#![cfg(feature = "datalog")]

#[cfg(feature = "n3")]
use std::collections::BTreeMap;
use std::sync::Arc;

use proptest::prelude::*;
#[cfg(feature = "n3")]
use tiramemsu::datalog_engine::interval::IvSet;
use tiramemsu::*;

fn v(s: &str) -> Value {
    Value::iri(format!("urn:tiramemsu:v:{s}"))
}

fn db(dir: &tempfile::TempDir, name: &str) -> Db {
    Db::open(
        dir.path().join(name),
        OpenOptions {
            clock: Arc::new(ManualClock::new(1_000)),
            ..OpenOptions::default()
        },
    )
    .unwrap()
}

/// `(subject, predicate, object, from, length)` facts over a few nodes.
fn facts() -> impl Strategy<Value = Vec<(u8, u8, u8, i64, i64)>> {
    proptest::collection::vec((0u8..6, 0u8..3, 0u8..6, 0i64..20, 1i64..8), 1..18)
}

fn load(db: &Db, facts: &[(u8, u8, u8, i64, i64)]) {
    db.transact(TxOptions::default(), |t| {
        for &(s, p, o, f, len) in facts {
            let p = ["e", "f", "g"][p as usize];
            t.assert(
                v(&format!("n{s}")),
                v(p),
                v(&format!("n{o}")),
                Valid::between(f, f + len),
            )?;
        }
        Ok(())
    })
    .unwrap();
}

const PROGRAMS: [&str; 3] = [
    "r(X, Y) :- v:e(X, Y).\nr(X, Z) :- r(X, Y), v:e(Y, Z).\n",
    "r(X, Y) :- v:e(X, Y).\nr(X, Z) :- v:f(X, Y), r(Y, Z).\nr(X, Y) :- v:g(Y, X).\n",
    "s(X, Y) :- v:e(X, Y), v:f(Y, Z).\nr(X, Y) :- s(X, Y).\nr(X, Z) :- r(X, Y), s(Y, Z), X != Z.\n",
];

fn rows(r: &DatalogResult) -> Vec<(Vec<DatalogTerm>, Vec<Valid>)> {
    r.rows
        .iter()
        .map(|x| (x.values.clone(), x.valid.clone()))
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 24, ..ProptestConfig::default() })]

    // demand rewriting never changes the rows (random programs and goals)
    #[test]
    fn demand_equals_full(f in facts(), prog in 0usize..3, seed in 0u8..6) {
        let dir = tempfile::tempdir().unwrap();
        let db = db(&dir, "m.db");
        load(&db, &f);
        let g = format!("r(v:n{seed}, Y)");
        let text = DatalogProgram::Text(PROGRAMS[prog].into());
        let on = db.now().datalog(&text, Some(&g), &DatalogOptions::default()).unwrap();
        let off = db.now().datalog(&text, Some(&g), &DatalogOptions { demand: false, ..Default::default() }).unwrap();
        prop_assert_eq!(rows(&on), rows(&off));
    }

    // proofs, morsels and the access choice never change the rows
    #[test]
    fn tracked_parallel_and_access_do_not_change_rows(f in facts(), prog in 0usize..3) {
        let dir = tempfile::tempdir().unwrap();
        let db = db(&dir, "m.db");
        load(&db, &f);
        let text = DatalogProgram::Text(PROGRAMS[prog].into());
        let base = DatalogOptions { demand: false, ..Default::default() };
        let plain = db.now().datalog(&text, Some("r(X, Y)"), &base).unwrap();
        for o in [
            DatalogOptions { proof: true, ..base.clone() },
            DatalogOptions { parallel_threshold: 1, ..base.clone() },
            DatalogOptions { access: DatalogAccess::Probe, ..base.clone() },
            DatalogOptions { access: DatalogAccess::Load, ..base.clone() },
        ] {
            let r = db.now().datalog(&text, Some("r(X, Y)"), &o).unwrap();
            prop_assert_eq!(rows(&r), rows(&plain));
        }
    }
}

/// Per `(s, p, o)`, the instants a conclusion holds.
#[cfg(feature = "n3")]
fn coverage(
    items: impl IntoIterator<Item = (String, String, String, Valid)>,
) -> BTreeMap<(String, String, String), IvSet> {
    let mut out: BTreeMap<(String, String, String), IvSet> = BTreeMap::new();
    for (s, p, o, valid) in items {
        let e = out.entry((s, p, o)).or_default();
        *e = e.union(&IvSet::of_valid(valid));
    }
    out
}

#[cfg(feature = "n3")]
proptest! {
    #![proptest_config(ProptestConfig { cases: 24, ..ProptestConfig::default() })]

    // @tg: verifies:: [[openspec:datalog-evaluation#Agreement with N3 on Horn programs#Horn differential]] {level: integration}
    #[test]
    fn horn_programs_agree_with_n3(f in facts()) {
        let dir = tempfile::tempdir().unwrap();
        let db = db(&dir, "m.db");
        load(&db, &f);
        let dl = DatalogProgram::Text(
            "v:w(X, Z) :- v:e(X, Y), v:f(Y, Z).\nv:t(X, Y) :- v:g(X, Y).\nv:t(X, Z) :- v:t(X, Y), v:g(Y, Z).\n".into(),
        );
        let n3 = Rules::Text(
            "{ ?x v:e ?y . ?y v:f ?z } => { ?x v:w ?z } .\n\
             { ?x v:g ?y } => { ?x v:t ?y } .\n\
             { ?x v:t ?y . ?y v:g ?z } => { ?x v:t ?z } ."
                .into(),
        );
        let d = db.now().datalog_reason(&dl, &DatalogOptions::default()).unwrap();
        let r = db.now().reason(n3, &ReasonOptions::default()).unwrap();
        let dc = coverage(d.conclusions.iter().map(|c| (c.s.to_string(), c.p.to_string(), c.o.to_string(), c.valid)));
        let nc = coverage(r.conclusions.iter().filter_map(|c| {
            let (s, p, o) = c.values()?;
            Some((s.to_string(), p.to_string(), o.to_string(), c.valid))
        }));
        prop_assert_eq!(dc, nc);
    }
}
