//! Immutable views: a time selection plus where it reads from.

use std::cell::RefCell;

use tm_core::{
    read, Eid, Error, Event, Executor, ObjectId, Result, TermReader, Triple, Value, ViewSpec,
};
use tm_exec::{CacheMode, Explain, PathRequest, PathRow, QueryEngine, QueryResult};
use tm_ir::{IrQuery, Params, PathMode};

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

    /// The database behind a view on committed state (`None` inside a speculation).
    pub(crate) fn db(&self) -> Option<&'a Db> {
        match self.src {
            Source::Db(db) => Some(db),
            Source::Writer(..) => None,
        }
    }

    /// The time selection of this view.
    pub fn spec(&self) -> ViewSpec {
        self.spec
    }

    /// A view of the same transaction time, keeping only statements valid at
    /// `epoch_ms` (intervals are half open, `[from, to)`). Combine it with
    /// `now`, `as_of` or `history` to ask "what did we believe then about when".
    /// The original view is unchanged.
    pub fn valid_at(self, epoch_ms: i64) -> View<'a> {
        View {
            spec: self.spec.valid_at(epoch_ms),
            ..self
        }
    }

    pub(crate) fn exec<R>(
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
    /// ascending eid order. This is the low-level lookup; use [`View::sparql`] or
    /// [`View::cypher`] for anything with joins. Never writes.
    ///
    /// # Errors
    ///
    /// `Sqlite` on a read failure.
    ///
    /// ```
    /// # use tiramemsu::*;
    /// # let dir = tempfile::tempdir().unwrap();
    /// # let db = Db::open(dir.path().join("m.db"), OpenOptions::default())?;
    /// let v = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
    /// db.transact(TxOptions::default(), |tx| {
    ///     tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?;
    ///     Ok(())
    /// })?;
    /// let alice = db.now().encode(&v("alice"))?.unwrap();
    /// let rows = db.now().triples(Some(alice), None, None)?;
    /// assert_eq!(rows.len(), 1);
    /// assert_eq!(db.now().decode(rows[0].o)?, v("acme"));
    /// # Ok::<(), Error>(())
    /// ```
    pub fn triples(
        &self,
        s: Option<ObjectId>,
        p: Option<ObjectId>,
        o: Option<ObjectId>,
    ) -> Result<Vec<Triple>> {
        let spec = self.spec;
        self.exec(|e, _| read::triples(e, &spec, s, p, o))
    }

    /// The graphs of this view: every graph with at least one visible membership of a
    /// visible statement, plus every declared graph (`CREATE GRAPH`), in id order.
    /// Time selection applies to the memberships and to the declarations.
    ///
    /// ```
    /// # use tiramemsu::*;
    /// # let dir = tempfile::tempdir().unwrap();
    /// let db = Db::open(dir.path().join("g.db"), OpenOptions::default())?;
    /// let v = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
    /// db.transact(TxOptions::default(), |tx| {
    ///     let e = tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?.eid();
    ///     tx.add_to_graph(e, v("session12"), AssertOpts::default())?;
    ///     Ok(())
    /// })?;
    /// let g = db.now().encode(&v("session12"))?.unwrap();
    /// assert_eq!(db.now().graphs()?, vec![g]);
    /// assert_eq!(db.now().graph_members(g)?.len(), 1);
    /// # Ok::<(), Error>(())
    /// ```
    // @lat: [[data-model#Named Graphs]]
    pub fn graphs(&self) -> Result<Vec<ObjectId>> {
        let spec = self.spec;
        self.exec(|e, _| read::graphs(e, &spec))
    }

    /// The eids of the statements that are members of `graph` in this view, ascending:
    /// the statement and its `sys:inGraph` membership are both visible here. A graph
    /// that no statement is in gives an empty list.
    pub fn graph_members(&self, graph: ObjectId) -> Result<Vec<Eid>> {
        let spec = self.spec;
        self.exec(|e, _| read::graph_members(e, &spec, graph))
    }

    /// The values of `(s, key)`: the objects of the statements the view selects,
    /// or, only in a now view and only when there is no such statement, the volatile
    /// value.
    pub fn values(&self, s: ObjectId, key: ObjectId) -> Result<Vec<ObjectId>> {
        let spec = self.spec;
        self.exec(|e, _| read::values(e, &spec, s, key))
    }

    /// Encodes a value for a lookup, so it can be passed to [`View::triples`],
    /// [`View::path`] or [`View::values`]. Never inserts: `None` when a dictionary value
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

    pub(crate) fn engine(&self) -> Result<&'a QueryEngine> {
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

    /// Evaluates a path from `start` under this view's transaction-time and
    /// valid-time selection, with the engine behind `tm_path`. `path` is SPARQL 1.1
    /// property-path text plus `{m,n}`; `max_hops` is a hard bound for every mode
    /// (`u32::MAX` for none). Rows come in the deterministic order of the mode; `REACH`
    /// rows carry no path value. Inside `Db::with` the path sees the speculative
    /// statements.
    ///
    /// Path steps can cross layers through the virtual hops `sys:subject` and
    /// `sys:object` of a fact id, for example `supportedBy/(sys:subject|sys:object)`.
    ///
    /// # Errors
    ///
    /// `Parse` (dialect `Path`) for a malformed path, `PathLimitExceeded` when the
    /// search passes `OpenOptions::path_max_states`, and `Unsupported` without the
    /// query engine.
    ///
    /// ```
    /// # use tiramemsu::*;
    /// # let dir = tempfile::tempdir().unwrap();
    /// # let db = Db::open(dir.path().join("m.db"), OpenOptions::default())?;
    /// let v = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
    /// db.transact(TxOptions::default(), |tx| {
    ///     tx.assert(v("a"), v("knows"), v("b"), Valid::ALWAYS)?;
    ///     tx.assert(v("b"), v("knows"), v("c"), Valid::ALWAYS)?;
    ///     Ok(())
    /// })?;
    /// let view = db.now();
    /// let a = view.encode(&v("a"))?.unwrap();
    /// let rows = view.path(a, "knows+", PathMode::Reachability, u32::MAX)?;
    /// assert_eq!(rows.len(), 2); // b and c
    /// # Ok::<(), Error>(())
    /// ```
    pub fn path(
        &self,
        start: ObjectId,
        path: &str,
        mode: PathMode,
        max_hops: u32,
    ) -> Result<Vec<PathRow>> {
        let engine = self
            .engine()?
            .path_engine()
            .ok_or_else(|| Error::Unsupported {
                feature: "View::path without the path engine".to_string(),
            })?
            .clone();
        let view = self.spec;
        self.exec(|e, _| {
            engine.eval(
                e,
                &PathRequest {
                    start,
                    path,
                    mode,
                    max_hops: Some(max_hops),
                    view,
                    end: None,
                },
            )
        })
    }

    /// Events with `t > since` visible to this view's snapshot (the whole log).
    pub fn events_since(&self, since: u64) -> Result<Vec<Event>> {
        self.exec(|e, _| read::events_since(e, since))
    }
}

// Statement dependents and fact bundles (OpenSpec change `add-fact-bundles`), kept in
// their own block.
impl View<'_> {
    /// The statements that stand on `eid` in this view: `eid` first, then,
    /// breadth-first, every visible statement whose subject or object is a statement
    /// already reached, each expansion in ascending eid order. Empty when `eid` is
    /// not visible here.
    ///
    /// On the now view this is exactly what retracting `eid` would take with it (the
    /// cascade set), read without the writer lock; on an as-of view it is what
    /// depended on `eid` then, and on the history view everything that ever did. The
    /// result is never truncated: `TxOptions::max_cascade` guards retractions only.
    ///
    /// # Errors
    ///
    /// `Sqlite` on a read failure.
    ///
    /// ```
    /// # use tiramemsu::*;
    /// # let dir = tempfile::tempdir().unwrap();
    /// # let db = Db::open(dir.path().join("m.db"), OpenOptions::default())?;
    /// let v = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
    /// let r = db.transact(TxOptions::default(), |tx| {
    ///     let job = tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?.eid();
    ///     tx.assert(job, v("source"), Value::str("chat"), Valid::ALWAYS)?;
    ///     tx.assert(v("belief9"), v("supportedBy"), job, Valid::ALWAYS)?;
    ///     Ok(())
    /// })?;
    /// let job = r.asserted[0];
    /// assert_eq!(db.now().dependents(job)?, r.asserted); // job, its source, the belief
    /// # Ok::<(), Error>(())
    /// ```
    // @lat: [[time-model#Cascade#Dependents]]
    pub fn dependents(&self, eid: Eid) -> Result<Vec<Eid>> {
        let spec = self.spec;
        self.exec(|e, _| read::dependents(e, &spec, eid))
    }
}
