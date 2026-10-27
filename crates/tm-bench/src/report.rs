//! The Markdown report of a results file: every workload with median and p95,
//! the run's inputs, and strengths and weaknesses derived from the numbers.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

use crate::suite::{Entry, Results};

/// Engines in report order.
const ORDER: [&str; 4] = ["sqlite", "sqlite-lftj", "redb", "oxigraph"];

fn engine_rank(e: &str) -> usize {
    ORDER.iter().position(|x| *x == e).unwrap_or(ORDER.len())
}

/// Whether a larger value is better for this unit.
fn higher_is_better(unit: Option<&str>) -> bool {
    unit == Some("queries/s")
}

/// The number a comparison uses: the median latency, or the value; a timeout
/// counts as the cap (so a factor against it is a lower bound).
fn score(e: &Entry, cap_us: f64) -> Option<f64> {
    if e.note.as_deref() == Some("timeout") {
        return Some(cap_us);
    }
    e.median_us.or(e.value)
}

fn fmt_us(us: f64) -> String {
    if us >= 1e6 {
        format!("{:.2} s", us / 1e6)
    } else if us >= 1e3 {
        format!("{:.2} ms", us / 1e3)
    } else {
        format!("{us:.0} µs")
    }
}

fn fmt_cell(e: &Entry, cap_s: u64) -> String {
    if e.note.as_deref() == Some("timeout") {
        return format!("timeout (> {cap_s} s)");
    }
    match (e.median_us, e.p95_us, e.value) {
        (Some(m), Some(p), _) => format!("{} (p95 {})", fmt_us(m), fmt_us(p)),
        (_, _, Some(v)) => match e.unit.as_deref() {
            Some("s") => format!("{v:.2} s"),
            Some("B/statement") => format!("{v:.0} B"),
            Some("queries/s") => format!("{v:.0} q/s"),
            Some(u) => format!("{v:.2} {u}"),
            None => format!("{v:.2}"),
        },
        _ => "-".to_string(),
    }
}

/// One comparison of `a` against `b` on the same measurement: the factor by
/// which `a` is better (above 1) or worse (below 1).
pub fn factor(a: &Entry, b: &Entry, cap_us: f64) -> Option<f64> {
    let (x, y) = (score(a, cap_us)?, score(b, cap_us)?);
    if x <= 0.0 || y <= 0.0 {
        return None;
    }
    Some(if higher_is_better(a.unit.as_deref()) {
        x / y
    } else {
        y / x
    })
}

/// Lines `(measurement, factor)` where `engine` is better (`better = true`)
/// or worse than `against` by more than 10 %.
pub fn compare(r: &Results, engine: &str, against: &str, better: bool) -> Vec<(String, f64)> {
    let mut by_key: BTreeMap<(String, String, String), BTreeMap<String, &Entry>> = BTreeMap::new();
    for e in &r.entries {
        by_key
            .entry((e.workload.clone(), e.query.clone(), e.variant.clone()))
            .or_default()
            .insert(e.engine.clone(), e);
    }
    let mut out = Vec::new();
    for ((w, q, v), engines) in &by_key {
        let (Some(a), Some(b)) = (engines.get(engine), engines.get(against)) else {
            continue;
        };
        let Some(f) = factor(a, b, r.meta.timeout_s.max(1) as f64 * 1e6) else {
            continue;
        };
        let timeout = |e: &Entry| e.note.as_deref() == Some("timeout");
        let bound = if timeout(a) || timeout(b) {
            " (at least: one side timed out)"
        } else {
            ""
        };
        if timeout(a) && timeout(b) {
            continue;
        }
        if (better && f > 1.1) || (!better && f < 1.0 / 1.1) {
            out.push((format!("{w} / {q} ({v}){bound}"), f));
        }
    }
    out.sort_by(|a, b| {
        let (x, y) = if better { (b.1, a.1) } else { (a.1, b.1) };
        x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

// @tg: implements:: [[openspec:backend-benchmarks#Honest report]] {since: "2026-10-27"}
/// The report of `r`.
pub fn markdown(r: &Results) -> String {
    let mut s = String::new();
    let m = &r.meta;
    let _ = writeln!(s, "## Run\n");
    let _ = writeln!(s, "- **Machine:** {}", m.machine);
    let _ = writeln!(s, "- **Scale:** {} statements in the WatDiv-style graph, {} timed repetitions per query after one checked run", m.statements, m.reps);
    let _ = writeln!(
        s,
        "- **Seeds:** {}",
        m.seeds
            .iter()
            .map(|(k, v)| format!("{k} {v}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let _ = writeln!(
        s,
        "- **Versions:** {}",
        m.versions
            .iter()
            .map(|(k, v)| format!("{k}: {v}"))
            .collect::<Vec<_>>()
            .join("; ")
    );
    let _ = writeln!(s, "- **Durability:** {}", m.durability);
    if m.timeout_s > 0 {
        let _ = writeln!(s, "- **Time cap:** {} s per query; an engine that passes it is shown as a timeout and left out of that query's comparison", m.timeout_s);
    }
    let _ = writeln!(s, "- **Command:** `{}`\n", m.command);

    let workloads: Vec<String> = {
        let mut seen = BTreeSet::new();
        r.entries
            .iter()
            .filter(|e| seen.insert(e.workload.clone()))
            .map(|e| e.workload.clone())
            .collect()
    };
    for w in &workloads {
        let rows: Vec<&Entry> = r.entries.iter().filter(|e| &e.workload == w).collect();
        let mut engines: Vec<String> = rows
            .iter()
            .map(|e| e.engine.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        engines.sort_by_key(|e| engine_rank(e));
        let _ = writeln!(s, "### {w}\n");
        let _ = writeln!(s, "| query | variant | rows | {} |", engines.join(" | "));
        let _ = writeln!(s, "|---|---|---|{}", "---|".repeat(engines.len()));
        let mut keys: Vec<(String, String)> = Vec::new();
        for e in &rows {
            let k = (e.query.clone(), e.variant.clone());
            if !keys.contains(&k) {
                keys.push(k);
            }
        }
        for (q, v) in keys {
            let cells: Vec<String> = engines
                .iter()
                .map(|en| {
                    rows.iter()
                        .find(|e| e.query == q && e.variant == v && &e.engine == en)
                        .map_or("-".to_string(), |e| fmt_cell(e, m.timeout_s))
                })
                .collect();
            let n = rows
                .iter()
                .find(|e| e.query == q && e.variant == v)
                .and_then(|e| e.rows)
                .map_or("-".to_string(), |n| n.to_string());
            let _ = writeln!(s, "| {q} | {v} | {n} | {} |", cells.join(" | "));
        }
        let _ = writeln!(s);
    }

    let _ = writeln!(s, "## Strengths and weaknesses (derived)\n");
    let _ = writeln!(s, "Each line compares one measurement and names the factor; differences under 10 % are left out.\n");
    let pairs = [
        ("redb", "sqlite"),
        ("redb", "oxigraph"),
        ("sqlite", "oxigraph"),
    ];
    for (a, b) in pairs {
        if !r.entries.iter().any(|e| e.engine == a) || !r.entries.iter().any(|e| e.engine == b) {
            continue;
        }
        for (better, title) in [(true, "faster or smaller"), (false, "slower or larger")] {
            let lines = compare(r, a, b, better);
            let _ = writeln!(s, "**{a} {title} than {b}** ({}):\n", lines.len());
            if lines.is_empty() {
                let _ = writeln!(s, "- none\n");
                continue;
            }
            for (k, f) in lines {
                let f = if better { f } else { 1.0 / f };
                let _ = writeln!(s, "- {k}: {f:.1}×");
            }
            let _ = writeln!(s);
        }
    }
    s
}
