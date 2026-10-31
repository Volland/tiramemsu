//! `tm`: the tiramemsu command line.
//!
//! A database's backend is chosen once, by `tm create --backend`; every other
//! command detects it from the file header and runs on the runner of that
//! backend (SQL on SQLite, the native runner on redb). Errors print their typed
//! name (`Parse: …`) on stderr and exit 1; usage errors exit 2.

#[cfg(feature = "datalog")]
mod datalog;
#[cfg(feature = "n3")]
mod n3;
mod output;
mod rdfio;
#[cfg(any(feature = "n3", feature = "datalog"))]
mod reasoning;
mod sg;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand, ValueEnum};
use tiramemsu::*;

#[derive(Parser)]
#[command(
    name = "tm",
    version,
    about = "Create, inspect, query and convert tiramemsu files"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Create a new database on a chosen backend.
    Create {
        file: PathBuf,
        /// Storage backend.
        #[arg(long, value_enum, default_value_t = BackendArg::Sqlite)]
        backend: BackendArg,
        /// When commits become durable.
        #[arg(long, value_enum, default_value_t = DurabilityArg::Normal)]
        durability: DurabilityArg,
    },
    /// Report the backend, format version and counts of a file.
    Info {
        file: PathBuf,
        /// Print JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Run a SPARQL or Cypher query.
    Query {
        file: PathBuf,
        /// SPARQL query text (`@path` reads it from a file).
        #[arg(long, conflicts_with = "cypher", required_unless_present = "cypher")]
        sparql: Option<String>,
        /// Cypher query text (`@path` reads it from a file).
        #[arg(long)]
        cypher: Option<String>,
        /// Output format.
        #[arg(long, value_enum, default_value_t = output::Format::Table)]
        format: output::Format,
        /// Read as of a transaction number.
        #[arg(long, conflicts_with_all = ["as_of_instant", "history"])]
        as_of: Option<u64>,
        /// Read as of an instant (epoch milliseconds).
        #[arg(long, conflicts_with = "history")]
        as_of_instant: Option<i64>,
        /// Read every statement ever asserted.
        #[arg(long)]
        history: bool,
        /// Restrict to statements valid at an instant (epoch milliseconds).
        #[arg(long)]
        valid_at: Option<i64>,
        /// Print the plan of the runner that serves the file instead of running (SPARQL).
        #[arg(long)]
        explain: bool,
        /// Stop the query after this many milliseconds.
        #[arg(long)]
        timeout: Option<u64>,
    },
    /// Load N-Triples, N-Quads, Turtle or TriG files in chunked transactions.
    Import {
        file: PathBuf,
        /// The RDF files to load (format from the extension unless --format).
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// The input format, when the extension does not tell.
        #[arg(long, value_enum)]
        format: Option<rdfio::RdfFormat>,
        /// Statements per transaction.
        #[arg(long, default_value_t = 100_000)]
        chunk: usize,
    },
    /// Write the live statements as N-Quads or N-Triples, or every statement as an
    /// RDF 1.2 statement graph (`--format sg-nt12`).
    Export {
        file: PathBuf,
        /// Output format.
        #[arg(long, value_enum, default_value_t = rdfio::ExportFormat::Nquads)]
        format: rdfio::ExportFormat,
        /// sg-nt12: assert what was live as of transaction T (default: now).
        #[arg(long, value_name = "T", conflicts_with = "history")]
        as_of: Option<u64>,
        /// sg-nt12: assert every statement.
        #[arg(long)]
        history: bool,
        /// sg-nt12: assert only statements valid at this instant (epoch ms).
        #[arg(long, value_name = "MS", allow_hyphen_values = true)]
        valid_at: Option<i64>,
        /// Output file (default: stdout).
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Convert a database losslessly to another backend.
    Convert {
        src: PathBuf,
        dst: PathBuf,
        /// Backend of the new file.
        #[arg(long, value_enum)]
        backend: BackendArg,
    },
    /// Compact a redb file (nothing to do on SQLite).
    Compact { file: PathBuf },
    /// Run N3 rules over memory and print what follows (writes nothing).
    #[cfg(feature = "n3")]
    Reason(n3::ReasonArgs),
    /// Store and print N3 rule sets.
    #[cfg(feature = "n3")]
    Rules {
        #[command(subcommand)]
        cmd: n3::RulesCmd,
    },
    /// Write the conclusions of a stored rule set as derived beliefs.
    #[cfg(feature = "n3")]
    Materialize(n3::MaterializeArgs),
    /// Run a Datalog program over memory and print a goal's rows (writes nothing).
    #[cfg(feature = "datalog")]
    Datalog(datalog::DatalogArgs),
    /// Check the constraints of a Datalog program and print the violations.
    #[cfg(feature = "datalog")]
    Check(datalog::CheckArgs),
    /// Write a stored Datalog program's conclusions as derived beliefs.
    #[cfg(feature = "datalog")]
    MaterializeProgram(datalog::MaterializeProgramArgs),
    /// Store, print and enforce Datalog programs.
    #[cfg(feature = "datalog")]
    Program {
        #[command(subcommand)]
        cmd: datalog::ProgramCmd,
    },
    /// Benchmark SQLite against redb on this machine (a reduced suite; the full
    /// comparison with Oxigraph is `bench/backends`).
    Bench {
        /// Statements in the generated WatDiv-style graph.
        #[arg(long, default_value_t = 100_000)]
        scale: u64,
        /// Timed repetitions per query.
        #[arg(long, default_value_t = 5)]
        reps: usize,
        /// Also write the results as JSON.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Where the benchmark databases go (default: a fresh temporary directory, removed after).
        #[arg(long)]
        dir: Option<PathBuf>,
    },
}

#[derive(Copy, Clone, ValueEnum)]
enum BackendArg {
    Sqlite,
    Redb,
}

impl From<BackendArg> for Backend {
    fn from(b: BackendArg) -> Backend {
        match b {
            BackendArg::Sqlite => Backend::Sqlite,
            BackendArg::Redb => Backend::Redb,
        }
    }
}

#[derive(Copy, Clone, ValueEnum)]
enum DurabilityArg {
    Normal,
    Immediate,
    None,
}

impl From<DurabilityArg> for Durability {
    fn from(d: DurabilityArg) -> Durability {
        match d {
            DurabilityArg::Normal => Durability::Normal,
            DurabilityArg::Immediate => Durability::Immediate,
            DurabilityArg::None => Durability::None,
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli.cmd) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            let e = tiramemsu_json::BindError::Db(e);
            eprintln!("{}: {e}", e.code());
            #[cfg(feature = "datalog")]
            if let tiramemsu_json::BindError::Db(e) = &e {
                datalog::print_violations(e);
            }
            ExitCode::FAILURE
        }
    }
}

fn open(file: &Path) -> Result<Db> {
    if !file.exists() {
        return Err(Error::storage(format!(
            "{}: no such file (create it with `tm create`)",
            file.display()
        )));
    }
    Db::open(file, OpenOptions::default())
}

fn run(cmd: Cmd) -> Result<()> {
    match cmd {
        // @tg: implements:: [[openspec:tm-cli#Create on a chosen backend]] {since: "2026-10-27"}
        Cmd::Create {
            file,
            backend,
            durability,
        } => {
            let db = Db::create(
                &file,
                backend.into(),
                OpenOptions {
                    durability: durability.into(),
                    ..OpenOptions::default()
                },
            )?;
            println!("created {} ({})", file.display(), db.backend().name());
            Ok(())
        }
        // @tg: implements:: [[openspec:tm-cli#Inspect a file]] {since: "2026-10-27"}
        Cmd::Info { file, json } => {
            let s = open(&file)?.summary()?;
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "backend": s.backend.name(),
                        "formatVersion": s.format_version,
                        "liveStatements": s.live_statements,
                        "statements": s.statements,
                        "terms": s.terms,
                        "transactions": s.transactions,
                        "fileSize": s.file_size,
                    })
                );
            } else {
                println!("backend          {}", s.backend.name());
                println!(
                    "format version   {}",
                    s.format_version.map_or("-".to_string(), |v| v.to_string())
                );
                println!("live statements  {}", s.live_statements);
                println!("statements       {}", s.statements);
                println!("terms            {}", s.terms);
                println!("transactions     {}", s.transactions);
                println!("file size        {} bytes", s.file_size);
            }
            Ok(())
        }
        // @tg: implements:: [[openspec:tm-cli#Query a file]] {since: "2026-10-27"}
        Cmd::Query {
            file,
            sparql,
            cypher,
            format,
            as_of,
            as_of_instant,
            history,
            valid_at,
            explain,
            timeout,
        } => {
            let db = open(&file)?;
            let view = match (as_of, as_of_instant, history) {
                (Some(t), _, _) => db.as_of(TimeRef::Tx(t)),
                (_, Some(ms), _) => db.as_of(TimeRef::Instant(ms)),
                (_, _, true) => db.history(),
                _ => db.now(),
            };
            let view = match valid_at {
                Some(ms) => view.valid_at(ms),
                None => view,
            };
            let budget = QueryBudget {
                timeout: timeout.map(Duration::from_millis),
                ..QueryBudget::default()
            };
            let view = view.with_budget(&budget);
            let mut out = std::io::stdout().lock();
            let text = match (&sparql, &cypher) {
                (Some(q), _) | (_, Some(q)) => query_text(q)?,
                _ => unreachable!("clap requires one of --sparql and --cypher"),
            };
            if explain {
                if cypher.is_some() {
                    return Err(Error::unsupported(
                        "--explain for Cypher (explain the equivalent SPARQL)",
                    ));
                }
                output::explain(&mut out, &view.explain_sparql(&text)?)?;
            } else if sparql.is_some() {
                output::sparql(&mut out, &view.sparql(&text)?, format)?;
            } else {
                output::cypher(&mut out, &view.cypher(&text, &CypherParams::new())?, format)?;
            }
            out.flush().map_err(io)
        }
        // @tg: implements:: [[openspec:tm-cli#Import and export]] {since: "2026-10-27"}
        Cmd::Import {
            file,
            inputs,
            format,
            chunk,
        } => {
            let db = open(&file)?;
            let mut total = rdfio::Loaded::default();
            for input in &inputs {
                let f = match format {
                    Some(f) => f,
                    None => rdfio::RdfFormat::from_path(input)?,
                };
                let r = rdfio::import(&db, input, f, chunk.max(1))?;
                eprintln!(
                    "{}: {} statements ({} new) in {} transactions",
                    input.display(),
                    r.statements,
                    r.new,
                    r.transactions
                );
                total.add(&r);
            }
            println!(
                "imported {} statements ({} new) in {} transactions",
                total.statements, total.new, total.transactions
            );
            Ok(())
        }
        Cmd::Export {
            file,
            format,
            as_of,
            history,
            valid_at,
            output,
        } => {
            let sel = sg::SgView {
                tx: match (as_of, history) {
                    (Some(t), _) => sg::SgTx::AsOf(t),
                    (None, true) => sg::SgTx::History,
                    (None, false) => sg::SgTx::Now,
                },
                valid_at,
            };
            let sg_flags = as_of.is_some() || history || valid_at.is_some();
            if format != rdfio::ExportFormat::SgNt12 && sg_flags {
                return Err(Error::unsupported(
                    "--as-of, --history and --valid-at need --format sg-nt12",
                ));
            }
            let db = open(&file)?;
            let write = |w: &mut dyn Write| -> Result<()> {
                match format {
                    rdfio::ExportFormat::SgNt12 => sg::export_sg(&db, sel, w).map(|_| ()),
                    f => rdfio::export(&db, f, w),
                }?;
                w.flush().map_err(io)
            };
            match output {
                Some(p) => write(&mut std::io::BufWriter::new(
                    std::fs::File::create(&p).map_err(io)?,
                )),
                None => write(&mut std::io::stdout().lock()),
            }
        }
        // @tg: implements:: [[openspec:tm-cli#Convert and compact]] {since: "2026-10-27"}
        Cmd::Convert { src, dst, backend } => {
            let r = tiramemsu::convert(&src, &dst, backend.into())?;
            println!(
                "converted {} → {}: {} transactions, {} statements, {} retractions, {} terms",
                src.display(),
                dst.display(),
                r.transactions,
                r.statements,
                r.retractions,
                r.terms
            );
            Ok(())
        }
        Cmd::Bench {
            scale,
            reps,
            out,
            dir,
        } => {
            let (dir, remove) = match dir {
                Some(d) => (d, false),
                None => (
                    std::env::temp_dir().join(format!("tm-bench-{}", std::process::id())),
                    true,
                ),
            };
            std::fs::create_dir_all(&dir).map_err(io)?;
            let cfg = tm_bench::Config::new(scale, reps);
            let command = std::env::args().collect::<Vec<_>>().join(" ");
            eprintln!(
                "benchmarking SQLite and redb at {scale} statements in {} …",
                dir.display()
            );
            let r = tm_bench::run(&cfg, &tm_bench::tiramemsu_engines, &dir, &command);
            if remove {
                let _ = std::fs::remove_dir_all(&dir);
            }
            let r = r.map_err(|e| Error::storage(format!("benchmark: {e}")))?;
            if let Some(p) = out {
                let text =
                    serde_json::to_string_pretty(&r).map_err(|e| Error::storage(e.to_string()))?;
                std::fs::write(&p, text).map_err(io)?;
            }
            println!("{}", tm_bench::report::markdown(&r));
            Ok(())
        }
        #[cfg(feature = "n3")]
        Cmd::Reason(a) => n3::reason(a),
        #[cfg(feature = "n3")]
        Cmd::Rules { cmd } => n3::rules(cmd),
        #[cfg(feature = "n3")]
        Cmd::Materialize(a) => n3::materialize(a),
        #[cfg(feature = "datalog")]
        Cmd::Datalog(a) => datalog::datalog(a),
        #[cfg(feature = "datalog")]
        Cmd::Check(a) => datalog::check(a),
        #[cfg(feature = "datalog")]
        Cmd::MaterializeProgram(a) => datalog::materialize_program(a),
        #[cfg(feature = "datalog")]
        Cmd::Program { cmd } => datalog::programs(cmd),
        Cmd::Compact { file } => {
            let db = open(&file)?;
            let before = db.summary()?.file_size;
            let done = db.compact()?;
            let after = db.summary()?.file_size;
            if done {
                println!("compacted {}: {before} → {after} bytes", file.display());
            } else {
                println!("{}: nothing to compact", file.display());
            }
            Ok(())
        }
    }
}

/// The query text: the argument, or the file it names after `@`.
fn query_text(arg: &str) -> Result<String> {
    match arg.strip_prefix('@') {
        Some(path) => std::fs::read_to_string(path).map_err(io),
        None => Ok(arg.to_string()),
    }
}

pub(crate) fn io(e: std::io::Error) -> Error {
    Error::storage(e.to_string())
}
