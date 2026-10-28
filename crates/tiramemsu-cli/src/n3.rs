//! `tm reason`, `tm rules put|show` and `tm materialize` (feature `n3`).
//!
//! Instants are epoch milliseconds or RFC 3339 dates and date-times
//! (`2024-06-01`, `2024-06-01T12:00:00Z`); a valid-time window is
//! `<from>..<to>` with either side left empty for an open end.

use std::path::PathBuf;

use clap::{Args, Subcommand};
use tiramemsu::value::{parse_date, parse_datetime};
use tiramemsu::*;

use crate::{io, open};

/// `tm reason`.
#[derive(Args)]
pub struct ReasonArgs {
    file: PathBuf,
    /// A file of N3 rules.
    #[arg(
        long,
        conflicts_with = "rule_set",
        required_unless_present = "rule_set"
    )]
    rules: Option<PathBuf>,
    /// A rule set stored with `tm rules put` (an IRI).
    #[arg(long)]
    rule_set: Option<String>,
    /// Reason over memory as of a transaction number.
    #[arg(long, conflicts_with = "as_of_instant")]
    as_of: Option<u64>,
    /// Reason over memory as of an instant.
    #[arg(long)]
    as_of_instant: Option<String>,
    /// Reason over the statements valid at one instant.
    #[arg(long, conflicts_with = "valid_during")]
    valid_at: Option<String>,
    /// Report conclusions inside this valid-time window (`<from>..<to>`).
    #[arg(long)]
    valid_during: Option<String>,
    /// Reason over the members of this named graph (repeatable).
    #[arg(long = "graph")]
    graphs: Vec<String>,
    /// Ignore valid time.
    #[arg(long)]
    atemporal: bool,
    /// The most valid-time segments to reason over.
    #[arg(long)]
    max_segments: Option<usize>,
    /// Print each conclusion's proof and the reasoner's N3 proof.
    #[arg(long)]
    proof: bool,
    /// Print JSON (the bridge's `reason` result) instead of text.
    #[arg(long)]
    json: bool,
}

/// `tm rules`.
#[derive(Subcommand)]
pub enum RulesCmd {
    /// Store N3 rules as a rule set (superseding an older version).
    Put {
        file: PathBuf,
        /// The rule set (an IRI).
        iri: String,
        /// The N3 rule file.
        path: PathBuf,
    },
    /// Print a stored rule set.
    Show {
        file: PathBuf,
        /// The rule set (an IRI).
        iri: String,
        /// Print JSON (text, statement, classification) instead of the rules.
        #[arg(long)]
        json: bool,
    },
}

/// `tm materialize`.
#[derive(Args)]
pub struct MaterializeArgs {
    file: PathBuf,
    /// The stored rule set (an IRI).
    iri: String,
    /// Materialize conclusions inside this valid-time window (`<from>..<to>`).
    #[arg(long)]
    valid_during: Option<String>,
    /// Reason over the members of this named graph (repeatable).
    #[arg(long = "graph")]
    graphs: Vec<String>,
    /// Refuse to write when the reasoning run is incomplete.
    #[arg(long)]
    require_complete: bool,
    /// Report what would be derived and write nothing.
    #[arg(long)]
    dry_run: bool,
    /// The most valid-time segments to reason over.
    #[arg(long)]
    max_segments: Option<usize>,
    /// Print JSON (the bridge's `materialize` report) instead of text.
    #[arg(long)]
    json: bool,
}

fn usage(msg: impl Into<String>) -> Error {
    Error::invalid_query(msg)
}

/// An instant: epoch milliseconds, or an RFC 3339 date or date-time.
pub fn instant(s: &str) -> Result<i64> {
    let s = s.trim();
    if let Ok(ms) = s.parse::<i64>() {
        return Ok(ms);
    }
    parse_datetime(s)
        .map(|(ms, _)| ms)
        .or_else(|| parse_date(s).map(|d| d * 86_400_000))
        .ok_or_else(|| {
            usage(format!(
                "{s:?} is not an instant (epoch milliseconds or an RFC 3339 date or date-time)"
            ))
        })
}

/// A window `<from>..<to>`; an empty side is an open end.
pub fn window(s: &str) -> Result<Valid> {
    let (from, to) = s
        .split_once("..")
        .ok_or_else(|| usage(format!("{s:?} is not a window <from>..<to>")))?;
    let side = |x: &str| {
        if x.trim().is_empty() {
            Ok(None)
        } else {
            instant(x).map(Some)
        }
    };
    Ok(Valid {
        from: side(from)?,
        to: side(to)?,
    })
}

fn scope(graphs: &[String]) -> ReasonScope {
    if graphs.is_empty() {
        ReasonScope::All
    } else {
        ReasonScope::Graphs(graphs.iter().map(|g| Value::iri(g.clone())).collect())
    }
}

const PREFIXES: &[(&str, &str)] = &[
    ("v:", "urn:tiramemsu:v:"),
    ("n3:", "urn:tiramemsu:n3:"),
    ("rdf:", "http://www.w3.org/1999/02/22-rdf-syntax-ns#"),
    ("rdfs:", "http://www.w3.org/2000/01/rdf-schema#"),
    ("xsd:", "http://www.w3.org/2001/XMLSchema#"),
    ("owl:", "http://www.w3.org/2002/07/owl#"),
];

/// A value for a person: IRIs under a known namespace as prefixed names.
fn value(v: &Value) -> String {
    match v {
        Value::Iri(iri) => PREFIXES
            .iter()
            .find_map(|(p, ns)| {
                iri.strip_prefix(ns)
                    .filter(|local| !local.is_empty() && !local.contains(['/', '#', ':']))
                    .map(|local| format!("{p}{local}"))
            })
            .unwrap_or_else(|| v.to_string()),
        Value::Int(i) => i.to_string(),
        Value::Bool(b) => b.to_string(),
        other => other.to_string(),
    }
}

fn term(t: &N3Term) -> String {
    match t {
        N3Term::Value(v) => value(v),
        other => other.to_string(),
    }
}

fn bound(b: Option<i64>, open: &str) -> String {
    b.map_or(open.to_string(), |ms| rdf::datetime_lexical(ms, Some(0)))
}

fn interval(v: &Valid) -> String {
    if v.from.is_none() && v.to.is_none() {
        return "always".into();
    }
    format!("[{}, {})", bound(v.from, "-∞"), bound(v.to, "∞"))
}

fn status(s: &ReasonStatus) -> String {
    match s {
        ReasonStatus::Complete => "complete".into(),
        ReasonStatus::Incomplete { limits } => format!("incomplete ({limits:?})"),
    }
}

fn conclusion_line(c: &Conclusion) -> String {
    format!(
        "({} {} {})  {}",
        term(&c.s),
        term(&c.p),
        term(&c.o),
        interval(&c.valid)
    )
}

fn print_json(j: &serde_json::Value) {
    println!("{j}");
}

fn bridge(e: tiramemsu_json::BindError) -> Error {
    match e {
        tiramemsu_json::BindError::Db(e) => e,
        tiramemsu_json::BindError::Arg(m) => usage(m),
    }
}

/// `tm reason`.
// @tg: implements:: [[openspec:n3-reasoning#Reasoning surfaces]] {since: "2026-10-28"}
pub fn reason(a: ReasonArgs) -> Result<()> {
    let db = open(&a.file)?;
    let rules = match (&a.rules, &a.rule_set) {
        (Some(p), _) => Rules::Text(std::fs::read_to_string(p).map_err(io)?),
        (_, Some(iri)) => Rules::Stored(Value::iri(iri.clone())),
        _ => unreachable!("clap requires one of --rules and --rule-set"),
    };
    let view = match (a.as_of, &a.as_of_instant) {
        (Some(t), _) => db.as_of(TimeRef::Tx(t)),
        (_, Some(i)) => db.as_of(TimeRef::Instant(instant(i)?)),
        _ => db.now(),
    };
    let view = match &a.valid_at {
        Some(i) => view.valid_at(instant(i)?),
        None => view,
    };
    let mut opts = ReasonOptions {
        scope: scope(&a.graphs),
        proof_n3: a.proof,
        ..ReasonOptions::default()
    };
    if let Some(w) = &a.valid_during {
        opts.window = window(w)?;
    }
    if a.atemporal {
        opts.time = TimeMode::Atemporal;
    }
    if let Some(n) = a.max_segments {
        opts.max_segments = n;
    }
    let r = view.reason(rules, &opts)?;
    if a.json {
        print_json(&tiramemsu_json::n3::reason_json(&r));
        return Ok(());
    }
    for (i, c) in r.conclusions.iter().enumerate() {
        println!("{}", conclusion_line(c));
        if a.proof {
            let premises: Vec<String> = c
                .proof
                .premises
                .iter()
                .map(|p| match p {
                    Premise::Statement(e) => format!("stmt {}", e.n()),
                    Premise::Conclusion(j) => format!("conclusion {}", j + 1),
                })
                .collect();
            println!(
                "    #{} by rule {} from {}",
                i + 1,
                c.proof.rule,
                premises.join(", ")
            );
        }
    }
    eprintln!(
        "{} conclusions; {} statements loaded{}, {} segments, {}, t={}",
        r.conclusions.len(),
        r.loaded,
        if r.pruned { " (pruned)" } else { "" },
        r.segments,
        status(&r.status),
        r.t
    );
    if let Some(c) = &r.contradiction {
        eprintln!(
            "contradiction: fuse rule {} fired in {}: {}",
            c.rule,
            interval(&c.valid),
            c.instance.join(" ")
        );
    }
    for e in &r.errors {
        eprintln!("builtin error: {e}");
    }
    if let Some(p) = &r.proof_n3 {
        println!("{p}");
    }
    Ok(())
}

/// `tm rules put|show`.
// @tg: implements:: [[openspec:n3-rule-sets#Rule-set surfaces]] {since: "2026-10-28"}
pub fn rules(cmd: RulesCmd) -> Result<()> {
    match cmd {
        RulesCmd::Put { file, iri, path } => {
            let db = open(&file)?;
            let text = std::fs::read_to_string(&path).map_err(io)?;
            let rs = Value::iri(iri.clone());
            let mut eid = None;
            let report = db.transact(TxOptions::default(), |tx| {
                eid = Some(tx.store_rules(rs.clone(), &text)?);
                Ok(())
            })?;
            let eid = eid.expect("the body ran").n();
            let info = db.now().rule_set_info(&rs)?;
            if report.asserted.is_empty() {
                println!("{iri}: unchanged (statement {eid})");
            } else if report.superseded.is_empty() {
                println!("{iri}: stored as statement {eid} (t={})", report.t.0);
            } else {
                println!(
                    "{iri}: new version, statement {eid} (t={}); the old version and its conclusions are retracted",
                    report.t.0
                );
            }
            println!("{} rules, {}", info.rules, monotonicity(&info.monotonicity));
            Ok(())
        }
        RulesCmd::Show { file, iri, json } => {
            let db = open(&file)?;
            let view = db.now();
            let j = tiramemsu_json::n3::rule_set_json(&view, &Value::iri(iri.clone()))
                .map_err(bridge)?;
            if json {
                print_json(&j);
            } else {
                println!(
                    "# rule set {iri}: statement {}, {} rules, {}",
                    j["statement"],
                    j["rules"],
                    monotonicity(&view.rule_set_info(&Value::iri(iri.clone()))?.monotonicity)
                );
                println!("{}", j["text"].as_str().unwrap_or_default());
            }
            Ok(())
        }
    }
}

fn monotonicity(m: &Monotonicity) -> String {
    match m {
        Monotonicity::Monotonic => "monotonic".into(),
        Monotonicity::NonMonotonic { rule, builtin } => {
            format!("non-monotonic (rule {rule} uses {builtin}; reason only)")
        }
        Monotonicity::ClockDependent { rule, builtin } => {
            format!("clock-dependent (rule {rule} uses {builtin}; reason only)")
        }
        Monotonicity::UntracedPremises { rule, builtin } => {
            format!("untraced premises (rule {rule} uses {builtin}; reason only)")
        }
    }
}

/// `tm materialize`.
// @tg: implements:: [[openspec:n3-materialization#Materialization surfaces]] {since: "2026-10-28"}
pub fn materialize(a: MaterializeArgs) -> Result<()> {
    let db = open(&a.file)?;
    let rs = Value::iri(a.iri.clone());
    let mut opts = MaterializeOptions {
        scope: scope(&a.graphs),
        require_complete: a.require_complete,
        dry_run: a.dry_run,
        ..MaterializeOptions::default()
    };
    if let Some(w) = &a.valid_during {
        opts.window = window(w)?;
    }
    if let Some(n) = a.max_segments {
        opts.max_segments = n;
    }
    let r = db.materialize(&rs, &opts)?;
    if a.json {
        print_json(&tiramemsu_json::n3::materialize_json(&db, &r).map_err(bridge)?);
        return Ok(());
    }
    if r.dry_run {
        // what would be derived: the conclusions not yet in memory
        let preview = db.now().reason(
            Rules::Stored(rs.clone()),
            &ReasonOptions {
                scope: opts.scope.clone(),
                window: opts.window,
                max_segments: opts.max_segments,
                limits: opts.limits,
                ..ReasonOptions::default()
            },
        )?;
        for c in &preview.conclusions {
            println!("{}", conclusion_line(c));
        }
    }
    let verb = if r.dry_run { "would derive" } else { "derived" };
    println!(
        "{}: {verb} {}, existing {}, widened {}, stale support {}, unstorable {} ({}, reasoned at t={})",
        a.iri,
        r.derived.len(),
        r.existing.len(),
        r.widened.len(),
        r.stale_support.len(),
        r.unstorable.len(),
        status(&r.status),
        r.reasoned_at
    );
    match (&r.commit, r.dry_run) {
        (_, true) => println!("dry run: nothing written"),
        (Some(c), false) => println!("committed t={}", c.t.0),
        (None, false) => println!("nothing to write"),
    }
    Ok(())
}
