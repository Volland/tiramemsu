//! The N3 reasoning scenario at a tiny scale: both engines agree before timing,
//! pruning loads only the edges, and more bounds mean more segments.

use tm_bench::reason::{run, ReasonConfig};

#[test]
fn reasoning_scenario_runs_on_both_backends() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = ReasonConfig {
        sizes: vec![2_000],
        bounds: vec![1, 8],
        reps: 1,
        seed: 7,
    };
    let entries = run(&cfg, dir.path()).unwrap_or_else(|e| panic!("{e}"));
    // 2 bounds settings × 3 runs × 2 engines
    assert_eq!(entries.len(), 12);
    let find = |q: &str, variant: &str, engine: &str| {
        entries
            .iter()
            .find(|e| e.query == q && e.variant.contains(variant) && e.engine == engine)
            .unwrap()
    };
    for engine in ["sqlite", "redb"] {
        let join = find("join", "1 bounds", engine);
        // pruning: only the follows edges are loaded (100 drawn, repeats are one statement)
        let edges = join.value.unwrap();
        assert!((50.0..=100.0).contains(&edges), "{edges}");
        assert_eq!(join.note.as_deref(), Some("1 segments"));
        assert!(join.rows.unwrap() > 0);
        // no pruning: the edges and the 1 900 names
        assert_eq!(
            find("load-all", "1 bounds", engine).value,
            Some(edges + 1_900.0)
        );
        let swept = find("join", "8 bounds", engine);
        let segments: usize = swept
            .note
            .as_deref()
            .unwrap()
            .split(' ')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert!(segments > 1, "{segments}");
    }
}
