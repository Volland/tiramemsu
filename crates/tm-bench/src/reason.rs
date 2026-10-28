//! The N3 reasoning scenario (feature `n3`): the cost of `View::reason`
//! (load, pruning and the valid-time sweep) against store size and the number
//! of distinct validity bounds, on SQLite and redb.
//!
//! The store holds `size` statements: 5 % are `v:follows` edges between
//! `size / 40` people, the rest are `v:name` literals the join rule never reads,
//! so pruning skips them. With `bounds = 1` every edge is always valid (one
//! segment); otherwise each edge is valid between two of `bounds` instants a day
//! apart, so the sweep cuts the timeline into about `bounds + 1` segments.
//!
//! Three runs per point: `join` (a two-hop join, pruned, pointwise), `join-atemporal`
//! (the same rule in one run, which isolates the sweep) and `load-all` (a rule with
//! a variable predicate, which turns pruning off and loads every statement, and
//! concludes nothing). Before timing, both engines must agree on the loaded
//! count, the segments and the conclusions.

use std::path::Path;
use std::time::Instant;

use tiramemsu::{
    Backend, Db, OpenOptions, ReasonOptions, ReasonResult, Rules, TimeMode, TxOptions, Valid, Value,
};

use crate::gen::Lcg;
use crate::suite::{stats, Entry};

const V: &str = "urn:tiramemsu:v:";
const DAY: i64 = 86_400_000;
/// 2020-01-01T00:00:00Z.
const EPOCH: i64 = 1_577_836_800_000;

/// The two-hop join the scenario times.
pub const JOIN: &str = "{ ?a v:follows ?b . ?b v:follows ?c } => { ?a v:fof ?c } .";
/// A rule with a variable predicate: loads the whole store, concludes nothing.
pub const LOAD_ALL: &str = "{ ?s ?p ?o . ?o v:never ?x } => { ?s v:hit ?x } .";

/// What to run.
#[derive(Clone, Debug)]
pub struct ReasonConfig {
    /// Store sizes in statements.
    pub sizes: Vec<u64>,
    /// Distinct validity bounds of the edges (1: always valid).
    pub bounds: Vec<u64>,
    /// Timed repetitions per run (after one checked run).
    pub reps: usize,
    /// Generator seed.
    pub seed: u64,
}

impl ReasonConfig {
    /// A short run: 10 000 statements, 1 and 16 bounds, 3 repetitions.
    pub fn quick() -> ReasonConfig {
        ReasonConfig {
            sizes: vec![10_000],
            bounds: vec![1, 16],
            reps: 3,
            seed: 7,
        }
    }

    /// The full grid: 10 000 and 100 000 statements, 1, 16 and 256 bounds.
    pub fn full() -> ReasonConfig {
        ReasonConfig {
            sizes: vec![10_000, 100_000],
            bounds: vec![1, 16, 256],
            reps: 3,
            seed: 7,
        }
    }
}

fn v(s: impl std::fmt::Display) -> Value {
    Value::iri(format!("{V}{s}"))
}

/// Loads the scenario's statements into a new database at `path`.
pub fn load(
    path: &Path,
    backend: Backend,
    size: u64,
    bounds: u64,
    seed: u64,
) -> Result<Db, String> {
    let e = |e: tiramemsu::Error| e.to_string();
    let db = Db::create(path, backend, OpenOptions::default()).map_err(e)?;
    let edges = (size / 20).max(2);
    let people = (size / 40).max(2);
    let mut rng = Lcg(seed);
    let mut rows = Vec::with_capacity(size as usize);
    for _ in 0..edges {
        let (a, b) = (rng.below(people), rng.below(people));
        let valid = if bounds <= 1 {
            Valid::ALWAYS
        } else {
            let i = rng.below(bounds - 1);
            let j = i + 1 + rng.below(bounds - 1 - i);
            Valid {
                from: Some(EPOCH + i as i64 * DAY),
                to: Some(EPOCH + j as i64 * DAY),
            }
        };
        rows.push((v(format!("p{a}")), v("follows"), v(format!("p{b}")), valid));
    }
    for k in 0..size.saturating_sub(edges) {
        let who = v(format!("p{}", k % people));
        rows.push((
            who,
            v("name"),
            Value::str(format!("name {k}")),
            Valid::ALWAYS,
        ));
    }
    for chunk in rows.chunks(50_000) {
        db.transact(TxOptions::default(), |tx| {
            for (s, p, o, valid) in chunk {
                tx.assert(s.clone(), p.clone(), o.clone(), *valid)?;
            }
            Ok(())
        })
        .map_err(e)?;
    }
    Ok(db)
}

fn run_once(db: &Db, rules: &str, time: TimeMode) -> Result<ReasonResult, String> {
    let opts = ReasonOptions {
        time,
        max_segments: usize::MAX,
        ..ReasonOptions::default()
    };
    db.now()
        .reason(Rules::Text(rules.into()), &opts)
        .map_err(|e| e.to_string())
}

/// Runs the scenario in `dir` and returns one entry per (size, bounds, run, engine):
/// workload `reason`, `rows` the conclusions, `value` the loaded statements and
/// `note` the segments.
pub fn run(cfg: &ReasonConfig, dir: &Path) -> Result<Vec<Entry>, String> {
    let mut out = Vec::new();
    let runs = [
        ("join", JOIN, TimeMode::Pointwise),
        ("join-atemporal", JOIN, TimeMode::Atemporal),
        ("load-all", LOAD_ALL, TimeMode::Atemporal),
    ];
    for &size in &cfg.sizes {
        for &bounds in &cfg.bounds {
            let engines = [("sqlite", Backend::Sqlite), ("redb", Backend::Redb)]
                .into_iter()
                .map(|(name, b)| {
                    let path = dir.join(format!("reason-{size}-{bounds}-{name}.db"));
                    load(&path, b, size, bounds, cfg.seed).map(|db| (name, db))
                })
                .collect::<Result<Vec<_>, _>>()?;
            for (query, rules, time) in runs {
                // checked run: every engine gives the same answer before anything is timed
                let mut seen: Option<(usize, usize, usize)> = None;
                for (name, db) in &engines {
                    let r = run_once(db, rules, time)?;
                    let key = (r.loaded, r.segments, r.conclusions.len());
                    match seen {
                        Some(k) if k != key => {
                            return Err(format!(
                                "reason `{query}` at {size}/{bounds}: {name} gives (loaded, segments, conclusions) {key:?}, sqlite {k:?}"
                            ))
                        }
                        _ => seen = Some(key),
                    }
                }
                let (loaded, segments, conclusions) = seen.unwrap_or_default();
                for (name, db) in &engines {
                    let mut samples = Vec::with_capacity(cfg.reps);
                    for _ in 0..cfg.reps.max(1) {
                        let t = Instant::now();
                        run_once(db, rules, time)?;
                        samples.push(t.elapsed().as_secs_f64() * 1e6);
                    }
                    let (median, p95) = stats(&mut samples);
                    out.push(Entry {
                        workload: "reason".into(),
                        query: query.into(),
                        engine: (*name).into(),
                        variant: format!("{size} statements, {bounds} bounds"),
                        rows: Some(conclusions as u64),
                        median_us: Some(median),
                        p95_us: Some(p95),
                        value: Some(loaded as f64),
                        unit: Some("statements loaded".into()),
                        note: Some(format!("{segments} segments")),
                    });
                }
            }
        }
    }
    Ok(out)
}

/// The entries of [`run`] as a Markdown table.
pub fn markdown(entries: &[Entry]) -> String {
    let mut s = String::from(
        "| store | run | engine | loaded | segments | conclusions | median ms | p95 ms |\n\
         |---|---|---|---:|---|---:|---:|---:|\n",
    );
    for e in entries {
        s.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {:.1} | {:.1} |\n",
            e.variant,
            e.query,
            e.engine,
            e.value.unwrap_or(0.0),
            e.note.as_deref().unwrap_or(""),
            e.rows.unwrap_or(0),
            e.median_us.unwrap_or(0.0) / 1000.0,
            e.p95_us.unwrap_or(0.0) / 1000.0,
        ));
    }
    s
}
