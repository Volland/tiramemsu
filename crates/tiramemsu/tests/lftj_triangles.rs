//! Spec `cyclic-join-execution` "Triangle benchmark": the harness of
//! `examples/triangles.rs` verifies identical triangle counts on the SQL and the
//! LFTJ route before it times anything.
#![cfg(feature = "exec")]

#[path = "../examples/triangles.rs"]
mod harness;

use harness::*;

// cyclic-join-execution "Triangle benchmark": SQL and native timing is compared
// only after the harness verified identical counts
// @lat: [[tests#Cyclic Joins#Triangle Harness Verifies Counts]]
#[test]
fn harness_verifies_counts_before_timing() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = Lcg(7);
    let cases = vec![
        ("uniform", uniform(400)),
        ("skewed", skewed(6, 30, 300, &mut rng)),
        ("layered", layered(12, &mut rng)),
    ];
    for (name, edges) in cases {
        let row = compare(dir.path(), name, edges, 1).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(row.name, name);
        assert!(row.edges > 0);
        if name != "uniform" {
            assert!(row.triangles > 0, "{name} has triangles");
        }
    }
    // the layered graph's count is known: each distinct C→A closing edge closes one
    // triangle per B node, since the A→B and B→C layers are complete, and the
    // pattern matches each directed triangle once per rotation
    let mut rng = Lcg(1);
    let edges = layered(5, &mut rng);
    let closers = {
        let mut c: Vec<_> = edges.iter().filter(|(s, _)| *s >= 20_000_000).collect();
        c.sort();
        c.dedup();
        c.len() as i64
    };
    let row = compare(dir.path(), "layered-5", edges, 1).unwrap();
    assert_eq!(row.triangles, closers * 5 * 3);
}
