//! Immutable views: a time selection plus where it reads from.

use std::cell::RefCell;

use tm_core::{
    read, Error, Event, Executor, ObjectId, Result, TermReader, Triple, Value, ViewSpec,
};
use tm_exec::{CacheMode, Explain, QueryEngine, QueryResult};
use tm_ir::{IrQuery, Params};

use crate::db::Db;

/// Access to the writer executor inside a speculation.
pub(crate) trait WriterAccess {
    fn run(&self, f: &mut dyn FnMut(&mut dyn Executor) -> Result<()>) -> Result<()>;
}

impl WriterAccess for RefCell<&mut dyn Executor> {
    fn run(&self, f: &mut dyn FnMut(&mut dyn Executor) -> Result<()>) -> Result<()> {
        let mut g = self.try_borrow_mut().map_err(|_| Error::Reentrant)?;
        f(&mut **g)
    }
}

/// Where a view's reads run (design D14 `ConnSource`).
#[derive(Clone, Copy)]
enum Source<'a> {
    /// Committed state: the reader pool, or the writer when there is no pool
    /// (`ConnSource::Pool`).
    Db(&'a Db),
    /// The writer inside a speculation: sees uncommitted state, bypasses the
    /// shared caches (`ConnSource::Speculative`).
    Writer(&'a dyn WriterAccess, Option<&'a QueryEngine>),
}

/// An immutable time selection: a transaction-time selector (now, as-of, history)
/// plus an optional valid-time filter. Creating or deriving a view does no I/O;
/// every read runs in one read snapshot.
///
/// Rows read through an as-of view report `t_ret` and `ret_kind` as `None`: any
/// retraction visible there happened after the view's transaction. Use the history
/// view for real lifetimes.
#[derive(Clone, Copy)]
pub struct View<'a> {
    spec: ViewSpec,
    src: Source<'a>,
}

impl std::fmt::Debug for View<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("View").field("spec", &self.spec).finish()
    }
}

impl<'a> View<'a> {
    pub(crate) fn on_db(db: &'a Db, spec: ViewSpec) -> View<'a> {
        View {
            spec,
            src: Source::Db(db),
        }
    }

    pub(crate) fn on_writer(
        w: &'a dyn WriterAccess,
        spec: ViewSpec,
        engine: Option<&'a QueryEngine>,
    ) -> View<'a> {
        View {
            spec,
            src: Source::Writer(w, engine),
        }
    }

    /// The time selection of this view.
    pub fn spec(&self) -> ViewSpec {
        self.spec
    }

    /// A view of the same transaction time, keeping only statements valid at
    /// `epoch_ms`. The original view is unchanged.
    pub fn valid_at(self, epoch_ms: i64) -> View<'a> {
        View {
            spec: self.spec.valid_at(epoch_ms),
            ..self
        }
    }

    fn exec<R>(
        &self,
        f: impl FnOnce(&mut dyn Executor, Option<&TermReader>) -> Result<R>,
    ) -> Result<R> {
        match self.src {
            Source::Db(db) => db.read_committed(|e| f(e, Some(db.term_reader()))),
            Source::Writer(w, _) => {
                let mut f = Some(f);
                let mut out = None;
                w.run(&mut |e| {
                    let f = f.take().expect("called once");
                    out = Some(f(e, None)?);
                    Ok(())
                })?;
                Ok(out.expect("ran"))
            }
        }
    }

    /// Every statement selected by the view that matches the bound positions, in
    /// ascending eid order. Never writes.
    pub fn triples(
        &self,
        s: Option<ObjectId>,
        p: Option<ObjectId>,
        o: Option<ObjectId>,
    ) -> Result<Vec<Triple>> {
        let spec = self.spec;
        self.exec(|e, _| read::triples(e, &spec, s, p, o))
    }

    /// The values of `(s, key)`: the objects of the statements the view selects,
    /// or, only in a now view and only when there is no such statement, the volatile
    /// value.
    pub fn values(&self, s: ObjectId, key: ObjectId) -> Result<Vec<ObjectId>> {
        let spec = self.spec;
        self.exec(|e, _| read::values(e, &spec, s, key))
    }

    /// Encodes a value for a lookup. Never inserts: `None` when a dictionary value
    /// is not stored, so any pattern using it matches nothing.
    pub fn encode(&self, v: &Value) -> Result<Option<ObjectId>> {
        self.exec(|e, _| TermReader::encode(e, v))
    }

    /// Decodes an ObjectId into its value.
    pub fn decode(&self, id: ObjectId) -> Result<Value> {
        self.exec(|e, cache| match cache {
            Some(r) => r.decode(e, id, true),
            None => TermReader::new(1).decode(e, id, false),
        })
    }

    /// The view descriptor: this handle's time selection as an IR [`tm_ir::View`],
    /// which front ends use as the default View of patterns without a time clause.
    pub fn descriptor(&self) -> tm_ir::View {
        self.spec.into()
    }

    fn engine(&self) -> Result<&'a QueryEngine> {
        let e = match self.src {
            Source::Db(db) => db.engine(),
            Source::Writer(_, e) => e,
        };
        e.ok_or_else(|| Error::Unsupported {
            feature: "queries on a database opened without the query engine".to_string(),
        })
    }

    /// Executes an IR query. Each pattern is evaluated under its own View; the
    /// handle's view is not applied (use [`View::descriptor`] when lowering).
    /// Outside `Db::with` the query runs on a pooled reader in one read
    /// transaction; inside it runs on the writer and sees the speculative state.
    pub fn execute_ir(&self, q: &IrQuery, params: &Params) -> Result<QueryResult> {
        let engine = self.engine()?;
        let p = engine.prepare(q, params)?;
        let mode = match self.src {
            Source::Db(_) => CacheMode::Shared,
            Source::Writer(..) => CacheMode::Scoped,
        };
        self.exec(|e, _| engine.execute(e, mode, &p))
    }

    /// Explains an IR query: regions, SQL, parameters and `EXPLAIN QUERY PLAN`
    /// (whole and per SQL region), with the real parameters bound. Never steps
    /// the query.
    pub fn explain_ir(&self, q: &IrQuery, params: &Params) -> Result<Explain> {
        let engine = self.engine()?;
        let p = engine.prepare(q, params)?;
        self.exec(|e, _| engine.explain(e, &p))
    }

    /// Events with `t > since` visible to this view's snapshot (the whole log).
    pub fn events_since(&self, since: u64) -> Result<Vec<Event>> {
        self.exec(|e, _| read::events_since(e, since))
    }
}
