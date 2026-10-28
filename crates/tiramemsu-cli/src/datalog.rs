//! `tm datalog`, `tm check`, `tm materialize-program` and
//! `tm program put|show|enforce|unenforce` (feature `datalog`).
//!
//! A program argument is a file of Datalog text, or, when no such file exists,
//! the IRI of a program stored with `tm program put`. Instants and windows are
//! read as [`crate::reasoning`] describes.

use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use tiramemsu::*;

use crate::reasoning::{self, interval, print_json, usage, window};
use crate::{io, open};

/// The view and run options the Datalog reads share.
#[derive(Args)]
pub struct RunArgs {
    /// Run over memory as of a transaction number.
    #[arg(long, conflicts_with = "as_of_instant")]
    as_of: Option<u64>,
    /// Run over memory as of an instant.
    #[arg(long)]
    as_of_instant: Option<String>,
    /// Run over the statements valid at one instant.
    #[arg(long, conflicts_with = "valid_during")]
    valid_at: Option<String>,
    /// Clip results to this valid-time window (`<from>..<to>`).
    #[arg(long)]
    valid_during: Option<String>,
    /// Read the members of this named graph (repeatable).
    #[arg(long = "graph")]
    graphs: Vec<String>,
    /// Ignore valid time.
    #[arg(long)]
    atemporal: bool,
    /// Fail with `DatalogIncomplete` instead of printing partial results.
    #[arg(long)]
    require_complete: bool,
}

/// `tm datalog`.
#[derive(Args)]
pub struct DatalogArgs {
    file: PathBuf,
    /// A file of Datalog text, or the IRI of a stored program.
    program: String,
    /// The goal, such as `reports(v:alice, X)` (default: the program's first `?-` goal).
    goal: Option<String>,
    #[command(flatten)]
    run: RunArgs,
    /// Evaluate the whole program, not only what the goal demands.
    #[arg(long)]
    no_demand: bool,
    /// Print each row's derivation per interval.
    #[arg(long)]
    proof: bool,
    /// Print the plan (strata, rule orders, access) instead of the rows.
    #[arg(long, conflicts_with_all = ["proof", "json"])]
    explain: bool,
    /// Print JSON (the bridge's `datalog` result) instead of text.
    #[arg(long)]
    json: bool,
}

/// `tm check`.
#[derive(Args)]
pub struct CheckArgs {
    file: PathBuf,
    /// A file of Datalog text, or the IRI of a stored program.
    program: String,
    /// Check only the constraint with this name (repeatable).
    #[arg(long = "only")]
    only: Vec<String>,
    #[command(flatten)]
    run: RunArgs,
    /// Print JSON (the bridge's `check` result) instead of text.
    #[arg(long)]
    json: bool,
}

/// `tm materialize-program`.
#[derive(Args)]
pub struct MaterializeProgramArgs {
    file: PathBuf,
    /// The stored program (an IRI; a file of text cannot be materialized).
    program: String,
    /// Materialize conclusions inside this valid-time window (`<from>..<to>`).
    #[arg(long)]
    valid_during: Option<String>,
    /// Read the members of this named graph (repeatable).
    #[arg(long = "graph")]
    graphs: Vec<String>,
    /// Refuse to write when the run is incomplete.
    #[arg(long)]
    require_complete: bool,
    /// Report what would be derived and write nothing.
    #[arg(long)]
    dry_run: bool,
    /// Print JSON (the bridge's `materializeProgram` report) instead of text.
    #[arg(long)]
    json: bool,
}

/// `tm program`.
#[derive(Subcommand)]
pub enum ProgramCmd {
    /// Store Datalog text as a program (superseding an older version).
    Put {
        file: PathBuf,
        /// The program (an IRI).
        iri: String,
        /// The Datalog file.
        path: PathBuf,
    },
    /// Print a stored program.
    Show {
        file: PathBuf,
        /// The program (an IRI).
        iri: String,
        /// Print JSON (text, statement, analysis, enforced) instead of the program.
        #[arg(long)]
        json: bool,
    },
    /// Enforce a stored program's `error` constraints at every commit. Fails with
    /// `ConstraintViolated` (and lists the violations) when memory already
    /// violates one, or `ConstraintNotEnforceable`.
    Enforce {
        file: PathBuf,
        /// The program (an IRI).
        iri: String,
    },
    /// Stop enforcing a stored program's constraints.
    Unenforce {
        file: PathBuf,
        /// The program (an IRI).
        iri: String,
    },
}

const PREFIXES: &[(&str, &str)] = &[
    ("v:", "urn:tiramemsu:v:"),
    ("dl:", "urn:tiramemsu:dl:"),
    ("sys:", "urn:tiramemsu:sys:"),
    ("tm:", "urn:tiramemsu:tm:"),
    ("rdf:", "http://www.w3.org/1999/02/22-rdf-syntax-ns#"),
    ("rdfs:", "http://www.w3.org/2000/01/rdf-schema#"),
    ("xsd:", "http://www.w3.org/2001/XMLSchema#"),
    ("owl:", "http://www.w3.org/2002/07/owl#"),
];

fn term(t: &DatalogTerm) -> String {
    match t {
        DatalogTerm::Value(v) => reasoning::value(PREFIXES, v),
        other => other.to_string(),
    }
}

fn intervals(valid: &[Valid]) -> String {
    valid.iter().map(interval).collect::<Vec<_>>().join(", ")
}

fn status(s: &DatalogStatus) -> String {
    match s {
        DatalogStatus::Complete => "complete".into(),
        DatalogStatus::Incomplete { limits } => format!("incomplete ({})", limits.join(", ")),
    }
}

fn proof_line(p: &DatalogProof) -> String {
    let premises: Vec<String> = p
        .premises
        .iter()
        .map(|x| match x {
            DatalogPremise::Statement(e) => format!("stmt {}", e.n()),
            DatalogPremise::Fact(i) => format!("fact {}", i + 1),
        })
        .collect();
    match p.rule {
        Some(r) => format!("by rule {r} from {}", premises.join(", ")),
        None => format!("from {}", premises.join(", ")),
    }
}

/// The program: a Datalog file, or a stored program's IRI.
fn program(arg: &str) -> Result<DatalogProgram> {
    if Path::new(arg).is_file() {
        return Ok(DatalogProgram::Text(
            std::fs::read_to_string(arg).map_err(io)?,
        ));
    }
    if !arg.contains(':') {
        return Err(usage(format!(
            "{arg}: no such program file (or give the IRI of a stored program)"
        )));
    }
    Ok(DatalogProgram::Stored(Value::iri(arg)))
}

/// The view and options of a run.
fn prepare<'a>(db: &'a Db, a: &RunArgs) -> Result<(View<'a>, DatalogOptions)> {
    let view = reasoning::view(
        db,
        a.as_of,
        a.as_of_instant.as_deref(),
        a.valid_at.as_deref(),
    )?;
    let mut opts = DatalogOptions {
        require_complete: a.require_complete,
        ..DatalogOptions::default()
    };
    if !a.graphs.is_empty() {
        opts.scope = DatalogScope::Graphs(a.graphs.iter().map(|g| Value::iri(g.clone())).collect());
    }
    if let Some(w) = &a.valid_during {
        opts.window = Some(window(w)?);
    }
    if a.atemporal {
        opts.time = DatalogTime::Atemporal;
    }
    Ok((view, opts))
}

/// `tm datalog`.
// @tg: implements:: [[openspec:datalog-programs#Datalog read surfaces]] {since: "2026-10-28"}
pub fn datalog(a: DatalogArgs) -> Result<()> {
    let db = open(&a.file)?;
    let p = program(&a.program)?;
    let (view, mut opts) = prepare(&db, &a.run)?;
    opts.demand = !a.no_demand;
    opts.proof = a.proof;
    if a.explain {
        let e = view.explain_datalog(&p, a.goal.as_deref(), &opts)?;
        for line in &e.plan {
            println!("{line}");
        }
        eprintln!("{} rows, {}", e.result.rows.len(), status(&e.result.status));
        return Ok(());
    }
    let r = view.datalog(&p, a.goal.as_deref(), &opts)?;
    if a.json {
        print_json(&tiramemsu_json::datalog::result_json(&r));
        return Ok(());
    }
    for row in &r.rows {
        let binding = if r.columns.is_empty() {
            "true".to_string()
        } else {
            r.columns
                .iter()
                .zip(&row.values)
                .map(|(c, v)| format!("{c}={}", term(v)))
                .collect::<Vec<_>>()
                .join(" ")
        };
        println!("{binding}  {}", intervals(&row.valid));
        for f in row.proof.iter().flatten() {
            println!("    {} {}", interval(&f.valid), proof_line(&f.proof));
        }
    }
    if a.proof {
        for (i, f) in r.facts.iter().enumerate() {
            let args: Vec<String> = f.args.iter().map(term).collect();
            println!(
                "fact {}: {}({})  {}{}",
                i + 1,
                f.relation,
                args.join(", "),
                intervals(&f.valid),
                f.proof
                    .as_ref()
                    .map_or(String::new(), |p| format!(" {}", proof_line(p)))
            );
        }
    }
    eprintln!(
        "{} rows; {} strata, {} facts derived, {} predicates read{}, {}, t={}",
        r.rows.len(),
        r.strata,
        r.derived,
        r.predicates.len(),
        if r.pruned { " (pruned)" } else { "" },
        status(&r.status),
        r.t
    );
    Ok(())
}

/// `tm check`: prints each violated constraint with its violations; violations
/// are a result, not an error, so the exit status is 0.
// @tg: implements:: [[openspec:datalog-constraints#Check constraints]] {since: "2026-10-28"}
pub fn check(a: CheckArgs) -> Result<()> {
    let db = open(&a.file)?;
    let p = program(&a.program)?;
    let (view, opts) = prepare(&db, &a.run)?;
    let r = view.check(&p, &a.only, &opts)?;
    if a.json {
        print_json(&tiramemsu_json::datalog::check_json(&r));
        return Ok(());
    }
    for c in &r.violated {
        println!(
            "{} ({}): {} violations",
            c.name,
            c.severity.name(),
            c.violations.len()
        );
        for v in &c.violations {
            let bindings: Vec<String> = v
                .bindings
                .iter()
                .map(|(k, t)| format!("{k}={}", term(t)))
                .collect();
            let eids: Vec<String> = v.eids.iter().map(|e| e.n().to_string()).collect();
            println!(
                "    {}  {}  stmts {}",
                bindings.join(" "),
                intervals(&v.valid),
                eids.join(", ")
            );
        }
    }
    eprintln!(
        "{} constraints checked, {} violated; {}, t={}",
        r.checked.len(),
        r.violated.len(),
        status(&r.status),
        r.t
    );
    Ok(())
}

/// `tm materialize-program`: writes a stored program's conclusions as derived
/// beliefs; with `--dry-run`, lists them and writes nothing.
// @tg: implements:: [[openspec:datalog-materialization#Materialization surfaces]]
pub fn materialize_program(a: MaterializeProgramArgs) -> Result<()> {
    let db = open(&a.file)?;
    if Path::new(&a.program).is_file() || !a.program.contains(':') {
        return Err(usage(format!(
            "{}: materialization needs the IRI of a stored program (see `tm program put`)",
            a.program
        )));
    }
    let stored = Value::iri(a.program.clone());
    let mut opts = MaterializeProgramOptions {
        require_complete: a.require_complete,
        dry_run: a.dry_run,
        ..MaterializeProgramOptions::default()
    };
    if !a.graphs.is_empty() {
        opts.scope = DatalogScope::Graphs(a.graphs.iter().map(|g| Value::iri(g.clone())).collect());
    }
    if let Some(w) = &a.valid_during {
        opts.window = Some(window(w)?);
    }
    let r = db.materialize_program(&stored, &opts)?;
    if a.json {
        let j = tiramemsu_json::datalog::materialize_json(&db, &r).map_err(|e| match e {
            tiramemsu_json::BindError::Db(e) => e,
            tiramemsu_json::BindError::Arg(m) => usage(m),
        })?;
        print_json(&j);
        return Ok(());
    }
    if r.dry_run {
        // what would be derived: the conclusions not yet in memory
        let preview = db.now().datalog_reason(
            &DatalogProgram::Stored(stored),
            &DatalogOptions {
                scope: opts.scope.clone(),
                window: opts.window,
                limits: opts.limits,
                ..DatalogOptions::default()
            },
        )?;
        for c in &preview.conclusions {
            println!(
                "({} {} {})  {}",
                term(&c.s),
                reasoning::value(PREFIXES, &c.p),
                term(&c.o),
                interval(&c.valid)
            );
        }
    }
    let list = |label: &str, eids: Vec<u64>| {
        if !eids.is_empty() {
            let ids: Vec<String> = eids.iter().map(u64::to_string).collect();
            println!("{label}: stmts {}", ids.join(", "));
        }
    };
    if !r.dry_run {
        list("derived", r.derived.iter().map(|e| e.n()).collect());
    }
    list("existing", r.existing.iter().map(|e| e.n()).collect());
    list("widened", r.widened.iter().map(|(_, n)| n.n()).collect());
    list("reconciled", r.reconciled.iter().map(|e| e.n()).collect());
    if !r.needs_rerun_after_writes.is_empty() {
        println!(
            "rerun after writes to: {}",
            r.needs_rerun_after_writes.join(", ")
        );
    }
    let verb = if r.dry_run { "would derive" } else { "derived" };
    eprintln!(
        "{}: {verb} {}, existing {}, widened {}, reconciled {}, stale support {}, unstorable {} ({}, reasoned at t={})",
        a.program,
        r.derived.len(),
        r.existing.len(),
        r.widened.len(),
        r.reconciled.len(),
        r.stale_support.len(),
        r.unstorable.len(),
        status(&r.status),
        r.reasoned_at
    );
    match (&r.commit, r.dry_run) {
        (_, true) => eprintln!("dry run: nothing written"),
        (Some(c), false) => eprintln!("committed t={}", c.t.0),
        (None, false) => eprintln!("nothing to write"),
    }
    Ok(())
}

fn summary(info: &ProgramInfo) -> String {
    let a = &info.analysis;
    format!(
        "{} rules, {} goals, {} constraints, {} strata",
        a.rules,
        a.goals,
        a.constraints.len(),
        a.strata.len()
    )
}

/// The live `(program dl:enforced true)` statement in `view`, if any.
fn enforced(view: &View<'_>, program: &Value) -> Result<Option<Eid>> {
    let (Some(s), Some(p)) = (
        view.encode(program)?,
        view.encode(&Value::iri(vocab::DL_ENFORCED))?,
    ) else {
        return Ok(None);
    };
    Ok(view.triples(Some(s), Some(p), None)?.first().map(|t| t.eid))
}

/// Lists the violations of a `ConstraintViolated` refusal on stderr, one per
/// line in the form `tm check` prints them; nothing for any other error.
pub fn print_violations(e: &Error) {
    let Error::ConstraintViolated { violations, .. } = e else {
        return;
    };
    for v in violations {
        let bindings: Vec<String> = v
            .bindings
            .iter()
            .map(|(k, t)| format!("{k}={}", reasoning::value(PREFIXES, t)))
            .collect();
        let eids: Vec<String> = v.eids.iter().map(|e| e.n().to_string()).collect();
        eprintln!(
            "    {}  {}  stmts {}",
            bindings.join(" "),
            intervals(&v.valid),
            eids.join(", ")
        );
    }
}

/// `tm program put|show|enforce|unenforce`.
// @tg: implements:: [[openspec:datalog-programs#Store a program]] {since: "2026-10-28"}
pub fn programs(cmd: ProgramCmd) -> Result<()> {
    match cmd {
        ProgramCmd::Put { file, iri, path } => {
            let db = open(&file)?;
            let text = std::fs::read_to_string(&path).map_err(io)?;
            let node = Value::iri(iri.clone());
            let mut eid = None;
            let report = db.transact(TxOptions::default(), |tx| {
                eid = Some(tx.store_program(node.clone(), &text)?);
                Ok(())
            })?;
            let eid = eid.expect("the body ran").n();
            let info = db.now().program_info(&DatalogProgram::Stored(node))?;
            if report.asserted.is_empty() {
                println!("{iri}: unchanged (statement {eid})");
            } else if report.superseded.is_empty() {
                println!("{iri}: stored as statement {eid} (t={})", report.t.0);
            } else {
                println!(
                    "{iri}: new version, statement {eid} (t={}); the old version stays visible as of earlier transactions",
                    report.t.0
                );
            }
            println!("{}", summary(&info));
            Ok(())
        }
        ProgramCmd::Show { file, iri, json } => {
            let db = open(&file)?;
            let node = Value::iri(iri.clone());
            let view = db.now();
            let info = view.program_info(&DatalogProgram::Stored(node.clone()))?;
            let on = enforced(&view, &node)?;
            if json {
                let mut j = tiramemsu_json::datalog::info_json(&info);
                j["enforced"] = on.is_some().into();
                j["enforcedStatement"] = on.map(|e| e.n()).into();
                print_json(&j);
            } else {
                println!(
                    "# program {iri}: statement {}, {}, {}",
                    info.statement
                        .map_or("-".to_string(), |e| e.n().to_string()),
                    summary(&info),
                    on.map_or("not enforced".to_string(), |e| format!(
                        "enforced (statement {})",
                        e.n()
                    ))
                );
                println!("{}", info.text);
            }
            Ok(())
        }
        // @tg: implements:: [[openspec:datalog-constraints#Constraint surfaces]] {since: "2026-10-28"}
        ProgramCmd::Enforce { file, iri } => {
            let db = open(&file)?;
            let node = Value::iri(iri.clone());
            let was = enforced(&db.now(), &node)?;
            let mut eid = None;
            let report = db.transact(TxOptions::default(), |tx| {
                eid = tx.enforce(node.clone(), true)?;
                Ok(())
            })?;
            let eid = eid.map_or("-".to_string(), |e| e.n().to_string());
            if was.is_some() {
                println!("{iri}: already enforced (statement {eid})");
            } else {
                println!(
                    "{iri}: enforced as statement {eid} (t={}); its error constraints hold now and are checked at every commit",
                    report.t.0
                );
            }
            Ok(())
        }
        ProgramCmd::Unenforce { file, iri } => {
            let db = open(&file)?;
            let node = Value::iri(iri.clone());
            if enforced(&db.now(), &node)?.is_none() {
                println!("{iri}: not enforced");
                return Ok(());
            }
            let report = db.transact(TxOptions::default(), |tx| {
                tx.enforce(node.clone(), false)?;
                Ok(())
            })?;
            println!("{iri}: no longer enforced (t={})", report.t.0);
            Ok(())
        }
    }
}
