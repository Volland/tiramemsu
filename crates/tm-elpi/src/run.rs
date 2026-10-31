//! Runs over a view: goal queries, constraint checks and conclusions
//! (OpenSpec change `add-elpi-engine`, `lat.md/elpi.md`).

use std::collections::{BTreeMap, BTreeSet, HashSet};

use tm_core::store::ReadStore;
use tm_core::{Eid, Error, Result, Valid, ValidSel, Value, ViewSpec};
use tm_store::read::View;

use crate::host::{Host, Reading, Seeds};
use crate::scan;
use crate::term::{list, to_eid, to_elpi_term, to_value, ElpiTerm, Tree};
use crate::valid::{intersect, merge, segments};

/// The declarations every program is loaded after. The host builtins are
/// declared here too, so clauses can call them, and again through the engine,
/// which routes them to the host.
pub const PRELUDE: &str = r#"
kind term type.
type iri string -> term.
type str string -> term.
type num int -> term.
type dbl float -> term.
type bool string -> term.
type lang string -> string -> term.
type lit string -> string -> term.
type node int -> term.
type bnode int -> term.
type tx int -> term.
kind stmt type.
type eid int -> stmt.
type stmtv stmt -> term.
kind bound type.
type at int -> bound.
type inf bound.
kind fact type.
type triple term -> term -> term -> fact.
kind severity type.
type error severity.
type warn severity.
pred tm.triple o:term, o:term, o:term, o:stmt.
pred tm.added o:term, o:term, o:term, o:stmt.
pred tm.retracted o:term, o:term, o:term, o:stmt.
pred tm.valid o:stmt, o:bound, o:bound.
pred tm.conclude o:fact, o:list stmt.
pred tm.violation o:severity, o:string, o:string, o:list stmt.
pred tm.violation-delta o:string, o:string, o:list stmt.
"#;

/// One part of a resolved program: a stored program or the given text.
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    /// The program node, for a stored part.
    pub node: Option<Value>,
    /// Its `elpi:source` statement, for a stored part.
    pub source: Option<Eid>,
    /// Its text, include lines removed.
    pub text: String,
}

/// A program ready to load: its parts, included ones first.
#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    /// The parts, in load order (the root last).
    pub parts: Vec<Part>,
}

impl Source {
    /// Program text with no includes.
    pub fn text(text: &str) -> Source {
        Source {
            parts: vec![Part {
                node: None,
                source: None,
                text: scan::strip_includes(text),
            }],
        }
    }

    /// The text the engine loads: the prelude, then every part.
    pub fn full_text(&self) -> String {
        let mut t = String::from(PRELUDE);
        for p in &self.parts {
            t.push('\n');
            t.push_str(&p.text);
        }
        t
    }

    /// True when any part may be non-monotonic.
    pub fn non_monotonic(&self) -> bool {
        self.parts.iter().any(|p| scan::non_monotonic(&p.text))
    }

    /// The source statements of the stored parts.
    pub fn sources(&self) -> Vec<Eid> {
        self.parts.iter().filter_map(|p| p.source).collect()
    }
}

/// Options of a run.
#[derive(Clone, Debug)]
pub struct ElpiOptions {
    /// The valid instant of queries and checks; the view's, else the clock, when `None`.
    pub valid_at: Option<i64>,
    /// At most this many solutions; 0 is all.
    pub limit: usize,
    /// Resolution steps; 0 is the engine's default.
    pub fuel: u64,
    /// At most this many rows from the builtins.
    pub max_rows: u64,
    /// Raw eids the builtins never return (a program's own conclusions).
    pub exclude: Vec<i64>,
}

impl Default for ElpiOptions {
    fn default() -> Self {
        ElpiOptions {
            valid_at: None,
            limit: 0,
            fuel: 0,
            max_rows: 10_000_000,
            exclude: Vec::new(),
        }
    }
}

/// One solution of a goal: each named variable's value.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    /// The bindings, by variable name.
    pub bindings: BTreeMap<String, ElpiTerm>,
}

/// What a run cost.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cost {
    /// Resolution steps the engine used.
    pub fuel_used: u64,
    /// Rows the builtins returned.
    pub rows_read: u64,
}

/// The rows of a goal.
#[derive(Clone, Debug, PartialEq)]
pub struct QueryResult {
    /// One row per solution.
    pub rows: Vec<Row>,
    /// False when `limit` cut the search short.
    pub exhausted: bool,
    /// What the run cost.
    pub cost: Cost,
}

/// `error` or `warn`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Severity {
    /// Blocks a commit when enforced.
    Error,
    /// Reported only.
    Warn,
}

impl Severity {
    /// `"error"` or `"warn"`.
    pub fn name(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warn => "warn",
        }
    }
}

/// One violation of a constraint.
#[derive(Clone, Debug, PartialEq)]
pub struct Violation {
    /// Its severity.
    pub severity: Severity,
    /// The constraint's name.
    pub name: String,
    /// The program's message.
    pub message: String,
    /// The statements it cites, in eid order.
    pub eids: Vec<Eid>,
    /// When it holds: the intersection of the cited statements' intervals.
    pub valid: Valid,
}

/// The violations of a check.
#[derive(Clone, Debug, PartialEq)]
pub struct CheckResult {
    /// Every violation found.
    pub violations: Vec<Violation>,
    /// What the run cost.
    pub cost: Cost,
}

/// One conclusion with its stated support.
#[derive(Clone, Debug, PartialEq)]
pub struct Conclusion {
    /// Subject.
    pub s: Value,
    /// Predicate (an IRI).
    pub p: Value,
    /// Object.
    pub o: Value,
    /// The statements it rests on, in eid order.
    pub support: Vec<Eid>,
    /// When it holds.
    pub valid: Valid,
}

/// The conclusions of a program.
#[derive(Clone, Debug, PartialEq)]
pub struct ConcludeResult {
    /// Every conclusion, once per support set and maximal interval.
    pub conclusions: Vec<Conclusion>,
    /// True when the program may be non-monotonic (evaluated per segment).
    pub non_monotonic: bool,
    /// What the runs cost.
    pub cost: Cost,
}

/// One solution as the engine returns it.
pub(crate) struct Sol {
    pub terms: BTreeMap<String, Tree>,
    pub printed: BTreeMap<String, String>,
}

/// The engine's answer to one goal, with the builtins' state.
pub(crate) struct Solved<'a> {
    pub sols: Vec<Sol>,
    pub exhausted: bool,
    pub fuel_used: u64,
    pub host: Host<'a>,
}

fn tx_view(store: &dyn ReadStore, spec: &ViewSpec) -> Result<View> {
    let mut v = View::resolve(store, spec)?;
    v.valid_at = None;
    Ok(v)
}

/// Checks that a program loads.
// @tg: implements:: [[openspec:elpi-engine#Goal queries over a view]] {since: "2026-10-31", confidence: partial}
pub fn validate(src: &Source) -> Result<()> {
    crate::engine::compile(&src.full_text()).map(|_| ())
}

/// The rows of `goal` over a view, read at one valid instant.
// @tg: implements:: [[openspec:elpi-engine#Goal queries over a view]] {since: "2026-10-31"}
// @tg: implements:: [[openspec:elpi-engine#Valid time of a run]] {since: "2026-10-31"}
// @tg: implements:: [[elpi#Host Callback Builtins]] {since: "2026-10-31"}
pub fn query(
    store: &dyn ReadStore,
    spec: &ViewSpec,
    clock: i64,
    src: &Source,
    goal: &str,
    opts: &ElpiOptions,
) -> Result<QueryResult> {
    let at = opts.valid_at.unwrap_or(match spec.valid {
        ValidSel::At(d) => d,
        ValidSel::Unfiltered => clock,
    });
    let view = tx_view(store, spec)?;
    let exclude: HashSet<i64> = opts.exclude.iter().copied().collect();
    let goal = goal
        .trim()
        .trim_start_matches("?-")
        .trim()
        .trim_end_matches('.');
    let host = Host::new(
        store,
        view,
        Reading::Instant(at),
        None,
        &exclude,
        opts.max_rows,
    );
    let s = crate::engine::solve(src, goal, opts, host)?;
    let rows = s
        .sols
        .iter()
        .map(|sol| Row {
            bindings: sol
                .terms
                .iter()
                .filter(|(k, _)| !k.starts_with('_'))
                .map(|(k, t)| {
                    (
                        k.clone(),
                        to_elpi_term(t, sol.printed.get(k).map(String::as_str)),
                    )
                })
                .collect(),
        })
        .collect();
    Ok(QueryResult {
        rows,
        exhausted: s.exhausted,
        cost: Cost {
            fuel_used: s.fuel_used,
            rows_read: s.host.rows,
        },
    })
}

fn eids_of(t: Option<&Tree>, host: &Host, what: &str) -> Result<Vec<Eid>> {
    let items = t
        .and_then(list)
        .ok_or_else(|| run_err(format!("{what}: the eid list is not a ground list")))?;
    let mut out = BTreeSet::new();
    for it in &items {
        let e = to_eid(it).ok_or_else(|| run_err(format!("{what}: {it:?} is not (eid N)")))?;
        if !host.seen.contains_key(&e) {
            return Err(run_err(format!(
                "{what}: statement {} was not read by this run",
                e.n()
            )));
        }
        out.insert(e);
    }
    Ok(out.into_iter().collect())
}

pub(crate) fn run_err(message: impl Into<String>) -> Error {
    Error::ElpiRun {
        message: message.into(),
    }
}

fn str_of(t: Option<&Tree>) -> Option<String> {
    match t {
        Some(Tree::Str(s)) => Some(s.clone()),
        _ => None,
    }
}

fn violations(s: &Solved, delta: bool) -> Result<Vec<Violation>> {
    let mut out = Vec::new();
    let mut keys = BTreeSet::new();
    for sol in &s.sols {
        let severity = if delta {
            Severity::Error
        } else {
            match sol.terms.get("Sev") {
                Some(Tree::Const(c)) if c == "error" => Severity::Error,
                Some(Tree::Const(c)) if c == "warn" => Severity::Warn,
                other => return Err(run_err(format!("tm.violation: severity {other:?}"))),
            }
        };
        let (Some(name), Some(message)) =
            (str_of(sol.terms.get("Name")), str_of(sol.terms.get("Msg")))
        else {
            return Err(run_err("tm.violation: name and message must be strings"));
        };
        let eids = eids_of(sol.terms.get("Eids"), &s.host, "tm.violation")?;
        let Some(valid) = intersect(eids.iter().filter_map(|e| s.host.seen.get(e))) else {
            continue;
        };
        if keys.insert((severity, name.clone(), message.clone(), eids.clone())) {
            out.push(Violation {
                severity,
                name,
                message,
                eids,
                valid,
            });
        }
    }
    Ok(out)
}

/// Every solution of `tm.violation Sev Name Msg Eids` over every statement of
/// the view (or, with `name`, of the `error` constraint `name` only).
// @tg: implements:: [[openspec:elpi-engine#Full constraint check]] {since: "2026-10-31"}
pub fn check(
    store: &dyn ReadStore,
    spec: &ViewSpec,
    src: &Source,
    name: Option<&str>,
    opts: &ElpiOptions,
) -> Result<CheckResult> {
    let view = tx_view(store, spec)?;
    let exclude: HashSet<i64> = opts.exclude.iter().copied().collect();
    let host = Host::new(store, view, Reading::All, None, &exclude, opts.max_rows);
    let goal = match name {
        Some(n) => format!("tm.violation Sev Name Msg Eids, Sev = error, Name = {n:?}"),
        None => "tm.violation Sev Name Msg Eids".to_string(),
    };
    let s = crate::engine::solve(src, &goal, opts, host)?;
    Ok(CheckResult {
        violations: violations(&s, false)?,
        cost: Cost {
            fuel_used: s.fuel_used,
            rows_read: s.host.rows,
        },
    })
}

/// Every solution of `tm.violation-delta Name Msg Eids`, with `tm.added` and
/// `tm.retracted` reading `seeds` (`lat.md/elpi#Two Constraint Entry Points`).
// @tg: implements:: [[elpi#Two Constraint Entry Points]] {since: "2026-10-31"}
pub fn check_delta(
    store: &dyn ReadStore,
    spec: &ViewSpec,
    src: &Source,
    seeds: &Seeds,
    opts: &ElpiOptions,
) -> Result<CheckResult> {
    let view = tx_view(store, spec)?;
    let exclude: HashSet<i64> = opts.exclude.iter().copied().collect();
    let host = Host::new(
        store,
        view,
        Reading::All,
        Some(seeds),
        &exclude,
        opts.max_rows,
    );
    let s = crate::engine::solve(src, "tm.violation-delta Name Msg Eids", opts, host)?;
    Ok(CheckResult {
        violations: violations(&s, true)?,
        cost: Cost {
            fuel_used: s.fuel_used,
            rows_read: s.host.rows,
        },
    })
}

type Key = (Value, Value, Value, Vec<Eid>);

fn conclusions_of(s: &Solved) -> Result<Vec<(Key, Valid)>> {
    let mut out = Vec::new();
    for sol in &s.sols {
        let fact = sol.terms.get("F");
        let Some(Tree::App(c, a)) = fact else {
            return Err(run_err(format!(
                "tm.conclude: {fact:?} is not (triple S P O)"
            )));
        };
        if c != "triple" || a.len() != 3 {
            return Err(run_err(format!("tm.conclude: {c} is not triple/3")));
        }
        let vals: Vec<Value> = a
            .iter()
            .map(|t| {
                to_value(t).ok_or_else(|| run_err(format!("tm.conclude: {t:?} is not a value")))
            })
            .collect::<Result<_>>()?;
        if !matches!(vals[1], Value::Iri(_)) {
            return Err(run_err("tm.conclude: the predicate must be (iri …)"));
        }
        let support = eids_of(sol.terms.get("S"), &s.host, "tm.conclude")?;
        if support.is_empty() {
            return Err(run_err("tm.conclude: empty support"));
        }
        let Some(valid) = intersect(support.iter().filter_map(|e| s.host.seen.get(e))) else {
            continue;
        };
        out.push((
            (vals[0].clone(), vals[1].clone(), vals[2].clone(), support),
            valid,
        ));
    }
    Ok(out)
}

/// Every solution of `tm.conclude F S`: conclusions with the support they
/// state, valid over the intersection of its intervals; a program that may be
/// non-monotonic is evaluated per valid-time segment.
// @tg: implements:: [[openspec:elpi-engine#Conclusions carry stated support]] {since: "2026-10-31"}
// @tg: implements:: [[elpi#Program States Its Support]] {since: "2026-10-31"}
// @tg: implements:: [[elpi#Pointwise Valid Time]] {since: "2026-10-31"}
pub fn conclude(
    store: &dyn ReadStore,
    spec: &ViewSpec,
    src: &Source,
    opts: &ElpiOptions,
) -> Result<ConcludeResult> {
    let view = tx_view(store, spec)?;
    let exclude: HashSet<i64> = opts.exclude.iter().copied().collect();
    let host = Host::new(store, view, Reading::All, None, &exclude, opts.max_rows);
    let first = crate::engine::solve(src, "tm.conclude F S", opts, host)?;
    let mut cost = Cost {
        fuel_used: first.fuel_used,
        rows_read: first.host.rows,
    };
    let non_monotonic = src.non_monotonic();
    let mut found: BTreeMap<String, (Key, Vec<Valid>)> = BTreeMap::new();
    let key_of = |k: &Key| format!("{:?}", k);
    if !non_monotonic {
        for (k, v) in conclusions_of(&first)? {
            found
                .entry(key_of(&k))
                .or_insert_with(|| (k.clone(), Vec::new()))
                .1
                .push(v);
        }
    } else {
        let segs = segments(first.host.seen.values());
        for seg in segs {
            let host = Host::new(
                store,
                view,
                Reading::Segment(seg),
                None,
                &exclude,
                opts.max_rows,
            );
            let s = crate::engine::solve(src, "tm.conclude F S", opts, host)?;
            cost.fuel_used += s.fuel_used;
            cost.rows_read += s.host.rows;
            for (k, v) in conclusions_of(&s)? {
                let Some(v) = intersect([&v, &seg]) else {
                    continue;
                };
                found
                    .entry(key_of(&k))
                    .or_insert_with(|| (k.clone(), Vec::new()))
                    .1
                    .push(v);
            }
        }
    }
    let mut conclusions = Vec::new();
    for (_, ((s, p, o, support), vs)) in found {
        let mut vs = vs;
        vs.sort_by_key(|v| (v.from.is_some(), v.from));
        vs.dedup();
        for valid in merge(vs) {
            conclusions.push(Conclusion {
                s: s.clone(),
                p: p.clone(),
                o: o.clone(),
                support: support.clone(),
                valid,
            });
        }
    }
    Ok(ConcludeResult {
        conclusions,
        non_monotonic,
        cost,
    })
}
