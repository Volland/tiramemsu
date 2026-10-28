//! The interval model of `add-interval-algebra` (task 1): Allen's relations and
//! `intersects` over half-open intervals with open ends.

use proptest::prelude::*;
use tm_core::{Allen, Valid};

fn interval() -> impl Strategy<Value = Valid> {
    (
        proptest::option::weighted(0.8, -6i64..6),
        proptest::option::weighted(0.8, -6i64..6),
    )
        .prop_filter_map("proper", |(f, t)| {
            let v = Valid { from: f, to: t };
            v.check().ok().map(|_| v)
        })
}

/// Each relation stated on endpoints, independently of `relate`.
fn holds(r: Allen, a: &Valid, b: &Valid) -> bool {
    let lo = |x: Option<i64>| x.map_or(i128::MIN, i128::from);
    let hi = |x: Option<i64>| x.map_or(i128::MAX, i128::from);
    let (af, at, bf, bt) = (lo(a.from), hi(a.to), lo(b.from), hi(b.to));
    match r {
        Allen::Before => at < bf,
        Allen::Meets => at == bf,
        Allen::Overlaps => af < bf && bf < at && at < bt,
        Allen::Starts => af == bf && at < bt,
        Allen::During => bf < af && at < bt,
        Allen::Finishes => bf < af && at == bt,
        Allen::Equals => af == bf && at == bt,
        other => holds(other.converse(), b, a),
    }
}

/// The assert overlap test as SQLite evaluates it on the stored columns.
fn sql_overlap(conn: &rusqlite::Connection, a: &Valid, b: &Valid) -> bool {
    conn.query_row(
        "SELECT (?1 IS NULL OR ?4 IS NULL OR ?1 < ?4) AND (?3 IS NULL OR ?2 IS NULL OR ?3 < ?2)",
        rusqlite::params![a.from, a.to, b.from, b.to],
        |r| r.get::<_, bool>(0),
    )
    .unwrap()
}

proptest! {
    #[test]
    fn exactly_one_relation_holds(a in interval(), b in interval()) {
        let true_ones: Vec<Allen> = Allen::ALL.into_iter().filter(|r| holds(*r, &a, &b)).collect();
        prop_assert_eq!(true_ones, vec![a.relate(&b)]);
    }

    #[test]
    fn converses_are_symmetric(a in interval(), b in interval()) {
        prop_assert_eq!(b.relate(&a), a.relate(&b).converse());
    }

    #[test]
    fn intersects_is_the_intersecting_relations(a in interval(), b in interval()) {
        prop_assert_eq!(a.intersects(&b), a.relate(&b).intersecting());
        prop_assert_eq!(a.overlaps(&b), a.intersects(&b));
    }
}

#[test]
fn intersects_equals_the_assert_overlap_sql() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    let ends: Vec<Option<i64>> = std::iter::once(None).chain((-3..4).map(Some)).collect();
    let mut all = Vec::new();
    for &f in &ends {
        for &t in &ends {
            let v = Valid { from: f, to: t };
            if v.check().is_ok() {
                all.push(v);
            }
        }
    }
    for a in &all {
        for b in &all {
            assert_eq!(a.intersects(b), sql_overlap(&conn, a, b), "{a:?} {b:?}");
        }
    }
}

#[test]
fn names_round_trip() {
    for r in Allen::ALL {
        assert_eq!(Allen::from_name(r.name()), Some(r));
        assert_eq!(r.converse().converse(), r);
    }
    assert_eq!(Allen::ALL.iter().filter(|r| r.intersecting()).count(), 9);
}
