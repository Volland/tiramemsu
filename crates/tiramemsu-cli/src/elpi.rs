//! `tm elpi query|check|conclude|materialize` and `tm elpi program
//! put|show|enforce|unenforce` (feature `elpi`).
//!
//! A program argument is a file of ELPI text, or, when no such file exists,
//! the IRI of a program stored with `tm elpi program put`. Instants are read
//! as [`crate::reasoning`] describes.

use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use tiramemsu::*;

use crate::reasoning::{self, interval, usage};
use crate::{io, open};

const PREFIXES: &[(&str, &str)] = &[("v:", "urn:tiramemsu:v:")];

/// The view and run options the ELPI reads share.
#[derive(Args)]
pub struct RunArgs {
    /// Run over memory as of a transaction number.
    #[arg(long, conflicts_with = "as_of_instant")]
    as_of: Option<u64>,
    /// Run over memory as of an instant.
    #[arg(long)]
    as_of_instant: Option<String>,
    /// Read the statements valid at this instant (default: now).
    #[arg(long)]
    valid_at: Option<String>,
    /// Resolution steps (default: the engine's budget).
    #[arg(long, default_value_t = 0)]
    fuel: u64,
    /// At most this many solutions (default: all).
    #[arg(long, default_value_t = 0)]
    limit: usize,
}

/// `tm elpi`.
#[derive(Subcommand)]
pub enum ElpiCmd {
    /// Solve a goal over memory and print each solution (writes nothing).
    Query {
        file: PathBuf,
        /// A file of ELPI text, or the IRI of a stored program.
        program: String,
        /// The goal, such as `reports (iri "urn:tiramemsu:v:alice") Y`.
        goal: String,
        #[command(flatten)]
        run: RunArgs,
    },
    /// Run a program's `tm.violation` and print every violation.
    Check {
        file: PathBuf,
        /// A file of ELPI text, or the IRI of a stored program.
        program: String,
        #[command(flatten)]
        run: RunArgs,
    },
    /// Print a program's `tm.conclude` conclusions with their support (writes nothing).
    Conclude {
        file: PathBuf,
        /// A file of ELPI text, or the IRI of a stored program.
        program: String,
        #[command(flatten)]
        run: RunArgs,
    },
    /// Write a stored program's conclusions as derived beliefs.
    Materialize {
        file: PathBuf,
        /// The program (an IRI).
        iri: String,
        /// Print what would be written and write nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Store, print and enforce ELPI programs.
    Program {
        #[command(subcommand)]
        cmd: ProgramCmd,
    },
}

/// `tm elpi program`.
#[derive(Subcommand)]
pub enum ProgramCmd {
    /// Store ELPI text as a program (superseding an older version).
    Put {
        file: PathBuf,
        /// The program (an IRI).
        iri: String,
        /// The ELPI file.
        path: PathBuf,
    },
    /// Print a stored program, its includes and whether it is enforced.
    Show {
        file: PathBuf,
        /// The program (an IRI).
        iri: String,
    },
    /// Check a stored program's `tm.violation-delta` at every commit (its full
    /// `tm.violation` runs once now).
    Enforce {
        file: PathBuf,
        /// The program (an IRI).
        iri: String,
    },
    /// Stop enforcing a stored program.
    Unenforce {
        file: PathBuf,
        /// The program (an IRI).
        iri: String,
    },
}

fn program(arg: &str) -> Result<ElpiProgram> {
    if Path::new(arg).is_file() {
        return Ok(ElpiProgram::Text(std::fs::read_to_string(arg).map_err(io)?));
    }
    if !arg.contains(':') {
        return Err(usage(format!(
            "{arg}: no such program file (or give the IRI of a stored program)"
        )));
    }
    Ok(ElpiProgram::Stored(Value::iri(arg)))
}

fn prepare<'a>(db: &'a Db, a: &RunArgs) -> Result<(View<'a>, ElpiOptions)> {
    let view = reasoning::view(db, a.as_of, a.as_of_instant.as_deref(), None)?;
    let opts = ElpiOptions {
        valid_at: a.valid_at.as_deref().map(reasoning::instant).transpose()?,
        fuel: a.fuel,
        limit: a.limit,
        ..ElpiOptions::default()
    };
    Ok((view, opts))
}

fn term(t: &ElpiTerm) -> String {
    match t {
        ElpiTerm::Value(v) => reasoning::value(PREFIXES, v),
        ElpiTerm::Other(o) => o.clone(),
    }
}

fn eids(e: &[Eid]) -> String {
    e.iter()
        .map(|e| e.n().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// `tm elpi …`.
// @tg: implements:: [[elpi#Own Calls]] {since: "2026-10-31", confidence: partial}
pub fn elpi(cmd: ElpiCmd) -> Result<()> {
    match cmd {
        ElpiCmd::Query {
            file,
            program: p,
            goal,
            run,
        } => {
            let db = open(&file)?;
            let (view, opts) = prepare(&db, &run)?;
            let r = view.elpi(&program(&p)?, &goal, &opts)?;
            for row in &r.rows {
                let cells: Vec<String> = row
                    .bindings
                    .iter()
                    .map(|(k, t)| format!("{k}={}", term(t)))
                    .collect();
                println!("{}", cells.join("  "));
            }
            eprintln!(
                "{} solution(s){}, {} steps, {} rows read",
                r.rows.len(),
                if r.exhausted { "" } else { " (limited)" },
                r.cost.fuel_used,
                r.cost.rows_read
            );
            Ok(())
        }
        ElpiCmd::Check {
            file,
            program: p,
            run,
        } => {
            let db = open(&file)?;
            let (view, opts) = prepare(&db, &run)?;
            let r = view.elpi_check(&program(&p)?, &opts)?;
            for v in &r.violations {
                println!(
                    "{} {}: {}  {}  stmts {}",
                    v.severity.name(),
                    v.name,
                    v.message,
                    interval(&v.valid),
                    eids(&v.eids)
                );
            }
            eprintln!("{} violation(s)", r.violations.len());
            Ok(())
        }
        ElpiCmd::Conclude {
            file,
            program: p,
            run,
        } => {
            let db = open(&file)?;
            let (view, opts) = prepare(&db, &run)?;
            let r = view.elpi_conclude(&program(&p)?, &opts)?;
            for c in &r.conclusions {
                println!(
                    "{} {} {}  {}  from {}",
                    reasoning::value(PREFIXES, &c.s),
                    reasoning::value(PREFIXES, &c.p),
                    reasoning::value(PREFIXES, &c.o),
                    interval(&c.valid),
                    eids(&c.support)
                );
            }
            eprintln!(
                "{} conclusion(s){}",
                r.conclusions.len(),
                if r.non_monotonic {
                    " (non-monotonic)"
                } else {
                    ""
                }
            );
            Ok(())
        }
        ElpiCmd::Materialize { file, iri, dry_run } => {
            let db = open(&file)?;
            let r = db.materialize_elpi(
                &Value::iri(iri),
                &MaterializeElpiOptions {
                    dry_run,
                    ..MaterializeElpiOptions::default()
                },
            )?;
            println!(
                "derived {}, existing {}, widened {}, reconciled {}, stale {}{}",
                r.derived.len(),
                r.existing.len(),
                r.widened.len(),
                r.reconciled.len(),
                r.stale_support.len(),
                if r.dry_run { " (dry run)" } else { "" }
            );
            if !r.needs_rerun_after_writes.is_empty() {
                println!("non-monotonic: rerun after writes");
            }
            Ok(())
        }
        ElpiCmd::Program { cmd } => programs(cmd),
    }
}

fn programs(cmd: ProgramCmd) -> Result<()> {
    match cmd {
        ProgramCmd::Put { file, iri, path } => {
            let db = open(&file)?;
            let text = std::fs::read_to_string(&path).map_err(io)?;
            let mut eid = None;
            db.transact(TxOptions::default(), |tx| {
                eid = Some(tx.store_elpi_program(Value::iri(iri.as_str()), &text)?);
                Ok(())
            })?;
            println!("{iri}: source statement {}", eid.map_or(0, |e| e.n()));
            Ok(())
        }
        ProgramCmd::Show { file, iri } => {
            let db = open(&file)?;
            let info = db.now().elpi_program_info(&Value::iri(iri.as_str()))?;
            println!(
                "% {iri}: statement {}{}{}",
                info.statement.n(),
                if info.enforced { ", enforced" } else { "" },
                if info.non_monotonic {
                    ", non-monotonic"
                } else {
                    ""
                }
            );
            for (p, e) in &info.includes {
                println!("% includes {p} (statement {})", e.n());
            }
            print!("{}", info.text);
            Ok(())
        }
        ProgramCmd::Enforce { file, iri } => set(&file, &iri, true),
        ProgramCmd::Unenforce { file, iri } => set(&file, &iri, false),
    }
}

fn set(file: &Path, iri: &str, on: bool) -> Result<()> {
    let db = open(file)?;
    db.transact(TxOptions::default(), |tx| {
        tx.enforce_elpi(Value::iri(iri), on)?;
        Ok(())
    })?;
    println!("{iri}: {}", if on { "enforced" } else { "not enforced" });
    Ok(())
}
