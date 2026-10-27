//! The engines under test: one trait, implemented here for tiramemsu on SQLite
//! (with and without LFTJ) and on redb, and in `bench/backends` for Oxigraph.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tiramemsu::rdf::{render, write_term};
use tiramemsu::*;

use crate::gen::{predicate_iri, Triple, NS, O};

/// The prefix of the error an engine returns when a query passes its time cap.
pub const TIMEOUT: &str = "Timeout";

/// Result rows: one string per cell in N-Triples syntax (empty when unbound).
pub type Rows = Vec<Vec<String>>;

/// The view a read runs in.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ViewSel {
    /// The current state.
    Now,
    /// As of a transaction number.
    AsOf(u64),
    /// Statements valid at an instant (epoch milliseconds).
    ValidAt(i64),
}

/// One write of the write workloads.
#[derive(Clone, Debug)]
pub enum WriteOp {
    /// One new statement in its own transaction.
    Insert(Triple),
    /// Replaces the value of `(s, p)` with `o` in one transaction.
    Replace(Triple),
    /// Asserts a statement with three annotations, then retracts it (the
    /// annotations cascade): two transactions.
    Cascade(Triple),
    /// Supersedes the live `(s, p, _)` statement with object `o`.
    Supersede(Triple),
}

/// An engine under test. Optional workloads return `None` when unsupported.
pub trait Engine: Send + Sync {
    /// The name in results (`sqlite`, `sqlite-lftj`, `redb`, `oxigraph`).
    fn name(&self) -> String;
    /// Bulk-loads `data` in chunks of `chunk` statements.
    fn load(&self, data: &[Triple], chunk: usize) -> Result<(), String>;
    /// Runs a SPARQL `SELECT` in `view`, stopping after `cap` with an error
    /// starting with [`TIMEOUT`].
    fn sparql(&self, q: &str, view: ViewSel, cap: Option<Duration>) -> Result<Rows, String>;
    /// Whether views other than `Now` are supported.
    fn views(&self) -> bool {
        false
    }
    /// Runs a Cypher query (with the same cap as [`Engine::sparql`]).
    fn cypher(&self, _q: &str, _cap: Option<Duration>) -> Option<Result<Rows, String>> {
        None
    }
    /// The current transaction number, for `AsOf` views.
    fn last_t(&self) -> Option<u64> {
        None
    }
    /// Runs one write.
    fn write(&self, _op: &WriteOp) -> Option<Result<(), String>> {
        None
    }
    /// Bytes on disk.
    fn size_bytes(&self) -> u64;
    /// Makes everything durable (before measuring size).
    fn sync(&self) -> Result<(), String> {
        Ok(())
    }
    /// Reclaims free space, when the engine can.
    fn compact(&self) -> Option<Result<(), String>> {
        None
    }
    /// The engine's version.
    fn version(&self) -> String;
}

fn e(x: Error) -> String {
    if let Error::DeadlineExceeded { timeout } = x {
        return format!("{TIMEOUT}: after {timeout:?}");
    }
    let b = format!("{x:?}");
    let kind: String = b.chars().take_while(|c| c.is_alphanumeric()).collect();
    format!("{kind}: {x}")
}

/// A generated IRI.
pub fn iri(local: &str) -> Value {
    Value::iri(format!("{NS}{local}"))
}

fn object(o: &O) -> Value {
    match o {
        O::Iri(x) => iri(x),
        O::Int(i) => Value::Int(*i),
        O::Str(s) => Value::str(s.clone()),
    }
}

/// Renders a stored value as the harness compares it.
pub fn cell(v: &Option<Value>) -> String {
    v.as_ref().map_or(String::new(), |v| write_term(&render(v)))
}

/// tiramemsu on one backend.
pub struct Tiramemsu {
    name: String,
    pub db: Arc<Db>,
    path: PathBuf,
}

impl Tiramemsu {
    /// Creates a fresh database file in `dir`; `lftj` turns on the SQLite route's
    /// cyclic-join operator.
    pub fn create(dir: &Path, backend: Backend, lftj: bool) -> Result<Tiramemsu, String> {
        let name = match (backend, lftj) {
            (Backend::Redb, _) => "redb".to_string(),
            (_, true) => "sqlite-lftj".to_string(),
            _ => "sqlite".to_string(),
        };
        let path = dir.join(format!("{name}.db"));
        let mut opts = OpenOptions::default();
        if lftj {
            opts.planner = PlannerOptions {
                lftj: LftjConfig {
                    enabled: true,
                    min_rows_estimate: 0,
                },
            };
        }
        let db = Db::create(&path, backend, opts).map_err(e)?;
        // Cypher labels, types and properties name the generated IRIs
        db.transact(TxOptions::default(), |tx| tx.set_vocab(NS))
            .map_err(e)?;
        Ok(Tiramemsu {
            name,
            db: Arc::new(db),
            path,
        })
    }

    fn view(&self, v: ViewSel) -> View<'_> {
        match v {
            ViewSel::Now => self.db.now(),
            ViewSel::AsOf(t) => self.db.as_of(TimeRef::Tx(t)),
            ViewSel::ValidAt(ms) => self.db.now().valid_at(ms),
        }
    }
}

impl Engine for Tiramemsu {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn load(&self, data: &[Triple], chunk: usize) -> Result<(), String> {
        let mut session = self.db.bulk_import().map_err(e)?;
        for part in data.chunks(chunk.max(1)) {
            session
                .chunk(|tx| {
                    for t in part {
                        tx.assert(
                            iri(&t.s),
                            Value::iri(predicate_iri(&t.p)),
                            object(&t.o),
                            Valid::ALWAYS,
                        )?;
                    }
                    Ok(())
                })
                .map_err(e)?;
        }
        session.finish();
        Ok(())
    }

    fn sparql(&self, q: &str, view: ViewSel, cap: Option<Duration>) -> Result<Rows, String> {
        let budget = QueryBudget {
            timeout: cap,
            ..QueryBudget::default()
        };
        match self.view(view).with_budget(&budget).sparql(q).map_err(e)? {
            SparqlResult::Solutions(s) => Ok(s
                .rows
                .iter()
                .map(|r| r.iter().map(cell).collect())
                .collect()),
            SparqlResult::Boolean(b) => Ok(vec![vec![b.to_string()]]),
            _ => Err("not a SELECT".into()),
        }
    }

    fn views(&self) -> bool {
        true
    }

    fn cypher(&self, q: &str, cap: Option<Duration>) -> Option<Result<Rows, String>> {
        let budget = QueryBudget {
            timeout: cap,
            ..QueryBudget::default()
        };
        Some(
            self.db
                .now()
                .with_budget(&budget)
                .cypher(q, &CypherParams::new())
                .map_err(e)
                .map(|r| {
                    r.rows
                        .iter()
                        .map(|row| row.iter().map(|v| v.to_json().to_string()).collect())
                        .collect()
                }),
        )
    }

    fn last_t(&self) -> Option<u64> {
        self.db.last_t().ok()
    }

    fn write(&self, op: &WriteOp) -> Option<Result<(), String>> {
        let r = match op {
            WriteOp::Insert(t) => self
                .db
                .transact(TxOptions::default(), |tx| {
                    tx.assert(
                        iri(&t.s),
                        Value::iri(predicate_iri(&t.p)),
                        object(&t.o),
                        Valid::ALWAYS,
                    )?;
                    Ok(())
                })
                .map(|_| ()),
            WriteOp::Replace(t) => self
                .db
                .transact(TxOptions::default(), |tx| {
                    let (s, p) = (
                        tx.encode(iri(&t.s))?,
                        tx.encode(Value::iri(predicate_iri(&t.p)))?,
                    );
                    for r in tx.live_rows(Some(s), Some(p), None)? {
                        if let Some(eid) = Eid::from_oid(ObjectId::from_raw(r.eid)) {
                            tx.retract(eid)?;
                        }
                    }
                    tx.assert(s, p, object(&t.o), Valid::ALWAYS)?;
                    Ok(())
                })
                .map(|_| ()),
            WriteOp::Cascade(t) => {
                let mut fact = None;
                let made = self.db.transact(TxOptions::default(), |tx| {
                    let eid = tx
                        .assert(
                            iri(&t.s),
                            Value::iri(predicate_iri(&t.p)),
                            object(&t.o),
                            Valid::ALWAYS,
                        )?
                        .eid();
                    for (k, note) in ["source", "confidence", "method"].iter().enumerate() {
                        tx.assert(eid, iri(note), Value::Int(k as i64), Valid::ALWAYS)?;
                    }
                    fact = Some(eid);
                    Ok(())
                });
                made.and_then(|_| {
                    self.db.transact(TxOptions::default(), |tx| {
                        if let Some(eid) = fact {
                            tx.retract(eid)?;
                        }
                        Ok(())
                    })
                })
                .map(|_| ())
            }
            WriteOp::Supersede(t) => self
                .db
                .transact(TxOptions::default(), |tx| {
                    let (s, p) = (
                        tx.encode(iri(&t.s))?,
                        tx.encode(Value::iri(predicate_iri(&t.p)))?,
                    );
                    let live = tx.live_rows(Some(s), Some(p), None)?;
                    match live
                        .first()
                        .and_then(|r| Eid::from_oid(ObjectId::from_raw(r.eid)))
                    {
                        Some(eid) => {
                            tx.supersede(eid, Patch::object(object(&t.o)))?;
                        }
                        None => {
                            tx.assert(s, p, object(&t.o), Valid::ALWAYS)?;
                        }
                    }
                    Ok(())
                })
                .map(|_| ()),
        };
        Some(r.map_err(e))
    }

    fn size_bytes(&self) -> u64 {
        let wal = {
            let mut p = self.path.clone().into_os_string();
            p.push("-wal");
            std::fs::metadata(PathBuf::from(p)).map_or(0, |m| m.len())
        };
        std::fs::metadata(&self.path).map_or(0, |m| m.len()) + wal
    }

    fn sync(&self) -> Result<(), String> {
        self.db.sync().map_err(e)
    }

    fn compact(&self) -> Option<Result<(), String>> {
        match self.db.backend() {
            Backend::Redb => Some(self.db.compact().map(|_| ()).map_err(e)),
            _ => None,
        }
    }

    fn version(&self) -> String {
        let base = format!("tiramemsu {}", env!("CARGO_PKG_VERSION"));
        match self.db.backend() {
            Backend::Redb => format!("{base} on redb"),
            _ => match self.db.read_sql("SELECT sqlite_version()") {
                Ok(rows) => match rows.first().and_then(|r| r.first()) {
                    Some(SqlValue::Text(v)) => format!("{base} on SQLite {v}"),
                    _ => base,
                },
                Err(_) => base,
            },
        }
    }
}
