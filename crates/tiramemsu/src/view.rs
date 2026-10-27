//! Immutable views: a time selection plus where it reads from.

use std::cell::RefCell;

use tm_core::{
    budget, read, recall, text, vector, Bundle, Eid, Error, Event, Executor, ObjectId, RecallHit,
    RecallQuery, Result, TermReader, TextHit, TextQuery, Triple, Value, VectorQuery, VectorResult,
    ViewSpec,
};
#[cfg(feature = "exec")]
use tm_exec::{CacheMode, Explain, PathRequest, PathRow, QueryResult, TimeRespecting};
#[cfg(feature = "exec")]
use tm_ir::{IrQuery, Params, PathCompleteness, PathMode};

use tm_core::store::ReadStore;
use tm_store::dict::TermCache;

use crate::budget::QueryBudget;
use crate::db::{Db, Engine};

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
    #[cfg_attr(not(all(feature = "sparql", feature = "cypher")), allow(dead_code))]
    Writer(&'a dyn WriterAccess, Option<&'a Engine>),
    /// A key-value transaction's own state inside a speculation.
    #[cfg_attr(not(feature = "redb"), allow(dead_code))]
    Store(&'a dyn ReadStore, Option<&'a Engine>),
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
    /// The bounds of every operation run through this view ([`View::with_budget`]).
    budget: Option<&'a QueryBudget>,
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
            budget: None,
        }
    }

    pub(crate) fn on_writer(
        w: &'a dyn WriterAccess,
        spec: ViewSpec,
        engine: Option<&'a Engine>,
    ) -> View<'a> {
        View {
            spec,
            src: Source::Writer(w, engine),
            budget: None,
        }
    }

    pub(crate) fn on_store(
        store: &'a dyn ReadStore,
        spec: ViewSpec,
        engine: Option<&'a Engine>,
    ) -> View<'a> {
        View {
            spec,
            src: Source::Store(store, engine),
            budget: None,
        }
    }

    /// The database behind a view on committed state (`None` inside a speculation).
    #[cfg_attr(not(all(feature = "sparql", feature = "cypher")), allow(dead_code))]
    pub(crate) fn db(&self) -> Option<&'a Db> {
        match self.src {
            Source::Db(db) => Some(db),
            Source::Writer(..) | Source::Store(..) => None,
        }
    }

    /// Runs a read on the backend of this view: `sql` on a SQLite connection,
    /// `kv` on a storage-trait snapshot (with the term cache and whether it may
    /// be used, which it may not on uncommitted state).
    pub(crate) fn either<R>(
        &self,
        sql: impl FnOnce(&mut dyn Executor, Option<&TermReader>) -> Result<R>,
        kv: impl FnOnce(&dyn ReadStore, &TermCache, bool) -> Result<R>,
    ) -> Result<R> {
        match self.src {
            Source::Db(db) => match db.kv() {
                Some(k) => {
                    let r = k.store.read()?;
                    kv(&*r, &k.terms, true)
                }
                None => self.exec(sql),
            },
            Source::Writer(..) => self.exec(sql),
            Source::Store(store, _) => kv(store, &TermCache::new(64), false),
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

    /// The same view with every operation run through it bounded by `budget`: one
    /// call (`sparql`, `cypher`, `path`, `triples`, ...) is one operation, with its
    /// own deadline and its own row and byte budget shared by every statement it
    /// runs. See [`QueryBudget`] for the fields and the errors. The original view is
    /// unchanged; without a budget a view behaves exactly as before.
    ///
    /// ```
    /// # use tiramemsu::*;
    /// # use std::time::Duration;
    /// # let dir = tempfile::tempdir().unwrap();
    /// # let db = Db::open(dir.path().join("m.db"), OpenOptions::default())?;
    /// let v = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
    /// db.transact(TxOptions::default(), |tx| {
    ///     tx.assert(v("a"), v("p"), v("b"), Valid::ALWAYS)?;
    ///     Ok(())
    /// })?;
    /// let budget = QueryBudget {
    ///     timeout: Some(Duration::from_secs(1)),
    ///     reader_timeout: Some(Duration::from_millis(100)),
    ///     max_rows: Some(1_000),
    ///     ..Default::default()
    /// };
    /// let view = db.now().with_budget(&budget);
    /// assert_eq!(view.triples(None, None, None)?.len(), 1);
    /// # Ok::<(), Error>(())
    /// ```
    // @lat: [[query#Query Budgets]]
    pub fn with_budget<'b>(self, budget: &'b QueryBudget) -> View<'b>
    where
        'a: 'b,
    {
        View {
            spec: self.spec,
            src: self.src,
            budget: Some(budget),
        }
    }

    /// The budget of this view, if it has one.
    pub fn budget(&self) -> Option<&'a QueryBudget> {
        self.budget
    }

    /// Runs one public operation under this view's budget.
    pub(crate) fn op<R>(&self, f: impl FnOnce() -> Result<R>) -> Result<R> {
        crate::budget::run(self.budget, f)
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
            Source::Store(..) => Err(Error::MissingCapability {
                capability: "sql".to_string(),
            }),
        }
    }

    /// Every statement selected by the view that matches the bound positions, in
    /// ascending eid order. This is the low-level lookup; use `View::sparql` or
    /// `View::cypher` (features `sparql`, `cypher`) for anything with joins. Never writes.
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
        self.op(|| {
            charged(
                self.either(
                    |e, _| read::triples(e, &spec, s, p, o),
                    |r, _, _| tm_store::read::triples(r, &spec, s, p, o),
                )?,
                7,
            )
        })
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
        self.op(|| {
            charged(
                self.either(
                    |e, _| read::graphs(e, &spec),
                    |r, _, _| tm_store::read::graphs(r, &spec),
                )?,
                1,
            )
        })
    }

    /// The eids of the statements that are members of `graph` in this view, ascending:
    /// the statement and its `sys:inGraph` membership are both visible here. A graph
    /// that no statement is in gives an empty list.
    pub fn graph_members(&self, graph: ObjectId) -> Result<Vec<Eid>> {
        let spec = self.spec;
        self.op(|| {
            charged(
                self.either(
                    |e, _| read::graph_members(e, &spec, graph),
                    |r, _, _| tm_store::read::graph_members(r, &spec, graph),
                )?,
                1,
            )
        })
    }

    /// The values of `(s, key)`: the objects of the statements the view selects,
    /// or, only in a now view and only when there is no such statement, the volatile
    /// value.
    pub fn values(&self, s: ObjectId, key: ObjectId) -> Result<Vec<ObjectId>> {
        let spec = self.spec;
        self.op(|| {
            charged(
                self.either(
                    |e, _| read::values(e, &spec, s, key),
                    |r, _, _| tm_store::read::values(r, &spec, s, key),
                )?,
                1,
            )
        })
    }

    /// Encodes a value for a lookup, so it can be passed to [`View::triples`],
    /// `View::path` (feature `exec`) or [`View::values`]. Never inserts: `None` when a dictionary value
    /// is not stored, so any pattern using it matches nothing.
    pub fn encode(&self, v: &Value) -> Result<Option<ObjectId>> {
        self.op(|| {
            self.either(
                |e, _| TermReader::encode(e, v),
                |r, _, _| tm_store::dict::encode(r, v),
            )
        })
    }

    /// Decodes an ObjectId into its value.
    pub fn decode(&self, id: ObjectId) -> Result<Value> {
        self.op(|| {
            self.either(
                |e, cache| match cache {
                    Some(r) => r.decode(e, id, true),
                    None => TermReader::new(1).decode(e, id, false),
                },
                |r, terms, cache| terms.decode(r, id, cache),
            )
        })
    }

    #[cfg(feature = "exec")]
    /// The view descriptor: this handle's time selection as an IR [`tm_ir::View`],
    /// which front ends use as the default View of patterns without a time clause.
    pub fn descriptor(&self) -> tm_ir::View {
        self.spec.into()
    }

    /// Runs a prepared query on this view's backend: SQL on SQLite, the native
    /// runner on a key-value store.
    #[cfg(feature = "exec")]
    pub(crate) fn run_prepared(
        &self,
        engine: &Engine,
        p: &tm_exec::Prepared,
    ) -> Result<QueryResult> {
        let mode = match self.src {
            Source::Db(_) => CacheMode::Shared,
            Source::Writer(..) | Source::Store(..) => CacheMode::Scoped,
        };
        self.either(
            |e, _| engine.execute(e, mode, p),
            |s, _, shared| {
                let mode = if shared {
                    CacheMode::Shared
                } else {
                    CacheMode::Scoped
                };
                tm_native::execute(engine, s, mode, p)
            },
        )
    }

    #[cfg(feature = "exec")]
    pub(crate) fn engine(&self) -> Result<&'a Engine> {
        let e = match self.src {
            Source::Db(db) => db.engine(),
            Source::Writer(_, e) => e,
            Source::Store(_, e) => e,
        };
        e.ok_or_else(|| Error::Unsupported {
            feature: "queries on a database opened without the query engine".to_string(),
        })
    }

    #[cfg(feature = "exec")]
    /// Executes an IR query. Each pattern is evaluated under its own View; the
    /// handle's view is not applied (use [`View::descriptor`] when lowering).
    /// Outside `Db::with` the query runs on a pooled reader in one read
    /// transaction; inside it runs on the writer and sees the speculative state.
    pub fn execute_ir(&self, q: &IrQuery, params: &Params) -> Result<QueryResult> {
        let engine = self.engine()?;
        let p = engine.prepare(q, params)?;
        // rows and bytes are charged by the engine as it decodes
        self.op(|| self.run_prepared(engine, &p))
    }

    #[cfg(feature = "exec")]
    /// Explains an IR query: regions, SQL, parameters and `EXPLAIN QUERY PLAN`
    /// (whole and per SQL region), with the real parameters bound. Never steps
    /// the query on SQLite. On redb the native runner runs the query (under
    /// this view's budget) and reports each step it took: the pattern, its
    /// index permutation, the join kind, the estimate and the rows it saw.
    pub fn explain_ir(&self, q: &IrQuery, params: &Params) -> Result<Explain> {
        let engine = self.engine()?;
        let p = engine.prepare(q, params)?;
        self.op(|| {
            self.either(
                |e, _| engine.explain(e, &p),
                |s, _, _| tm_native::explain(engine, s, &p),
            )
        })
    }

    #[cfg(feature = "exec")]
    /// Evaluates a path from `start` under this view's transaction-time and
    /// valid-time selection, with the engine behind `tm_path`. `path` is SPARQL 1.1
    /// property-path text plus `{m,n}`; `max_hops` is a hard bound for every mode
    /// (`u32::MAX` for none). Rows come in the deterministic order of the mode; `REACH`
    /// rows carry no path value. Inside `Db::with` the path sees the speculative
    /// statements. [`View::path_with`] adds a graph filter.
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
        self.path_with(
            start,
            path,
            &PathArgs {
                mode,
                max_hops,
                ..PathArgs::default()
            },
        )
    }

    #[cfg(feature = "exec")]
    /// Evaluates a path from `start` with the options of `args`: the mode, the hop
    /// bound, an optional graph set and optional time respect. Otherwise as
    /// [`View::path`], which is the shorthand with neither.
    ///
    /// - With `args.graphs`, every statement the path traverses (the statement
    ///   stepped over, or for `sys:subject`, `sys:object` and `sys:predicate` the
    ///   statement whose part is stepped to or from) must be a member of at least one
    ///   of the graphs, the membership being visible in this view; zero-hop rows are
    ///   kept.
    /// - With `args.time_respecting`, valid time never goes backwards along a path:
    ///   from `after` (or −∞), a stored hop over a statement valid `[from, to)` needs
    ///   `to > τ` and moves τ to `max(τ, from)`; virtual hops keep τ. Each row
    ///   carries its `arrival` (in `REACH` mode the earliest over every such walk),
    ///   `None` for −∞.
    ///
    /// # Errors
    ///
    /// As [`View::path`].
    ///
    /// ```
    /// # use tiramemsu::*;
    /// # let dir = tempfile::tempdir().unwrap();
    /// # let db = Db::open(dir.path().join("m.db"), OpenOptions::default())?;
    /// let v = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
    /// db.transact(TxOptions::default(), |tx| {
    ///     let ab = tx.assert(v("a"), v("knows"), v("b"), Valid::ALWAYS)?;
    ///     tx.assert(v("b"), v("knows"), v("c"), Valid::ALWAYS)?; // in no graph
    ///     tx.add_to_graph(ab.eid(), v("session12"), AssertOpts::default())?;
    ///     Ok(())
    /// })?;
    /// let view = db.now();
    /// let a = view.encode(&v("a"))?.unwrap();
    /// let g = view.encode(&v("session12"))?.unwrap();
    /// let args = PathArgs { graphs: Some(vec![g]), ..PathArgs::default() };
    /// let rows = view.path_with(a, "knows+", &args)?;
    /// assert_eq!(rows.len(), 1); // only b: `b knows c` is in no graph
    ///
    /// // a contact chain in time order: a met b in [1, 5), b met c in [3, 9)
    /// db.transact(TxOptions::default(), |tx| {
    ///     tx.assert(v("a"), v("met"), v("b"), Valid::between(1, 5))?;
    ///     tx.assert(v("b"), v("met"), v("c"), Valid::between(3, 9))?;
    ///     Ok(())
    /// })?;
    /// let view = db.now();
    /// let args = PathArgs {
    ///     time_respecting: Some(TimeRespecting::default()),
    ///     ..PathArgs::default()
    /// };
    /// let arrivals: Vec<_> = view.path_with(a, "met+", &args)?.iter().map(|r| r.arrival).collect();
    /// assert_eq!(arrivals, [Some(1), Some(3)]); // b at 1, c at 3
    /// let late = PathArgs {
    ///     time_respecting: Some(TimeRespecting { after: Some(6) }),
    ///     ..PathArgs::default()
    /// };
    /// assert!(view.path_with(a, "met+", &late)?.is_empty()); // a met b before 6
    /// # Ok::<(), Error>(())
    /// ```
    pub fn path_with(&self, start: ObjectId, path: &str, args: &PathArgs) -> Result<Vec<PathRow>> {
        Ok(self.path_report(start, path, args)?.rows)
    }

    #[cfg(feature = "exec")]
    /// [`View::path_with`], with how completely the search was evaluated
    /// ([`PathCompleteness`]): `Exhaustive` when no state was left to expand,
    /// `StoppedAtBound` when the explicit `max_hops` stopped it with states left
    /// (complete within the bound), `StoppedAtCap` when `args.capped` applied the
    /// configured hop cap to an unbounded search and the cap stopped it (longer
    /// paths may exist). A search past `OpenOptions::path_max_states` still fails
    /// with `PathLimitExceeded` rather than returning a prefix.
    ///
    /// # Errors
    ///
    /// As [`View::path`].
    ///
    /// ```
    /// # use tiramemsu::*;
    /// # let dir = tempfile::tempdir().unwrap();
    /// # let db = Db::open(dir.path().join("m.db"), OpenOptions::default())?;
    /// let v = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
    /// db.transact(TxOptions::default(), |tx| {
    ///     tx.assert(v("a"), v("next"), v("b"), Valid::ALWAYS)?;
    ///     tx.assert(v("b"), v("next"), v("c"), Valid::ALWAYS)?;
    ///     tx.assert(v("c"), v("next"), v("a"), Valid::ALWAYS)?; // a cycle
    ///     Ok(())
    /// })?;
    /// let view = db.now();
    /// let a = view.encode(&v("a"))?.unwrap();
    /// let reach = view.path_report(a, "next+", &PathArgs::default())?;
    /// assert_eq!(reach.completeness, PathCompleteness::Exhaustive);
    /// let two = PathArgs { mode: PathMode::Trail, max_hops: 2, ..PathArgs::default() };
    /// let r = view.path_report(a, "next+", &two)?;
    /// assert_eq!(r.completeness, PathCompleteness::StoppedAtBound { max_hops: 2 });
    /// // the configured cap (`OpenOptions::path_max_hops`, 15 by default) on an
    /// // unbounded trail around the cycle
    /// let capped = PathArgs { mode: PathMode::Trail, capped: true, ..PathArgs::default() };
    /// let r = view.path_report(a, "next+", &capped)?;
    /// assert_eq!(r.completeness, PathCompleteness::Exhaustive); // a trail ends after 3 hops
    /// # Ok::<(), Error>(())
    /// ```
    pub fn path_report(&self, start: ObjectId, path: &str, args: &PathArgs) -> Result<PathReport> {
        let engine = self
            .engine()?
            .path_engine()
            .ok_or_else(|| Error::Unsupported {
                feature: "View::path without the path engine".to_string(),
            })?
            .clone();
        let view = self.spec;
        let hop_cap = args.capped && args.max_hops == u32::MAX;
        let max_hops = if hop_cap {
            engine.options().max_hops
        } else {
            args.max_hops
        };
        self.op(|| {
            let req = PathRequest {
                start,
                path,
                mode: args.mode,
                max_hops: Some(max_hops),
                hop_cap,
                view,
                end: None,
                graphs: args.graphs.clone(),
                time_respecting: args.time_respecting,
            };
            let (rows, completeness) = self.either(
                |e, _| engine.eval_report(e, &req),
                |s, _, _| engine.eval_report_on(&mut tm_native::path::StorePathSource(s), &req),
            )?;
            if budget::active() {
                budget::charge_rows(rows.len() as u64)?;
                let hops: u64 = rows
                    .iter()
                    .map(|r| r.path.as_ref().map_or(0, |p| p.hops.len() as u64))
                    .sum();
                budget::charge_bytes(rows.len() as u64 * 40 + hops * 24)?;
            }
            Ok(PathReport { rows, completeness })
        })
    }

    /// Events with `t > since` visible to this view's snapshot (the whole log).
    pub fn events_since(&self, since: u64) -> Result<Vec<Event>> {
        self.op(|| {
            charged(
                self.either(
                    |e, _| read::events_since(e, since),
                    |r, _, _| tm_store::read::events_since(r, since),
                )?,
                8,
            )
        })
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
        self.op(|| {
            charged(
                self.either(
                    |e, _| read::dependents(e, &spec, eid),
                    |r, _, _| tm_store::read::dependents(r, &spec, eid),
                )?,
                1,
            )
        })
    }

    /// The fact bundle of `root` in this view: the statements that stand on `root`
    /// ([`View::dependents`]) plus every visible statement they reference,
    /// transitively, as a portable [`Bundle`] that [`Tx::import_bundle`] writes into
    /// another database. Write it as JSON or N-Triples with [`BundleFormat`].
    ///
    /// Left out, with whatever references them: statements about transactions
    /// (transaction numbers are local to a file), engine bookkeeping such as
    /// `sys:supersedes` and `sys:confirmedBy` (memberships are kept), and statements
    /// that reference a statement outside the view. Anonymous nodes become
    /// bundle-local labels.
    ///
    /// # Errors
    ///
    /// `NotLive(root)` when `root` is not visible in this view, and `Unsupported`
    /// when `root` itself is one of the statements left out.
    ///
    /// ```
    /// # use tiramemsu::*;
    /// # let dir = tempfile::tempdir().unwrap();
    /// let a = Db::open(dir.path().join("a.db"), OpenOptions::default())?;
    /// let b = Db::open(dir.path().join("b.db"), OpenOptions::default())?;
    /// let v = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
    /// let r = a.transact(TxOptions::default(), |tx| {
    ///     let job = tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?.eid();
    ///     tx.assert(job, v("confidence"), Value::Double(0.8), Valid::ALWAYS)?;
    ///     Ok(())
    /// })?;
    /// let bundle = a.now().bundle(r.asserted[0])?;
    /// let report = b.transact(TxOptions::default(), |tx| tx.import_bundle(&bundle).map(|_| ()))?;
    /// assert_eq!(report.asserted.len(), 2);
    /// # Ok::<(), Error>(())
    /// ```
    ///
    /// [`Tx::import_bundle`]: tm_core::Tx::import_bundle
    /// [`BundleFormat`]: crate::BundleFormat
    // @lat: [[data-model#Fact Bundles]]
    pub fn bundle(&self, root: Eid) -> Result<Bundle> {
        let spec = self.spec;
        self.op(|| {
            let b = self.either(
                |e, _| read::bundle(e, &spec, root),
                |r, _, _| tm_store::read::bundle(r, &spec, root),
            )?;
            charge(b.statements.len(), 5)?;
            Ok(b)
        })
    }
}

// Text recall (OpenSpec change `add-text-retrieval`), kept in its own block.
impl View<'_> {
    /// Recalls the statements of this view whose object is a string matching
    /// `query`, with their lexical score and evidence, ranked by the documented
    /// policy ([`text::RANK_POLICY`]: lexical score, then confidence, confirmations,
    /// authors and recency, then statement eid). Plain and language-tagged strings
    /// are searchable, inline short strings included; typed literals are not.
    ///
    /// Visibility is that of [`View::triples`]: a retracted statement is absent
    /// from a now view and present in an as-of view before its retraction, and a
    /// valid-time filter applies. `query.graphs` keeps statements with a visible
    /// membership in one of the graphs. Every query runs in one read snapshot (on
    /// the writer inside [`Db::with`], where the speculative strings are found too)
    /// and is one budgeted operation. The SPARQL (`tm:textMatch`) and Cypher
    /// (`tiramemsu.text.search`) entrypoints run this same recall.
    ///
    /// # Errors
    ///
    /// `MissingCapability("fts5")` on a host without FTS5 (ordinary reads still
    /// work), `TextIndexUnavailable` when the index was never built
    /// ([`OpenOptions::text_index`](crate::OpenOptions::text_index),
    /// [`Db::rebuild_text_index`]) or is behind, `InvalidQuery` for a query
    /// without a word, and the budget errors.
    ///
    /// ```
    /// # use tiramemsu::*;
    /// # let dir = tempfile::tempdir().unwrap();
    /// let opts = OpenOptions { text_index: true, ..OpenOptions::default() };
    /// let db = Db::open(dir.path().join("m.db"), opts)?;
    /// let v = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
    /// let r = db.transact(TxOptions::default(), |tx| {
    ///     let e = tx.assert(v("alice"), v("note"), Value::str("prefers tea"), Valid::ALWAYS)?.eid();
    ///     tx.assert(e, v("confidence"), Value::Double(0.9), Valid::ALWAYS)?;
    ///     tx.assert(v("bob"), v("note"), Value::str("tea"), Valid::ALWAYS)?; // inline short string
    ///     Ok(())
    /// })?;
    /// let hits = db.now().text_search(&TextQuery::new("tea"))?;
    /// assert_eq!(hits.len(), 2);
    /// let alice = hits.iter().find(|h| h.eid == r.asserted[0]).unwrap();
    /// assert_eq!(alice.evidence.confidence, Some(0.9));
    /// let bob = hits.iter().find(|h| h.eid == r.asserted[2]).unwrap();
    /// assert_eq!(bob.evidence.confidence, None); // absent, not invented
    /// # Ok::<(), Error>(())
    /// ```
    ///
    /// [`Db::with`]: crate::Db::with
    /// [`Db::rebuild_text_index`]: crate::Db::rebuild_text_index
    // @lat: [[query#Text Recall]]
    pub fn text_search(&self, query: &TextQuery) -> Result<Vec<TextHit>> {
        let spec = self.spec;
        self.op(|| {
            self.either(
                |e, cache| match cache {
                    Some(r) => text::search(e, &spec, query, r, true),
                    None => text::search(e, &spec, query, &TermReader::new(64), false),
                },
                |r, terms, cache| tm_store::search::text_search(r, &spec, query, terms, cache),
            )
        })
    }
}

impl View<'_> {
    /// Vector recall: the statements of this view whose embedding under
    /// `query.model` is closest to `query.vector`, with their similarity and the
    /// evidence the store holds, ranked by `tiramemsu-vector-rank/1`.
    ///
    /// The search is exact: it scores every embedding of a statement this view
    /// selects (as-of, history, valid time, graphs and predicates applied first),
    /// so time travel never loses a hit to an index cut. With `prefilter` it keeps
    /// the best candidates by sign-bit distance and re-ranks them exactly, and the
    /// result says it is not exhaustive. It needs no SQLite extension.
    ///
    /// ```
    /// # use tiramemsu::*;
    /// # let dir = tempfile::tempdir().unwrap();
    /// let db = Db::open(dir.path().join("m.db"), OpenOptions::default())?;
    /// let v = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
    /// let model = v("toy-embedder");
    /// db.transact(TxOptions::default(), |tx| {
    ///     tx.define_embedding_model(model.clone(), &EmbeddingModel::new(3))?;
    ///     let paris = tx.assert(v("alice"), v("livesIn"), v("paris"), Valid::ALWAYS)?.eid();
    ///     let tea = tx.assert(v("alice"), v("likes"), v("tea"), Valid::ALWAYS)?.eid();
    ///     tx.set_embedding(paris, model.clone(), &[0.9, 0.1, 0.0])?;
    ///     tx.set_embedding(tea, model.clone(), &[0.0, 0.2, 0.9])?;
    ///     Ok(())
    /// })?;
    /// let m = db.now().encode(&model)?.unwrap();
    /// let found = db.now().vector_search(&VectorQuery::new(m, vec![1.0, 0.0, 0.0]))?;
    /// assert!(found.exact);
    /// assert_eq!(db.now().decode(found.hits[0].o)?, v("paris"));
    /// # Ok::<(), Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// `InvalidQuery` for a missing or undefined model or a query vector that does
    /// not fit it.
    // @lat: [[query#Vector Recall]]
    pub fn vector_search(&self, query: &VectorQuery) -> Result<VectorResult> {
        let spec = self.spec;
        self.op(|| {
            self.either(
                |e, cache| match cache {
                    Some(r) => vector::search(e, &spec, query, r, true),
                    None => vector::search(e, &spec, query, &TermReader::new(64), false),
                },
                |r, terms, cache| tm_store::search::vector_search(r, &spec, query, terms, cache),
            )
        })
    }

    /// The statements of this view without an embedding under `model`, in eid
    /// order, at most `limit`: the backlog of an embedding job.
    pub fn unembedded(&self, model: ObjectId, limit: Option<usize>) -> Result<Vec<Eid>> {
        let spec = self.spec;
        self.op(|| {
            self.either(
                |e, _| vector::unembedded(e, &spec, model, limit),
                |r, _, _| tm_store::search::unembedded(r, &spec, model, limit),
            )
        })
    }

    /// Hybrid recall: text and vector recall in this view, fused by reciprocal
    /// rank, optionally expanded to each hit's layers and to facts about its
    /// subject. Ranked by `tiramemsu-recall-rank/1`.
    ///
    /// # Errors
    ///
    /// `InvalidQuery` when neither channel is given, and the errors of
    /// [`View::text_search`] and [`View::vector_search`].
    // @lat: [[query#Hybrid Recall]]
    pub fn recall(&self, query: &RecallQuery) -> Result<Vec<RecallHit>> {
        let spec = self.spec;
        self.op(|| {
            self.either(
                |e, cache| match cache {
                    Some(r) => recall::recall(e, &spec, query, r, true),
                    None => recall::recall(e, &spec, query, &TermReader::new(64), false),
                },
                |r, terms, cache| tm_store::search::recall(r, &spec, query, terms, cache),
            )
        })
    }
}

/// Charges `n` rows of `cells` cells each to the operation budget, 8 bytes per cell.
fn charge(n: usize, cells: u64) -> Result<()> {
    if budget::active() {
        budget::charge_rows(n as u64)?;
        budget::charge_bytes(n as u64 * cells * 8)?;
    }
    Ok(())
}

/// [`charge`] for a list read, passing the list through.
fn charged<T>(rows: Vec<T>, cells: u64) -> Result<Vec<T>> {
    charge(rows.len(), cells)?;
    Ok(rows)
}

#[cfg(feature = "exec")]
/// The options of [`View::path_with`]. `PathArgs::default()` is `REACH` with no hop
/// bound, no graph filter and no time respect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathArgs {
    /// The path mode.
    pub mode: PathMode,
    /// A hard bound on the hop count for every mode (`u32::MAX` for none).
    pub max_hops: u32,
    /// Graph-scoped evaluation: every traversed statement must be a member of at
    /// least one of these graphs. `None` = no graph filter; `Some(vec![])` leaves
    /// only zero-hop rows.
    pub graphs: Option<Vec<ObjectId>>,
    /// Time-respecting evaluation: valid time never goes backwards along a path, and
    /// rows carry their `arrival`. `None` = ordinary evaluation.
    pub time_respecting: Option<TimeRespecting>,
    /// With no explicit bound (`max_hops == u32::MAX`), stop at the database's
    /// configured hop cap (`OpenOptions::path_max_hops`) as a Cypher `*` pattern
    /// does; [`View::path_report`] then says whether the cap cut the search.
    /// Ignored when `max_hops` is set. Off by default.
    pub capped: bool,
}

#[cfg(feature = "exec")]
impl Default for PathArgs {
    fn default() -> PathArgs {
        PathArgs {
            mode: PathMode::Reachability,
            max_hops: u32::MAX,
            graphs: None,
            time_respecting: None,
            capped: false,
        }
    }
}

#[cfg(feature = "exec")]
/// The result of [`View::path_report`]: the rows of [`View::path_with`] and how
/// completely the search was evaluated.
#[derive(Clone, Debug, PartialEq)]
pub struct PathReport {
    /// The rows, as [`View::path_with`] returns them.
    pub rows: Vec<PathRow>,
    /// `Exhaustive`, `StoppedAtBound` or `StoppedAtCap`.
    pub completeness: PathCompleteness,
}
