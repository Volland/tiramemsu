//! Spec `sql-execution` "Join order from statistics" (decision D19).

mod common;

use common::skewed::*;
use common::*;
use tiramemsu::ir::builder::IrBuilder;
use tiramemsu::ir::Op;

fn b() -> IrBuilder {
    IrBuilder::sparql()
}

fn permutations<T: Clone>(items: &[T]) -> Vec<Vec<T>> {
    if items.len() <= 1 {
        return vec![items.to_vec()];
    }
    let mut out = Vec::new();
    for i in 0..items.len() {
        let mut rest = items.to_vec();
        let x = rest.remove(i);
        for mut p in permutations(&rest) {
            p.insert(0, x.clone());
            out.push(p);
        }
    }
    out
}

/// The alias (`tN`) of the first triple scan of the plan.
fn first_scan(plan: &[String]) -> String {
    plan.iter()
        .find_map(|l| {
            let mut w = l.split_whitespace();
            match (w.next(), w.next()) {
                (Some("SCAN" | "SEARCH"), Some(a)) if a.starts_with('t') => Some(a.to_string()),
                _ => None,
            }
        })
        .unwrap_or_else(|| panic!("no triple scan in {plan:?}"))
}

/// For every permutation of `patterns`, the plan starts from the pattern at
/// `selective` (an index into the unpermuted list) and results are equal.
fn check(t: &TestDb, patterns: &[(&str, &str, &str)], selective: usize, project: &[&str]) {
    let mut baseline: Option<Vec<Vec<String>>> = None;
    for perm in permutations(&(0..patterns.len()).collect::<Vec<_>>()) {
        let pats: Vec<(&str, &str, &str)> = perm.iter().map(|i| patterns[*i]).collect();
        let q = b().query(b().bgp(&pats).project(project));
        let ex = explain(&t.db.now(), &q);
        let want = perm.iter().position(|i| *i == selective).unwrap();
        let plan = &ex.sql_region().expect("sql region").query_plan;
        assert_eq!(
            first_scan(plan),
            format!("t{want}"),
            "permutation {perm:?} starts elsewhere:\n{}\n{}",
            ex.sql.as_deref().unwrap(),
            plan.join("\n")
        );
        let got = sorted(&run(&t.db.now(), &q));
        match &baseline {
            None => baseline = Some(got),
            Some(b0) => assert_eq!(&got, b0, "permutation {perm:?} changed the result"),
        }
    }
    assert!(!baseline.unwrap().is_empty(), "the golden BGP has results");
}

// @lat: [[tests#Query#Skewed Joins Start Selective]]
#[test]
fn skewed_joins_start_selective() {
    let t = skewed();
    assert!(
        stat_rows(&t) > 0,
        "automatic statistics exist without optimize()"
    );
    // Selective pattern first: four-pattern star on the 90 % class and the 50-row predicate
    check(
        &t,
        &[
            ("?x", "v:type", "v:Person"),
            ("?x", "v:knows", "?y"),
            ("?x", "v:rare", "v:Special"),
            ("?x", "v:works", "?o"),
        ],
        2,
        &["x", "y", "o"],
    );
    // four-pattern chain from the rare predicate
    check(
        &t,
        &[
            ("?a", "v:rare", "?b"),
            ("?a", "v:knows", "?c"),
            ("?c", "v:type", "v:Person"),
            ("?c", "v:works", "?d"),
        ],
        0,
        &["a", "c", "d"],
    );
    // predicate-only patterns
    check(
        &t,
        &[
            ("?x", "v:rare", "?y"),
            ("?x", "v:knows", "?z"),
            ("?z", "v:works", "?w"),
        ],
        0,
        &["x", "z", "w"],
    );
    let _ = Op::unit();
}
