//! `View::cypher`, `Tx::cypher` and `Db::cypher_write`: the Cypher front end
//! (`tm-cypher`) on top of the query engine and the transaction engine.

use std::sync::Arc;

use tm_core::{Error, Executor, Result, SqlValue, TermReader, Tx, TxOptions, Value};
use tm_cypher::{CompileCtx, CypherParams, CypherResult, Rows, Runner, Vocab};
use tm_exec::{CacheMode, QueryEngine, QueryResult};
use tm_ir::{IrQuery, Params};

use crate::budget::QueryBudget;
use crate::db::Db;
use crate::sparql::read_settings;
use crate::view::View;

fn rows_of(r: &QueryResult) -> Result<Rows> {
    let columns = r.columns.iter().map(|v| v.name().to_string()).collect();
    let mut rows = Vec::with_capacity(r.rows.len());
    for row in &r.rows {
        let mut out = Vec::with_capacity(row.len());
        for cell in row {
            out.push(match cell {
                None => None,
                Some(c) => match c.as_term() {
                    Some(v) => Some(v.clone()),
                    None => return Err(Error::unsupported("a list value in an IR result")),
                },
            });
        }
        rows.push(out);
    }
    Ok(Rows { columns, rows })
}

fn volatile_entries(e: &mut dyn Executor, s: &Value) -> Result<Vec<(String, Value)>> {
    let Some(sid) = TermReader::encode(e, s)? else {
        return Ok(Vec::new());
    };
    let rows = e.rows(
        "SELECT key, value FROM volatile WHERE s = ?1 ORDER BY key",
        &[SqlValue::Integer(sid.raw())],
    )?;
    let reader = TermReader::new(16);
    let mut out = Vec::new();
    for r in rows {
        let (Some(k), Some(v)) = (r[0].as_i64(), r[1].as_i64()) else {
            continue;
        };
        let key = reader.decode(e, tm_core::ObjectId::from_raw(k), false)?;
        let val = reader.decode(e, tm_core::ObjectId::from_raw(v), false)?;
        if let Value::Iri(k) = key {
            out.push((k, val));
        }
    }
    Ok(out)
}

/// A read-only runner over a view.
struct ViewRunner<'v, 'a> {
    view: &'v View<'a>,
}

impl Runner for ViewRunner<'_, '_> {
    fn run_ir(&mut self, q: &IrQuery, params: &Params) -> Result<Rows> {
        rows_of(&self.view.execute_ir(q, params)?)
    }

    fn now_ms(&self) -> i64 {
        match self.view.db() {
            Some(db) => db.now_ms(),
            None => std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as i64),
        }
    }

    fn object_id(&mut self, v: &Value) -> Result<Option<i64>> {
        Ok(self.view.encode(v)?.map(|i| i.raw()))
    }

    fn volatile_of(&mut self, s: &Value) -> Result<Vec<(String, Value)>> {
        self.view.exec(|e, _| volatile_entries(e, s))
    }

    fn path_max_hops(&self) -> u32 {
        self.view
            .engine()
            .ok()
            .and_then(|e| e.path_engine())
            .map_or(15, |p| p.options().max_hops)
    }

    fn writable(&self) -> bool {
        false
    }

    fn with_tx(&mut self, _f: &mut dyn FnMut(&mut Tx<'_>) -> Result<()>) -> Result<()> {
        Err(Error::unsupported("a write on a read-only view"))
    }
}

/// What a read-only Cypher run touched, for saved answers: every IR query it ran
/// and whether it read volatile values.
#[derive(Default)]
pub(crate) struct CypherTrace {
    pub(crate) queries: Vec<IrQuery>,
    pub(crate) volatile: bool,
}

/// A [`ViewRunner`] that records what it runs into a [`CypherTrace`].
struct TracingRunner<'v, 'a, 't> {
    inner: ViewRunner<'v, 'a>,
    trace: &'t mut CypherTrace,
}

impl Runner for TracingRunner<'_, '_, '_> {
    fn run_ir(&mut self, q: &IrQuery, params: &Params) -> Result<Rows> {
        self.trace.queries.push(q.clone());
        self.inner.run_ir(q, params)
    }

    fn now_ms(&self) -> i64 {
        self.inner.now_ms()
    }

    fn object_id(&mut self, v: &Value) -> Result<Option<i64>> {
        self.inner.object_id(v)
    }

    fn volatile_of(&mut self, s: &Value) -> Result<Vec<(String, Value)>> {
        self.trace.volatile = true;
        self.inner.volatile_of(s)
    }

    fn path_max_hops(&self) -> u32 {
        self.inner.path_max_hops()
    }

    fn writable(&self) -> bool {
        false
    }

    fn with_tx(&mut self, f: &mut dyn FnMut(&mut Tx<'_>) -> Result<()>) -> Result<()> {
        self.inner.with_tx(f)
    }
}

/// A runner over a write transaction: reads see the transaction's own writes.
struct TxRunner<'t, 'a> {
    tx: &'t mut Tx<'a>,
    engine: Arc<QueryEngine>,
    now_ms: i64,
}

impl Runner for TxRunner<'_, '_> {
    fn run_ir(&mut self, q: &IrQuery, params: &Params) -> Result<Rows> {
        let p = self.engine.prepare(q, params)?;
        let engine = self.engine.clone();
        let r = self
            .tx
            .read_with(|e| engine.execute(e, CacheMode::Scoped, &p))?;
        rows_of(&r)
    }

    fn now_ms(&self) -> i64 {
        self.now_ms
    }

    fn object_id(&mut self, v: &Value) -> Result<Option<i64>> {
        Ok(self.tx.lookup(v)?.map(|i| i.raw()))
    }

    fn volatile_of(&mut self, s: &Value) -> Result<Vec<(String, Value)>> {
        self.tx.read_with(|e| volatile_entries(e, s))
    }

    fn path_max_hops(&self) -> u32 {
        self.engine
            .path_engine()
            .map_or(15, |p| p.options().max_hops)
    }

    fn writable(&self) -> bool {
        true
    }

    fn with_tx(&mut self, f: &mut dyn FnMut(&mut Tx<'_>) -> Result<()>) -> Result<()> {
        f(&mut *self.tx)
    }
}

fn vocab_of(settings: crate::sparql::Settings) -> Vocab {
    let (v, prefixes) = settings;
    let mut out = Vocab::default();
    if let Some(v) = v {
        out.vocab = v;
    }
    out.prefixes = prefixes;
    out
}

impl View<'_> {
    /// Runs a read-only Cypher query on this view. Every pattern is evaluated
    /// under the view's time selection unless the query overrides it. A query with
    /// a write clause fails with `Unsupported` before anything runs; use
    /// [`Db::cypher_write`] or [`TxCypher::cypher`] to write.
    ///
    /// # Errors
    ///
    /// `Parse` for invalid text, `Unsupported` for write clauses or constructs
    /// outside the supported subset, and `Eval` for runtime errors.
    ///
    /// ```
    /// # use tiramemsu::*;
    /// # let dir = tempfile::tempdir().unwrap();
    /// # let db = Db::open(dir.path().join("m.db"), OpenOptions::default())?;
    /// let none = CypherParams::default();
    /// db.cypher_write(TxOptions::default(), "CREATE (:Person {name: 'Alice'})", &none)?;
    /// let r = db.now().cypher("MATCH (p:Person) RETURN p.name AS name", &none)?;
    /// assert_eq!(r.rows, [[CypherValue::String("Alice".into())]]);
    /// // A write clause is refused on a view.
    /// assert!(db.now().cypher("CREATE (:Person)", &none).is_err());
    /// # Ok::<(), Error>(())
    /// ```
    pub fn cypher(&self, text: &str, params: &CypherParams) -> Result<CypherResult> {
        self.op(|| {
            let vocab = vocab_of(self.exec(|e, _| read_settings(e))?);
            let ctx = CompileCtx {
                vocab,
                view: self.descriptor(),
                writable: false,
            };
            let prog = tm_cypher::compile(text, params, &ctx).map_err(|e| e.into_core(text))?;
            let mut runner = ViewRunner { view: self };
            run_reporting(&prog, params, &mut runner, text)
        })
    }
}

impl View<'_> {
    /// [`View::cypher`] with the given `@vocab` and prefix table, recording into
    /// `trace` what the run touched (saved answers). Not wrapped in an operation:
    /// the caller runs it under the view's budget.
    pub(crate) fn cypher_traced(
        &self,
        text: &str,
        params: &CypherParams,
        settings: &crate::sparql::Settings,
        trace: &mut CypherTrace,
    ) -> Result<CypherResult> {
        let ctx = CompileCtx {
            vocab: vocab_of(settings.clone()),
            view: self.descriptor(),
            writable: false,
        };
        let prog = tm_cypher::compile(text, params, &ctx).map_err(|e| e.into_core(text))?;
        let mut runner = TracingRunner {
            inner: ViewRunner { view: self },
            trace,
        };
        run_reporting(&prog, params, &mut runner, text)
    }
}

/// Runs a compiled program, filling [`CypherResult::path_completeness`] from the
/// path searches it ran.
fn run_reporting(
    prog: &tm_cypher::CypherProgram,
    params: &CypherParams,
    runner: &mut dyn tm_cypher::Runner,
    text: &str,
) -> Result<CypherResult> {
    let (r, paths) = tm_exec::path::report::collect(|| tm_cypher::exec::run(prog, params, runner));
    let mut r = r.map_err(|e| e.into_core(text))?;
    r.path_completeness = paths;
    Ok(r)
}

/// Cypher inside a caller's `transact` closure, so Cypher and the Rust operations
/// share one transaction.
pub trait TxCypher {
    /// Runs a Cypher query (reads and writes) in this transaction: every clause
    /// sees the effects of the earlier ones, and a failure fails the transaction.
    fn cypher(&mut self, text: &str, params: &CypherParams) -> Result<CypherResult>;
}

impl TxCypher for Tx<'_> {
    fn cypher(&mut self, text: &str, params: &CypherParams) -> Result<CypherResult> {
        let engine = self.extension::<QueryEngine>().ok_or_else(|| {
            Error::unsupported("Cypher on a database opened without the query engine")
        })?;
        let vocab = vocab_of(self.read_with(|e| read_settings(e))?);
        let ctx = CompileCtx {
            vocab,
            view: tm_ir::View::NOW,
            writable: true,
        };
        let prog = tm_cypher::compile(text, params, &ctx).map_err(|e| e.into_core(text))?;
        let now_ms = self.instant();
        let mut runner = TxRunner {
            tx: self,
            engine,
            now_ms,
        };
        run_reporting(&prog, params, &mut runner, text)
    }
}

impl Db {
    /// Runs a Cypher query that may write in exactly one transaction and returns
    /// its rows together with the transaction report (in `CypherResult::report`).
    /// Use it for a one-shot write; use [`TxCypher::cypher`] to combine it with other
    /// operations.
    ///
    /// # Errors
    ///
    /// `Parse`, `Unsupported` and `Eval` for the query, `DeleteConnectedNode` for
    /// `DELETE` of a node with relationships, and the write errors of
    /// [`Db::transact`]. A failure commits nothing.
    ///
    /// ```
    /// # use tiramemsu::*;
    /// # let dir = tempfile::tempdir().unwrap();
    /// # let db = Db::open(dir.path().join("m.db"), OpenOptions::default())?;
    /// let out = db.cypher_write(
    ///     TxOptions::default(),
    ///     "CREATE (:Person {name: 'Alice'})-[:KNOWS]->(:Person {name: 'Bob'})",
    ///     &CypherParams::default(),
    /// )?;
    /// assert!(!out.report.unwrap().asserted.is_empty());
    /// # Ok::<(), Error>(())
    /// ```
    pub fn cypher_write(
        &self,
        opts: TxOptions,
        text: &str,
        params: &CypherParams,
    ) -> Result<CypherResult> {
        let mut result = None;
        let report = self.transact(opts, |tx| {
            result = Some(tx.cypher(text, params)?);
            Ok(())
        })?;
        let mut r = result.ok_or_else(|| Error::invalid_query("no result"))?;
        r.report = Some(report);
        Ok(r)
    }

    /// [`Db::cypher_write`] as one operation bounded by `budget`: the deadline and
    /// cancellation cover the whole query (a stopped query commits nothing), and the
    /// row and byte budgets cover every statement it runs.
    ///
    /// # Errors
    ///
    /// Those of [`Db::cypher_write`], plus `Cancelled`, `DeadlineExceeded` and
    /// `ResultLimitExceeded`; after any of them nothing is committed.
    ///
    /// ```
    /// # use tiramemsu::*;
    /// # let dir = tempfile::tempdir().unwrap();
    /// # let db = Db::open(dir.path().join("m.db"), OpenOptions::default())?;
    /// let token = CancelToken::new();
    /// token.cancel();
    /// let budget = QueryBudget { cancel: Some(token), ..Default::default() };
    /// let none = CypherParams::default();
    /// let r = db.cypher_write_budgeted(TxOptions::default(), "CREATE (:Person)", &none, &budget);
    /// assert!(matches!(r, Err(Error::Cancelled)));
    /// assert!(db.now().triples(None, None, None)?.is_empty());
    /// # Ok::<(), Error>(())
    /// ```
    pub fn cypher_write_budgeted(
        &self,
        opts: TxOptions,
        text: &str,
        params: &CypherParams,
        budget: &QueryBudget,
    ) -> Result<CypherResult> {
        crate::budget::run(Some(budget), || self.cypher_write(opts, text, params))
    }
}
