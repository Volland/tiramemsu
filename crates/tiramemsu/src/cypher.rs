//! `View::cypher`, `Tx::cypher` and `Db::cypher_write`: the Cypher front end
//! (`tm-cypher`) on top of the query engine and the transaction engine.

use std::sync::Arc;

use tm_core::{Error, Executor, Result, SqlValue, TermReader, Tx, TxOptions, Value};
use tm_cypher::{CompileCtx, CypherParams, CypherResult, Rows, Runner, Vocab};
use tm_exec::{CacheMode, QueryEngine, QueryResult};
use tm_ir::{IrQuery, Params};

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

    fn writable(&self) -> bool {
        false
    }

    fn with_tx(&mut self, _f: &mut dyn FnMut(&mut Tx<'_>) -> Result<()>) -> Result<()> {
        Err(Error::unsupported("a write on a read-only view"))
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
    /// a write clause fails with `Unsupported` before anything runs.
    pub fn cypher(&self, text: &str, params: &CypherParams) -> Result<CypherResult> {
        let vocab = vocab_of(self.exec(|e, _| read_settings(e))?);
        let ctx = CompileCtx {
            vocab,
            view: self.descriptor(),
            writable: false,
        };
        let prog = tm_cypher::compile(text, params, &ctx).map_err(|e| e.into_core(text))?;
        let mut runner = ViewRunner { view: self };
        tm_cypher::exec::run(&prog, params, &mut runner).map_err(|e| e.into_core(text))
    }
}

/// Cypher inside a caller's `transact` closure.
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
        tm_cypher::exec::run(&prog, params, &mut runner).map_err(|e| e.into_core(text))
    }
}

impl Db {
    /// Runs a Cypher query that may write in exactly one transaction and returns
    /// its rows together with the transaction report.
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
}
