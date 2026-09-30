//! The corpus format and the runner of the differential suite.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;
use tiramemsu::*;

use super::fixtures;
use super::normalise::{apply_mode, diff, from_cypher, from_term, Canon, Mode};

/// One query pair.
#[derive(Debug, Clone, Deserialize)]
pub struct Pair {
    pub name: String,
    pub category: String,
    pub fixture: String,
    pub sparql: String,
    pub cypher: String,
    #[serde(default = "d_bag")]
    pub mode: String,
    #[serde(default = "d_unordered")]
    pub order: String,
    /// SPARQL variable to Cypher column; both sides are projected to these, sorted by variable.
    pub columns: BTreeMap<String, String>,
    /// `now` (default), `asof:N`, `asof-instant:MS`, `history`, `valid:MS`.
    #[serde(default)]
    pub view: Option<String>,
    /// Columns whose Cypher Integers are transaction numbers.
    #[serde(default)]
    pub tx_columns: Vec<String>,
    /// Divergence pairs pin the exact row counts instead of equality.
    #[serde(default)]
    pub expect_sparql_rows: Option<usize>,
    #[serde(default)]
    pub expect_cypher_rows: Option<usize>,
}

fn d_bag() -> String {
    "bag".into()
}

fn d_unordered() -> String {
    "unordered".into()
}

#[derive(Deserialize)]
struct File {
    pair: Vec<Pair>,
}

/// Every category the corpus must cover.
pub const CATEGORIES: [&str; 22] = [
    "single-hop",
    "multi-hop",
    "labels",
    "literal-filter",
    "comparison",
    "optional",
    "exists",
    "not-exists",
    "union",
    "aggregation",
    "order-limit",
    "subquery",
    "as-of-tx",
    "as-of-instant",
    "valid-at",
    "history",
    "per-pattern",
    "time-props",
    "dual-view",
    "iso-divergence",
    "parallel-divergence",
    "multi-valued",
];

/// All pairs of the corpus files.
pub fn corpus() -> Vec<Pair> {
    let mut out = Vec::new();
    for text in [
        include_str!("corpus/basic.toml"),
        include_str!("corpus/temporal.toml"),
        include_str!("corpus/dual.toml"),
        include_str!("corpus/divergence.toml"),
    ] {
        let f: File = toml::from_str(text).expect("corpus file");
        out.extend(f.pair);
    }
    out
}

/// The completeness check: at least 40 pairs and every category. Returns the missing names.
pub fn completeness(pairs: &[Pair]) -> Result<(), Vec<String>> {
    let mut missing = Vec::new();
    if pairs.len() < 40 {
        missing.push(format!("at least 40 pairs (found {})", pairs.len()));
    }
    let have: BTreeSet<&str> = pairs.iter().map(|p| p.category.as_str()).collect();
    for c in CATEGORIES {
        if !have.contains(c) {
            missing.push(c.to_string());
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(missing)
    }
}

fn parse_view<'a>(db: &'a Db, spec: &Option<String>) -> View<'a> {
    match spec.as_deref() {
        None | Some("now") => db.now(),
        Some("history") => db.history(),
        Some(s) if s.starts_with("asof:") => db.as_of(TimeRef::Tx(s[5..].parse().unwrap())),
        Some(s) if s.starts_with("asof-instant:") => {
            db.as_of(TimeRef::Instant(s[13..].parse().unwrap()))
        }
        Some(s) if s.starts_with("valid:") => db.now().valid_at(s[6..].parse().unwrap()),
        Some(other) => panic!("unknown view {other}"),
    }
}

/// Rows of a SPARQL result over the mapped variables.
pub fn sparql_rows(view: &View<'_>, p: &Pair) -> Result<Vec<Vec<Canon>>, String> {
    let r = view.sparql(&p.sparql).map_err(|e| format!("SPARQL: {e}"))?;
    let SparqlResult::Solutions(sol) = r else {
        return Err("SPARQL: not a SELECT".into());
    };
    let idx: Vec<usize> = p
        .columns
        .keys()
        .map(|v| {
            sol.col(v)
                .ok_or_else(|| format!("SPARQL: no column {v} in {:?}", sol.vars))
        })
        .collect::<Result<_, _>>()?;
    Ok(sol
        .rows
        .iter()
        .map(|r| idx.iter().map(|i| from_term(&r[*i])).collect())
        .collect())
}

/// Rows of a Cypher result over the mapped columns.
pub fn cypher_rows(view: &View<'_>, p: &Pair) -> Result<Vec<Vec<Canon>>, String> {
    let r = view
        .cypher(&p.cypher, &CypherParams::new())
        .map_err(|e| format!("Cypher: {e}"))?;
    let cols: Vec<(usize, bool)> = p
        .columns
        .iter()
        .map(|(sv, cc)| {
            r.columns
                .iter()
                .position(|c| c == cc)
                .map(|i| (i, p.tx_columns.contains(sv)))
                .ok_or_else(|| format!("Cypher: no column {cc} in {:?}", r.columns))
        })
        .collect::<Result<_, _>>()?;
    Ok(r.rows
        .iter()
        .map(|row| {
            cols.iter()
                .map(|(i, tx)| from_cypher(&row[*i], *tx))
                .collect()
        })
        .collect())
}

/// The outcome of one pair.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Pass,
    /// The SPARQL side was not run (the `sparql` feature is off).
    Pending,
    Fail(String),
}

fn report(p: &Pair, why: &str) -> String {
    format!(
        "pair `{}` (fixture `{}`)\n  SPARQL: {}\n  Cypher: {}\n{why}",
        p.name, p.fixture, p.sparql, p.cypher
    )
}

/// Runs one pair on a fresh fixture database.
pub fn run_pair(p: &Pair, sparql_enabled: bool) -> Outcome {
    let fx = fixtures::load(&p.fixture);
    let view = parse_view(&fx.db, &p.view);
    let mode = Mode {
        set: p.mode == "set",
        ordered: p.order == "ordered",
    };
    let cy = match cypher_rows(&view, p) {
        Ok(r) => r,
        Err(e) => return Outcome::Fail(report(p, &e)),
    };
    if let Some(n) = p.expect_cypher_rows {
        if cy.len() != n {
            return Outcome::Fail(report(
                p,
                &format!("  expected {n} Cypher rows, got {}", cy.len()),
            ));
        }
    }
    if !sparql_enabled {
        return Outcome::Pending;
    }
    let sp = match sparql_rows(&view, p) {
        Ok(r) => r,
        Err(e) => return Outcome::Fail(report(p, &e)),
    };
    if let Some(n) = p.expect_sparql_rows {
        if sp.len() != n {
            return Outcome::Fail(report(
                p,
                &format!("  expected {n} SPARQL rows, got {}", sp.len()),
            ));
        }
    }
    if p.expect_sparql_rows.is_some()
        && p.expect_cypher_rows.is_some()
        && p.expect_sparql_rows != p.expect_cypher_rows
    {
        // a divergence pair: the exact counts are the assertion
        return Outcome::Pass;
    }
    match diff(&apply_mode(sp, mode), &apply_mode(cy, mode)) {
        None => Outcome::Pass,
        Some(d) => Outcome::Fail(report(p, &d)),
    }
}
