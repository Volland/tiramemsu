//! The single-writer transaction engine.
//!
//! [`Store`] owns the writer executor and runs transactions, dry runs and
//! speculation. [`Tx`] is the write handle passed to a transaction body.

mod bundle;
mod cascade;
mod config;
mod graph;
mod ops;
pub(crate) mod reserved;
pub mod schema;
mod supersede;
mod volatile;

use std::collections::{HashMap, HashSet};
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::clock::{Clock, SystemClock};
use crate::error::{Error, Position, Result};
use crate::exec::{Capabilities, Executor, Host, HostOptions, SqlValue};
use crate::id::{Eid, ObjectId, Tag, TxId, COUNTER_MAX};
use crate::report::{IdUsage, RetKind, TxOptions, TxReport, Valid};
use crate::storage::{self, meta::Counters, stats::Stats};
use crate::term::TermDict;
use crate::value::Value;
use crate::vocab;

pub use schema::PredicateSchema;

const SPEC_SAVEPOINT: &str = "spec";

/// Options of a [`Store`].
#[derive(Clone)]
pub struct StoreOptions {
    /// Clock for transaction instants.
    pub clock: Arc<dyn Clock>,
    /// Lock wait before `SQLITE_BUSY`.
    pub busy_timeout: Duration,
    /// Size of the writer's committed term cache.
    pub term_cache_capacity: usize,
    /// Run `PRAGMA optimize` every this many commits (and after bulk loads).
    pub optimize_every: u64,
}

impl Default for StoreOptions {
    fn default() -> Self {
        StoreOptions {
            clock: Arc::new(SystemClock),
            busy_timeout: Duration::from_secs(5),
            term_cache_capacity: 16_384,
            optimize_every: 1000,
        }
    }
}

impl std::fmt::Debug for StoreOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoreOptions")
            .field("busy_timeout", &self.busy_timeout)
            .field("term_cache_capacity", &self.term_cache_capacity)
            .field("optimize_every", &self.optimize_every)
            .finish()
    }
}

/// The writer side of a database: one executor, the term dictionary, the clock and
/// the statistics upkeep. Not thread-safe by itself; the facade puts it behind a mutex.
pub struct Store {
    exec: Box<dyn Executor>,
    dict: TermDict,
    clock: Arc<dyn Clock>,
    stats: Stats,
    caps: Capabilities,
    path: PathBuf,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store").field("path", &self.path).finish()
    }
}

impl Store {
    /// Opens (creating or migrating) the database at `path` through `host`.
    ///
    /// A new file gets the format-1 schema and its triggers; an older file is
    /// migrated forward in one transaction.
    ///
    /// # Errors
    ///
    /// [`Error::FormatVersion`] when the file is from a newer format,
    /// [`Error::ForeignFile`] when it is a SQLite file that is not a tiramemsu
    /// database, [`Error::Sqlite`] for host failures.
    pub fn open(host: &dyn Host, path: &Path, opts: StoreOptions) -> Result<Store> {
        let exec = storage::open(
            host,
            path,
            &HostOptions {
                busy_timeout: opts.busy_timeout,
            },
        )?;
        let mut store = Store::from_executor(exec, path, opts);
        if store.caps.fts5 {
            // index what a host without FTS5 wrote since the index was current
            let st = crate::text::state(store.exec.as_mut())?;
            if st.built() && st.stale_from != 0 {
                store.text_maintenance(|e| {
                    let st = crate::text::state(e)?;
                    crate::text::catch_up(e, st)
                })?;
            }
        }
        Ok(store)
    }

    /// Wraps an already opened and initialised writer executor.
    pub fn from_executor(exec: Box<dyn Executor>, path: &Path, opts: StoreOptions) -> Store {
        let caps = exec.capabilities();
        Store {
            exec,
            dict: TermDict::new(opts.term_cache_capacity),
            clock: opts.clock,
            stats: Stats::new(opts.optimize_every),
            caps,
            path: path.to_path_buf(),
        }
    }

    /// The capabilities declared by the host.
    pub fn capabilities(&self) -> Capabilities {
        self.caps
    }

    /// The database path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// How many times `PRAGMA optimize` ran after a commit since opening.
    pub fn optimize_runs(&self) -> u64 {
        self.stats.runs()
    }

    /// Runs a full `ANALYZE` on the writer and makes pooled readers reload the
    /// statistics. Clears [`Store::statistics_due`] on success.
    pub fn optimize(&mut self) -> Result<()> {
        storage::stats::analyze(self.exec.as_mut())?;
        self.stats.refreshed();
        Ok(())
    }

    /// Suppresses (`true`) or restores (`false`) the per-commit statistics
    /// trigger. While deferred, commits run no analysis and only mark statistics
    /// due; the next commit after deferral ends runs the upkeep unless
    /// [`Store::optimize`] ran first. Used by the facade's bulk import session.
    pub fn defer_statistics(&mut self, deferred: bool) {
        self.stats.set_deferred(deferred);
    }

    /// True while the per-commit statistics trigger is suppressed.
    pub fn statistics_deferred(&self) -> bool {
        self.stats.deferred()
    }

    /// True when deferred commits changed the data and statistics have not been
    /// refreshed since.
    pub fn statistics_due(&self) -> bool {
        self.stats.due()
    }

    /// Builds the derived text index unless it is current, and brings a stale one
    /// up to date, in one write transaction that touches no graph row. Returns
    /// true when it built the whole index.
    ///
    /// # Errors
    ///
    /// `MissingCapability("fts5")` on a host without FTS5.
    pub fn enable_text_index(&mut self) -> Result<bool> {
        self.text_maintenance(crate::text::enable)
    }

    /// Drops and rebuilds the derived text index from every statement, live and
    /// retracted, in one write transaction. Graph rows (`triple`, `term`, `tx`)
    /// are neither changed nor added. Returns the number of indexed values.
    ///
    /// # Errors
    ///
    /// `MissingCapability("fts5")` on a host without FTS5.
    pub fn rebuild_text_index(&mut self) -> Result<u64> {
        self.text_maintenance(crate::text::rebuild)
    }

    fn text_maintenance<R>(&mut self, f: fn(&mut dyn Executor) -> Result<R>) -> Result<R> {
        if !self.caps.fts5 {
            return Err(Error::MissingCapability {
                capability: "fts5".to_string(),
            });
        }
        self.derived_write(f)
    }

    /// Runs `f` in one write transaction that is not a graph transaction: no
    /// transaction number, no event, no counter change. Only for derived records
    /// (the text index, saved answers); `f` must not write `triple`, `term` or
    /// `tx`. Errors and unwinding callbacks roll everything back.
    pub fn derived_write<R>(
        &mut self,
        f: impl FnOnce(&mut dyn Executor) -> Result<R>,
    ) -> Result<R> {
        let exec = self.exec.as_mut();
        exec.begin_immediate()?;
        let res = catch_unwind(AssertUnwindSafe(|| {
            let r = f(exec)?;
            exec.commit()?;
            Ok(r)
        }));
        match res {
            Ok(Ok(r)) => Ok(r),
            Ok(Err(e)) => {
                let _ = exec.rollback();
                Err(e)
            }
            Err(panic) => {
                let _ = exec.rollback();
                resume_unwind(panic)
            }
        }
    }

    /// The writer executor (for reads outside transactions and for tests).
    pub fn executor(&mut self) -> &mut dyn Executor {
        self.exec.as_mut()
    }

    /// Runs `f` inside one read transaction on the writer. Errors and unwinding
    /// callbacks roll back the snapshot before returning or resuming the panic.
    pub fn read<R>(&mut self, f: impl FnOnce(&mut dyn Executor) -> Result<R>) -> Result<R> {
        let exec = self.exec.as_mut();
        exec.begin_read()?;
        let res = catch_unwind(AssertUnwindSafe(|| {
            let r = f(exec)?;
            exec.commit()?;
            Ok(r)
        }));
        match res {
            Ok(Ok(r)) => Ok(r),
            Ok(Err(e)) => {
                let _ = exec.rollback();
                Err(e)
            }
            Err(panic) => {
                let _ = exec.rollback();
                resume_unwind(panic)
            }
        }
    }

    /// Runs one transaction. On any error the transaction is rolled back and leaves
    /// no trace; ids seen inside a failed body may be reissued. With
    /// `opts.dry_run`, the report is returned and every effect discarded (ids burned).
    ///
    /// Use it for every write: one call is one `BEGIN IMMEDIATE` transaction with one
    /// gap-free [`TxId`], and the [`TxReport`] lists what it asserted, retracted and
    /// superseded. Calling `transact` from inside `f` is not possible (the closure
    /// holds the only writer).
    ///
    /// # Errors
    ///
    /// Any error returned by `f` or by an operation inside it (for example
    /// [`Error::UniqueViolation`]) aborts and rolls back the transaction; a host
    /// failure is reported as [`Error::Sqlite`].
    ///
    /// # Panics
    ///
    /// A callback panic is resumed with its original payload after rolling back
    /// the transaction and dictionary state.
    ///
    /// # Example
    ///
    /// ```
    /// use tm_core::{vocab::v, Error, Store, StoreOptions, TxOptions, Valid, Value};
    /// use tm_rusqlite as host;
    ///
    /// # let dir = tempfile::tempdir().unwrap();
    /// # let path = dir.path().join("db");
    /// let mut store = Store::open(&host::RusqliteHost::new(), &path, StoreOptions::default())?;
    ///
    /// // A failing body leaves nothing behind.
    /// let failed = store.transact(TxOptions::default(), |tx| {
    ///     tx.assert(Value::iri(v("a")), Value::iri(v("p")), Value::Int(1), Valid::ALWAYS)?;
    ///     Err(Error::custom("changed my mind"))
    /// });
    /// assert!(failed.is_err());
    ///
    /// let report = store.transact(TxOptions::default(), |tx| {
    ///     tx.assert(Value::iri(v("a")), Value::iri(v("p")), Value::Int(1), Valid::ALWAYS)?;
    ///     Ok(())
    /// })?;
    /// assert_eq!(report.t.0, 1); // the failed attempt used no number
    /// # Ok::<(), Error>(())
    /// ```
    pub fn transact(
        &mut self,
        opts: TxOptions,
        f: impl FnOnce(&mut Tx<'_>) -> Result<()>,
    ) -> Result<TxReport> {
        if opts.dry_run {
            return self.run_speculative(opts, f, |tx, _| Ok(tx.report()));
        }
        self.exec.begin_immediate()?;
        let res = catch_unwind(AssertUnwindSafe(|| {
            let mut tx = Tx::begin(self.exec.as_mut(), &mut self.dict, &*self.clock, opts)?;
            f(&mut tx)?;
            tx.finish_commit()
        }));
        match res {
            Ok(Ok(report)) => {
                if let Err(e) = self.exec.commit() {
                    let _ = self.exec.rollback();
                    self.dict.rollback();
                    return Err(e);
                }
                self.dict.commit();
                let inserted = report.asserted.len();
                self.stats.after_commit(self.exec.as_mut(), inserted);
                Ok(report)
            }
            Ok(Err(e)) => {
                let _ = self.exec.rollback();
                self.dict.rollback();
                Err(e)
            }
            Err(panic) => {
                let _ = self.exec.rollback();
                self.dict.rollback();
                resume_unwind(panic)
            }
        }
    }

    /// Speculation: runs `ops` inside a savepoint, then `query` on the writer so it
    /// sees the uncommitted state, then rolls everything back. Ids allocated inside
    /// are burned. `query` is not called when `ops` fails.
    ///
    /// Use it to ask "what would the graph look like if" without changing anything:
    /// `query` runs on the writer and sees the uncommitted state. Nothing is
    /// committed and no transaction number is consumed.
    /// Callback panics are resumed after rollback and burning allocated ids.
    // @lat: [[time-model#Speculative Transactions]]
    pub fn speculate<R>(
        &mut self,
        ops: impl FnOnce(&mut Tx<'_>) -> Result<()>,
        query: impl FnOnce(&mut dyn Executor) -> Result<R>,
    ) -> Result<R> {
        self.run_speculative(TxOptions::default(), ops, |tx, _| query(&mut *tx.exec))
    }

    fn run_speculative<R>(
        &mut self,
        opts: TxOptions,
        ops: impl FnOnce(&mut Tx<'_>) -> Result<()>,
        after: impl FnOnce(&mut Tx<'_>, ()) -> Result<R>,
    ) -> Result<R> {
        self.exec.begin_immediate()?;
        if let Err(e) = self.exec.savepoint(SPEC_SAVEPOINT) {
            let _ = self.exec.rollback();
            return Err(e);
        }
        let mut c0 = None;
        let mut c1 = None;
        let res = catch_unwind(AssertUnwindSafe(|| {
            let mut tx = Tx::begin(self.exec.as_mut(), &mut self.dict, &*self.clock, opts)?;
            c0 = Some(tx.c0);
            // Capture counters before propagating a callback panic: ids that the
            // caller saw during speculation must remain burned after rollback.
            let r = catch_unwind(AssertUnwindSafe(|| {
                ops(&mut tx)?;
                after(&mut tx, ())
            }));
            c1 = Some(tx.counters());
            match r {
                Ok(r) => r,
                Err(panic) => resume_unwind(panic),
            }
        }));
        // roll the savepoint back, then burn the advanced id counters
        let rolled = self
            .exec
            .rollback_to(SPEC_SAVEPOINT)
            .and_then(|_| self.exec.release(SPEC_SAVEPOINT));
        self.dict.rollback();
        let burn = |exec: &mut dyn Executor| -> Result<()> {
            if let (Some(c0), Some(c1)) = (c0, c1) {
                c1.ids_over(&c0).store(&c0, exec)?;
            }
            Ok(())
        };
        let committed = match rolled {
            Ok(()) => burn(self.exec.as_mut()).and_then(|_| self.exec.commit()),
            Err(e) => Err(e),
        };
        if let Err(e) = committed {
            let _ = self.exec.rollback();
            // burn on its own so that no id shown inside is ever reissued
            if let Some(c1) = c1 {
                let _ = self.burn_after_failure(c1);
            }
            return match res {
                Ok(r) => Err(r.err().unwrap_or(e)),
                Err(panic) => resume_unwind(panic),
            };
        }
        match res {
            Ok(r) => r,
            Err(panic) => resume_unwind(panic),
        }
    }

    fn burn_after_failure(&mut self, c1: Counters) -> Result<()> {
        self.exec.begin_immediate()?;
        let r = (|| {
            let now = Counters::load(self.exec.as_mut())?;
            let target = Counters {
                next_term: c1.next_term.max(now.next_term),
                next_node: c1.next_node.max(now.next_node),
                next_bnode: c1.next_bnode.max(now.next_bnode),
                next_stmt: c1.next_stmt.max(now.next_stmt),
                ..now
            };
            target.store(&now, self.exec.as_mut())
        })();
        match r {
            Ok(()) => self.exec.commit(),
            Err(e) => {
                let _ = self.exec.rollback();
                Err(e)
            }
        }
    }
}

/// The write handle of one transaction. Every operation runs inside the writer's
/// SQLite transaction; a failed operation fails the whole transaction.
pub struct Tx<'a> {
    exec: &'a mut dyn Executor,
    dict: &'a mut TermDict,
    t: u64,
    instant: i64,
    c0: Counters,
    c: Counters,
    opts: TxOptions,
    asserted: Vec<Eid>,
    existing: Vec<Eid>,
    retracted: Vec<(Eid, RetKind)>,
    superseded: Vec<(Eid, Eid)>,
    memberships: Vec<Eid>,
    memberships_retracted: Vec<(Eid, RetKind)>,
    /// The id of `sys:inGraph` once known (`Some(None)`: not interned yet).
    in_graph: Option<Option<ObjectId>>,
    schema_cache: HashMap<ObjectId, PredicateSchema>,
    multi_seen: HashSet<ObjectId>,
    ext: Option<std::sync::Arc<dyn std::any::Any + Send + Sync>>,
    /// How this transaction keeps the derived text index.
    text: crate::text::Upkeep,
}

impl std::fmt::Debug for Tx<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tx").field("t", &self.t).finish()
    }
}

/// Values accepted in statement positions: decoded ids or fresh values.
pub trait IntoObject {
    /// Encodes (interning on the write path) and returns the ObjectId.
    fn into_object(self, tx: &mut Tx<'_>) -> Result<ObjectId>;
}

impl IntoObject for ObjectId {
    fn into_object(self, _tx: &mut Tx<'_>) -> Result<ObjectId> {
        self.tag()?;
        self.check_origin()?;
        Ok(self)
    }
}

impl IntoObject for Eid {
    fn into_object(self, _tx: &mut Tx<'_>) -> Result<ObjectId> {
        self.oid().check_origin()?;
        Ok(self.oid())
    }
}

impl IntoObject for TxId {
    fn into_object(self, _tx: &mut Tx<'_>) -> Result<ObjectId> {
        self.oid().check_origin()?;
        Ok(self.oid())
    }
}

/// Takes the next number from an id counter, failing with `IdSpaceExhausted` once
/// the counter has handed out 2⁴⁸ − 1, so that no number reaches the origin bits.
// @lat: [[data-model#ObjectId#Origin Bits]]
fn bump(counter: &mut i64, kind: Tag) -> Result<u64> {
    let n = *counter;
    if !(0..=COUNTER_MAX as i64).contains(&n) {
        return Err(Error::IdSpaceExhausted { kind });
    }
    *counter += 1;
    Ok(n as u64)
}

impl IntoObject for Value {
    fn into_object(self, tx: &mut Tx<'_>) -> Result<ObjectId> {
        tx.dict.intern(tx.exec, &self)
    }
}

impl IntoObject for &Value {
    fn into_object(self, tx: &mut Tx<'_>) -> Result<ObjectId> {
        tx.dict.intern(tx.exec, self)
    }
}

impl<'a> Tx<'a> {
    // @lat: [[time-model#Transaction Time]]
    fn begin(
        exec: &'a mut dyn Executor,
        dict: &'a mut TermDict,
        clock: &dyn Clock,
        opts: TxOptions,
    ) -> Result<Tx<'a>> {
        let c0 = Counters::load(exec)?;
        let text = crate::text::Upkeep::begin(exec)?;
        // `t` is the next number after `last_t`, bounded like every id counter
        let t = bump(&mut (c0.last_t + 1), Tag::Tx)?;
        let instant = clock.now_ms().max(c0.last_instant + 1);
        dict.begin(c0.next_term);
        exec.execute(
            "INSERT INTO tx(t, instant) VALUES (?1, ?2)",
            &[SqlValue::Integer(t as i64), SqlValue::Integer(instant)],
        )?;
        Ok(Tx {
            exec,
            dict,
            t,
            instant,
            c0,
            c: c0,
            opts,
            asserted: Vec::new(),
            existing: Vec::new(),
            retracted: Vec::new(),
            superseded: Vec::new(),
            memberships: Vec::new(),
            memberships_retracted: Vec::new(),
            in_graph: None,
            schema_cache: HashMap::new(),
            multi_seen: HashSet::new(),
            ext: None,
            text,
        })
    }

    /// The current counters including the dictionary's next term id.
    fn counters(&self) -> Counters {
        Counters {
            next_term: self.dict.next_term(),
            ..self.c
        }
    }

    fn finish_commit(self) -> Result<TxReport> {
        let mut c = self.counters();
        c.last_t = self.t as i64;
        c.last_instant = self.instant;
        c.store(&self.c0, self.exec)?;
        Ok(self.report())
    }

    /// Attaches a host object to this transaction (the facade attaches its query
    /// engine so that front ends can run queries inside the transaction).
    pub fn set_extension(&mut self, ext: std::sync::Arc<dyn std::any::Any + Send + Sync>) {
        self.ext = Some(ext);
    }

    /// The host object attached with [`Tx::set_extension`], if it has type `T`.
    pub fn extension<T: Send + Sync + 'static>(&self) -> Option<std::sync::Arc<T>> {
        self.ext.clone()?.downcast::<T>().ok()
    }

    /// This transaction's number.
    pub fn t(&self) -> TxId {
        TxId(self.t)
    }

    /// This transaction's instant (epoch ms).
    pub fn instant(&self) -> i64 {
        self.instant
    }

    /// The ids this transaction has allocated so far: statement numbers, anonymous
    /// nodes, blank nodes and dictionary terms. After a dry run or a speculation
    /// these ids are burned (never issued again); after a commit they are in use.
    pub fn id_usage(&self) -> IdUsage {
        let c = self.counters();
        let n = |now: i64, then: i64| (now - then).max(0) as u64;
        IdUsage {
            statements: (self.c0.next_stmt..c.next_stmt)
                .map(|k| Eid::new(k as u64))
                .collect(),
            nodes: n(c.next_node, self.c0.next_node),
            blank_nodes: n(c.next_bnode, self.c0.next_bnode),
            terms: n(c.next_term, self.c0.next_term),
        }
    }

    /// The options of this transaction.
    pub fn options(&self) -> TxOptions {
        self.opts
    }

    /// The report of everything done so far.
    pub fn report(&self) -> TxReport {
        let asserted: HashSet<Eid> = self.asserted.iter().copied().collect();
        let mut seen = HashSet::new();
        let existing = self
            .existing
            .iter()
            .copied()
            .filter(|e| !asserted.contains(e) && seen.insert(*e))
            .collect();
        TxReport {
            t: TxId(self.t),
            instant: self.instant,
            asserted: self.asserted.clone(),
            existing,
            retracted: self.retracted.clone(),
            superseded: self.superseded.clone(),
            memberships: self.memberships.clone(),
            memberships_retracted: self.memberships_retracted.clone(),
        }
    }

    /// Runs a read on this transaction's connection: the closure sees the
    /// transaction's own uncommitted statements and terms. It is meant for queries
    /// (`SELECT`); the executor is the writer, so callers must not write through it.
    pub fn read_with<R>(&mut self, f: impl FnOnce(&mut dyn Executor) -> Result<R>) -> Result<R> {
        f(&mut *self.exec)
    }

    /// Encodes a value, interning it if needed.
    pub fn encode(&mut self, v: impl IntoObject) -> Result<ObjectId> {
        v.into_object(self)
    }

    /// Looks a value up without interning it.
    pub fn lookup(&mut self, v: &Value) -> Result<Option<ObjectId>> {
        self.dict.lookup_value(self.exec, v)
    }

    /// Decodes an id, seeing this transaction's own terms.
    pub fn decode(&mut self, id: ObjectId) -> Result<Value> {
        self.dict.decode(self.exec, id)
    }

    /// Allocates a fresh `NODE` id without writing any statement.
    pub fn new_node(&mut self) -> Result<ObjectId> {
        let n = bump(&mut self.c.next_node, Tag::Node)?;
        Ok(ObjectId::from_unsigned(Tag::Node, n))
    }

    /// Allocates a fresh `BNODE` id without writing any statement.
    pub fn new_bnode(&mut self) -> Result<ObjectId> {
        let n = bump(&mut self.c.next_bnode, Tag::BNode)?;
        Ok(ObjectId::from_unsigned(Tag::BNode, n))
    }

    fn alloc_eid(&mut self) -> Result<Eid> {
        Ok(Eid::new(bump(&mut self.c.next_stmt, Tag::Stmt)?))
    }

    fn sys(&mut self, iri: &str) -> Result<ObjectId> {
        self.dict.intern(self.exec, &Value::iri(iri))
    }

    fn sys_lookup(&mut self, iri: &str) -> Result<Option<ObjectId>> {
        let k = crate::term::TermKey {
            tag: Tag::Iri as u8,
            lex: iri.to_string(),
            dt: None,
            lang: None,
        };
        Ok(self
            .dict
            .lookup(self.exec, &k)?
            .map(|id| ObjectId::from_unsigned(Tag::Iri, id as u64)))
    }

    fn iri_of(&mut self, id: ObjectId) -> Result<Option<String>> {
        self.dict.iri(self.exec, id)
    }

    /// Checks that a dictionary id names an existing term of its tag.
    fn check_known(&mut self, id: ObjectId, pos: Position) -> Result<()> {
        let tag = id.tag()?;
        if tag.is_dictionary() {
            let ok = self
                .dict
                .term(self.exec, id.unsigned_payload() as i64)?
                .is_some_and(|t| t.tag == tag);
            if !ok {
                return Err(Error::InvalidTerm {
                    position: pos,
                    reason: format!("unknown term id {id:?}"),
                });
            }
        }
        Ok(())
    }

    /// Validates the kinds of `s`, `p`, `o`.
    fn check_positions(&mut self, s: ObjectId, p: ObjectId, o: ObjectId) -> Result<()> {
        let (st, pt, _) = (s.tag()?, p.tag()?, o.tag()?);
        for id in [s, p, o] {
            id.check_origin()?;
        }
        if !st.is_subject() {
            return Err(Error::InvalidTerm {
                position: Position::Subject,
                reason: format!("{} cannot be a subject", st.name()),
            });
        }
        if pt != Tag::Iri {
            return Err(Error::InvalidTerm {
                position: Position::Predicate,
                reason: format!("{} cannot be a predicate", pt.name()),
            });
        }
        self.check_known(s, Position::Subject)?;
        self.check_known(p, Position::Predicate)?;
        self.check_known(o, Position::Object)?;
        Ok(())
    }

    /// Inserts one statement row (no checks) and records it.
    fn insert_row(
        &mut self,
        eid: Eid,
        s: ObjectId,
        p: ObjectId,
        o: ObjectId,
        valid: Valid,
    ) -> Result<()> {
        self.exec.execute(
            "INSERT INTO triple(eid, s, p, o, t_add, v_from, v_to) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            &[
                SqlValue::Integer(eid.oid().raw()),
                SqlValue::Integer(s.raw()),
                SqlValue::Integer(p.raw()),
                SqlValue::Integer(o.raw()),
                SqlValue::Integer(self.t as i64),
                SqlValue::from(valid.from),
                SqlValue::from(valid.to),
            ],
        )?;
        self.asserted.push(eid);
        self.record_multi(eid, s, p, o)?;
        self.index_text(eid, o)?;
        if self.is_flag_predicate(p)? {
            self.schema_cache.clear();
        }
        Ok(())
    }

    /// Records `p` in `pred_multi` when another row with the same `(s, p, o)` exists.
    // @lat: [[storage#Multi-Eid Predicates]]
    fn record_multi(&mut self, eid: Eid, s: ObjectId, p: ObjectId, o: ObjectId) -> Result<()> {
        if self.multi_seen.contains(&p) {
            return Ok(());
        }
        let dup = self
            .exec
            .query_i64(
                "SELECT eid FROM triple WHERE s = ?1 AND p = ?2 AND o = ?3 AND eid <> ?4 LIMIT 1",
                &[
                    SqlValue::Integer(s.raw()),
                    SqlValue::Integer(p.raw()),
                    SqlValue::Integer(o.raw()),
                    SqlValue::Integer(eid.oid().raw()),
                ],
            )?
            .is_some();
        if dup {
            self.multi_seen.insert(p);
            let added = self.exec.execute(
                "INSERT INTO pred_multi(p) SELECT ?1 WHERE NOT EXISTS \
                 (SELECT 1 FROM pred_multi WHERE p = ?1)",
                &[SqlValue::Integer(p.raw())],
            )?;
            if added > 0 {
                self.c.multi_version += 1;
            }
        }
        Ok(())
    }

    /// Keeps the derived text index current for a new statement whose object is a
    /// string: indexes the value on a host with FTS5, or records on a host without
    /// it that the index lacks this statement.
    // @lat: [[storage#Text Index]]
    fn index_text(&mut self, eid: Eid, o: ObjectId) -> Result<()> {
        use crate::text::Upkeep;
        if self.text == Upkeep::Off || !crate::text::is_text(o) {
            return Ok(());
        }
        match self.text {
            Upkeep::Index => {
                let (text, lang) = match self.dict.decode(self.exec, o)? {
                    Value::Str(s) => (s, None),
                    Value::LangStr { lex, lang } => (lex, Some(lang)),
                    _ => return Ok(()),
                };
                crate::text::index_value(self.exec, o, &text, lang.as_deref())
            }
            Upkeep::Defer { marked: false } => {
                self.exec.execute(
                    "UPDATE meta SET value = ?1 WHERE key = 'text_stale' AND value = 0",
                    &[SqlValue::Integer(eid.oid().raw())],
                )?;
                self.text = Upkeep::Defer { marked: true };
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Sets `t_ret`/`ret_kind` on a live row. Returns false if it was not live.
    fn retract_row(&mut self, eid: Eid, kind: RetKind) -> Result<bool> {
        let n = self.exec.execute(
            "UPDATE triple SET t_ret = ?1, ret_kind = ?2 WHERE eid = ?3 AND t_ret IS NULL",
            &[
                SqlValue::Integer(self.t as i64),
                SqlValue::Integer(kind as i64),
                SqlValue::Integer(eid.oid().raw()),
            ],
        )?;
        if n > 0 {
            if self.is_membership(eid)? {
                self.memberships_retracted.push((eid, kind));
            } else {
                self.retracted.push((eid, kind));
            }
            self.schema_cache.clear();
        }
        Ok(n > 0)
    }

    /// The id of `sys:inGraph`, `None` while it has never been interned.
    fn in_graph_id(&mut self) -> Result<Option<ObjectId>> {
        if let Some(Some(id)) = self.in_graph {
            return Ok(Some(id));
        }
        let id = self.sys_lookup(vocab::SYS_IN_GRAPH)?;
        self.in_graph = Some(id);
        Ok(id)
    }

    /// True when statement `eid` is a graph membership (its predicate is `sys:inGraph`).
    fn is_membership(&mut self, eid: Eid) -> Result<bool> {
        let Some(ig) = self.in_graph_id()? else {
            return Ok(false);
        };
        Ok(self
            .exec
            .query_i64(
                "SELECT p FROM triple WHERE eid = ?1",
                &[SqlValue::Integer(eid.oid().raw())],
            )?
            .is_some_and(|p| p == ig.raw()))
    }

    fn is_flag_predicate(&mut self, p: ObjectId) -> Result<bool> {
        Ok(self
            .iri_of(p)?
            .is_some_and(|i| vocab::SCHEMA_FLAGS.contains(&i.as_str())))
    }
}
