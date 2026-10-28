//! The single-writer transaction engine over the storage traits.
//!
//! A port of `tm_core::engine`: every check, error, ordering and report is the
//! same, and each SQL statement of the original is a trait call plus, at most,
//! an in-memory filter. [`Engine`] owns the backend handle, the term dictionary
//! and the clock; [`Tx`] is the write handle passed to a transaction body.

mod bundle;
mod cascade;
mod config;
mod derive;
mod embedding;
mod graph;
mod ops;
mod schema;
mod supersede;
mod volatile;

use std::collections::{HashMap, HashSet};
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::sync::Arc;

use tm_core::storage::meta::Counters;
use tm_core::{
    Clock, Eid, Error, IdUsage, ObjectId, Position, Result, RetKind, SystemClock, Tag, TxId,
    TxOptions, TxReport, Valid, Value,
};

use crate::dict::TermDict;
use crate::store::{Backend, ReadStore, StmtRow, WriteStore};
use crate::text;

pub use schema::PredicateSchema;

/// The logical format a trait backend stores: the same rows as SQLite format 5.
pub const LOGICAL_FORMAT: i64 = tm_core::storage::FORMAT_VERSION;

/// The `meta` rows of a fresh store: SQLite's format-1 rows with the logical
/// format, plus the text-index rows of format 2.
const INITIAL: [(&str, i64); 10] = [
    ("format_version", LOGICAL_FORMAT),
    ("next_term", 1),
    ("next_node", 1),
    ("next_bnode", 1),
    ("next_stmt", 1),
    ("last_t", 0),
    ("last_instant", 0),
    ("multi_version", 0),
    ("text_index", 0),
    ("text_stale", 0),
];

const COUNTER_KEYS: [&str; 7] = [
    "next_term",
    "next_node",
    "next_bnode",
    "next_stmt",
    "last_t",
    "last_instant",
    "multi_version",
];

/// Reads the engine counters.
pub fn load_counters(store: &dyn ReadStore) -> Result<Counters> {
    let mut v = [0i64; 7];
    for (slot, k) in v.iter_mut().zip(COUNTER_KEYS) {
        *slot = store
            .meta(k)?
            .ok_or_else(|| Error::storage("meta is missing counters"))?;
    }
    Ok(Counters {
        next_term: v[0],
        next_node: v[1],
        next_bnode: v[2],
        next_stmt: v[3],
        last_t: v[4],
        last_instant: v[5],
        multi_version: v[6],
    })
}

/// Writes every counter of `c` that differs from `before`.
pub fn store_counters(c: &Counters, before: &Counters, store: &mut dyn WriteStore) -> Result<()> {
    let now = [
        c.next_term,
        c.next_node,
        c.next_bnode,
        c.next_stmt,
        c.last_t,
        c.last_instant,
        c.multi_version,
    ];
    let old = [
        before.next_term,
        before.next_node,
        before.next_bnode,
        before.next_stmt,
        before.last_t,
        before.last_instant,
        before.multi_version,
    ];
    for ((k, v), o) in COUNTER_KEYS.into_iter().zip(now).zip(old) {
        if v != o {
            store.set_meta(k, v)?;
        }
    }
    Ok(())
}

/// Options of an [`Engine`].
#[derive(Clone)]
pub struct EngineOptions {
    /// Clock for transaction instants.
    pub clock: Arc<dyn Clock>,
    /// Size of the writer's committed term cache.
    pub term_cache_capacity: usize,
}

impl Default for EngineOptions {
    fn default() -> Self {
        EngineOptions {
            clock: Arc::new(SystemClock),
            term_cache_capacity: 16_384,
        }
    }
}

impl std::fmt::Debug for EngineOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineOptions")
            .field("term_cache_capacity", &self.term_cache_capacity)
            .finish()
    }
}

/// The writer side of a database on the storage traits. Not thread-safe by
/// itself; a facade puts it behind a mutex, as it does `tm_core::Store`.
// @tg: implements:: [[openspec:storage-trait#One engine for every backend]] {since: "2026-10-27"}
pub struct Engine {
    backend: Arc<dyn Backend>,
    dict: TermDict,
    clock: Arc<dyn Clock>,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine")
            .field("backend", &self.backend.name())
            .finish()
    }
}

/// Runs `f` on a fresh write transaction, committing on success and aborting on
/// an error or a panic (which is resumed after the abort).
fn in_write<R>(
    backend: &dyn Backend,
    f: impl FnOnce(&mut dyn WriteStore) -> Result<R>,
) -> Result<R> {
    let mut w = backend.write()?;
    let res = catch_unwind(AssertUnwindSafe(|| f(&mut *w)));
    match res {
        Ok(Ok(r)) => {
            w.commit()?;
            Ok(r)
        }
        Ok(Err(e)) => {
            let _ = w.abort();
            Err(e)
        }
        Err(panic) => {
            let _ = w.abort();
            resume_unwind(panic)
        }
    }
}

impl Engine {
    /// Opens the store behind `backend`, initialising an empty one.
    ///
    /// # Errors
    ///
    /// [`Error::FormatVersion`] when the store holds a different logical format.
    pub fn open(backend: Arc<dyn Backend>, opts: EngineOptions) -> Result<Engine> {
        let format = backend.read()?.meta("format_version")?;
        match format {
            None => in_write(&*backend, |w| {
                for (k, v) in INITIAL {
                    w.set_meta(k, v)?;
                }
                Ok(())
            })?,
            Some(LOGICAL_FORMAT) => {}
            Some(found) => {
                return Err(Error::FormatVersion {
                    found,
                    supported: LOGICAL_FORMAT,
                })
            }
        }
        Ok(Engine {
            backend,
            dict: TermDict::new(opts.term_cache_capacity),
            clock: opts.clock,
        })
    }

    /// The backend.
    pub fn backend(&self) -> &Arc<dyn Backend> {
        &self.backend
    }

    /// Number of committed terms in the writer's cache.
    pub fn term_cache_len(&self) -> usize {
        self.dict.cached()
    }

    /// A reader on the last committed state.
    pub fn reader(&self) -> Result<Box<dyn ReadStore + Send>> {
        self.backend.read()
    }

    /// Runs `f` on a reader of the last committed state.
    pub fn read<R>(&self, f: impl FnOnce(&dyn ReadStore) -> Result<R>) -> Result<R> {
        let r = self.backend.read()?;
        f(&*r)
    }

    /// Runs `f` in one write transaction that is not a graph transaction: no
    /// transaction number, no event, no counter change. Only for derived records
    /// (the text index, saved answers). Errors and panics roll it back.
    pub fn derived_write<R>(
        &mut self,
        f: impl FnOnce(&mut dyn WriteStore) -> Result<R>,
    ) -> Result<R> {
        in_write(&*self.backend, f)
    }

    /// Builds the text index unless it is built; returns true when it built it.
    pub fn enable_text_index(&mut self) -> Result<bool> {
        self.derived_write(text::enable)
    }

    /// Drops and rebuilds the text index; returns the number of indexed values.
    pub fn rebuild_text_index(&mut self) -> Result<u64> {
        self.derived_write(text::rebuild)
    }

    /// Runs one transaction (see `tm_core::Store::transact`). On any error the
    /// transaction is aborted and leaves no trace; with `opts.dry_run` the report
    /// is returned and every effect discarded, with allocated ids burned.
    pub fn transact(
        &mut self,
        opts: TxOptions,
        f: impl FnOnce(&mut Tx<'_>) -> Result<()>,
    ) -> Result<TxReport> {
        if opts.dry_run {
            return self.run_speculative(opts, f, |tx| Ok(tx.report()));
        }
        let mut w = self.backend.write()?;
        let dict = &mut self.dict;
        let clock = &*self.clock;
        let res = catch_unwind(AssertUnwindSafe(|| {
            let mut tx = Tx::begin(&mut *w, dict, clock, opts)?;
            f(&mut tx)?;
            tx.finish_commit()
        }));
        match res {
            Ok(Ok(report)) => {
                if let Err(e) = w.commit() {
                    self.dict.rollback();
                    return Err(e);
                }
                self.dict.commit();
                Ok(report)
            }
            Ok(Err(e)) => {
                let _ = w.abort();
                self.dict.rollback();
                Err(e)
            }
            Err(panic) => {
                let _ = w.abort();
                self.dict.rollback();
                resume_unwind(panic)
            }
        }
    }

    /// Speculation: runs `ops`, then `query` on the transaction's own state, then
    /// aborts. Ids allocated inside are burned; `query` is not called when `ops`
    /// fails.
    pub fn speculate<R>(
        &mut self,
        ops: impl FnOnce(&mut Tx<'_>) -> Result<()>,
        query: impl FnOnce(&dyn ReadStore) -> Result<R>,
    ) -> Result<R> {
        self.run_speculative(TxOptions::default(), ops, |tx| query(&*tx.store))
    }

    /// Like [`Engine::speculate`], with the write handle passed to `query`
    /// (front ends run their queries through it).
    pub fn speculate_with<R>(
        &mut self,
        ops: impl FnOnce(&mut Tx<'_>) -> Result<()>,
        query: impl FnOnce(&mut Tx<'_>) -> Result<R>,
    ) -> Result<R> {
        self.run_speculative(TxOptions::default(), ops, query)
    }

    // @tg: implements:: [[openspec:storage-trait#Speculation without nested savepoints]] {since: "2026-10-27"}
    fn run_speculative<R>(
        &mut self,
        opts: TxOptions,
        ops: impl FnOnce(&mut Tx<'_>) -> Result<()>,
        after: impl FnOnce(&mut Tx<'_>) -> Result<R>,
    ) -> Result<R> {
        let mut w = self.backend.write()?;
        let mut c0 = None;
        let mut c1 = None;
        let dict = &mut self.dict;
        let clock = &*self.clock;
        let res = catch_unwind(AssertUnwindSafe(|| {
            let mut tx = Tx::begin(&mut *w, dict, clock, opts)?;
            c0 = Some(tx.c0);
            // capture counters before propagating a callback panic: ids the caller
            // saw during speculation must stay burned after the abort
            let r = catch_unwind(AssertUnwindSafe(|| {
                ops(&mut tx)?;
                after(&mut tx)
            }));
            c1 = Some(tx.counters());
            match r {
                Ok(r) => r,
                Err(panic) => resume_unwind(panic),
            }
        }));
        // abort everything, then burn the advanced id counters in a new transaction
        let _ = w.abort();
        self.dict.rollback();
        let burned = match (c0, c1) {
            (Some(_), Some(c1)) => in_write(&*self.backend, |w| {
                let now = load_counters(&*w)?;
                store_counters(&max_ids(&c1, &now), &now, w)
            }),
            _ => Ok(()),
        };
        if let Err(e) = burned {
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
}

/// `now` with every id counter raised to at least `c1`'s.
fn max_ids(c1: &Counters, now: &Counters) -> Counters {
    Counters {
        next_term: c1.next_term.max(now.next_term),
        next_node: c1.next_node.max(now.next_node),
        next_bnode: c1.next_bnode.max(now.next_bnode),
        next_stmt: c1.next_stmt.max(now.next_stmt),
        ..*now
    }
}

/// Takes the next number from an id counter, failing with `IdSpaceExhausted` once
/// the counter has handed out 2⁴⁸ − 1.
fn bump(counter: &mut i64, kind: Tag) -> Result<u64> {
    let n = *counter;
    if !(0..=tm_core::id::COUNTER_MAX as i64).contains(&n) {
        return Err(Error::IdSpaceExhausted { kind });
    }
    *counter += 1;
    Ok(n as u64)
}

/// The write handle of one transaction. Every operation runs inside the
/// backend's write transaction; a failed operation fails the whole transaction.
pub struct Tx<'a> {
    store: &'a mut dyn WriteStore,
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
    in_graph: Option<Option<ObjectId>>,
    schema_cache: HashMap<ObjectId, PredicateSchema>,
    multi_seen: HashSet<ObjectId>,
    ext: Option<Arc<dyn std::any::Any + Send + Sync>>,
    text: text::Upkeep,
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

impl IntoObject for Value {
    fn into_object(self, tx: &mut Tx<'_>) -> Result<ObjectId> {
        tx.dict.intern(tx.store, &self)
    }
}

impl IntoObject for &Value {
    fn into_object(self, tx: &mut Tx<'_>) -> Result<ObjectId> {
        tx.dict.intern(tx.store, self)
    }
}

impl<'a> Tx<'a> {
    fn begin(
        store: &'a mut dyn WriteStore,
        dict: &'a mut TermDict,
        clock: &dyn Clock,
        opts: TxOptions,
    ) -> Result<Tx<'a>> {
        let c0 = load_counters(&*store)?;
        let text = text::Upkeep::begin(&*store)?;
        let t = bump(&mut (c0.last_t + 1), Tag::Tx)?;
        let instant = clock.now_ms().max(c0.last_instant + 1);
        dict.begin(c0.next_term);
        store.insert_tx(t as i64, instant)?;
        Ok(Tx {
            store,
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
        store_counters(&c, &self.c0, self.store)?;
        Ok(self.report())
    }

    /// Attaches a host object to this transaction.
    pub fn set_extension(&mut self, ext: Arc<dyn std::any::Any + Send + Sync>) {
        self.ext = Some(ext);
    }

    /// The host object attached with [`Tx::set_extension`], if it has type `T`.
    pub fn extension<T: Send + Sync + 'static>(&self) -> Option<Arc<T>> {
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

    /// The ids this transaction has allocated so far.
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

    /// The transaction's own state, uncommitted writes included, for reads.
    pub fn store(&self) -> &dyn ReadStore {
        &*self.store
    }

    /// Runs a read on the transaction's own state.
    pub fn read_with<R>(&mut self, f: impl FnOnce(&dyn ReadStore) -> Result<R>) -> Result<R> {
        f(&*self.store)
    }

    /// Encodes a value, interning it if needed.
    pub fn encode(&mut self, v: impl IntoObject) -> Result<ObjectId> {
        v.into_object(self)
    }

    /// Looks a value up without interning it.
    pub fn lookup(&mut self, v: &Value) -> Result<Option<ObjectId>> {
        self.dict.lookup_value(&*self.store, v)
    }

    /// Decodes an id, seeing this transaction's own terms.
    pub fn decode(&mut self, id: ObjectId) -> Result<Value> {
        self.dict.decode(&*self.store, id)
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
        self.dict.intern(self.store, &Value::iri(iri))
    }

    fn sys_lookup(&mut self, iri: &str) -> Result<Option<ObjectId>> {
        let k = tm_core::term::TermKey {
            tag: Tag::Iri as u8,
            lex: iri.to_string(),
            dt: None,
            lang: None,
        };
        Ok(self
            .dict
            .lookup(&*self.store, &k)?
            .map(|id| ObjectId::from_unsigned(Tag::Iri, id as u64)))
    }

    fn iri_of(&mut self, id: ObjectId) -> Result<Option<String>> {
        self.dict.iri(&*self.store, id)
    }

    /// Checks that a dictionary id names an existing term of its tag.
    fn check_known(&mut self, id: ObjectId, pos: Position) -> Result<()> {
        let tag = id.tag()?;
        if tag.is_dictionary() {
            let ok = self
                .dict
                .term(&*self.store, id.unsigned_payload() as i64)?
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
        self.store.insert_statement(&StmtRow {
            eid: eid.oid().raw(),
            s: s.raw(),
            p: p.raw(),
            o: o.raw(),
            t_add: self.t as i64,
            t_ret: None,
            v_from: valid.from,
            v_to: valid.to,
            ret_kind: None,
        })?;
        self.asserted.push(eid);
        self.record_multi(eid, s, p, o)?;
        self.index_text(o)?;
        if self.is_flag_predicate(p)? {
            self.schema_cache.clear();
        }
        Ok(())
    }

    /// Records `p` in `pred_multi` when another row with the same `(s, p, o)` exists.
    fn record_multi(&mut self, eid: Eid, s: ObjectId, p: ObjectId, o: ObjectId) -> Result<()> {
        if self.multi_seen.contains(&p) {
            return Ok(());
        }
        let me = eid.oid().raw();
        let mut dup = false;
        for r in self.store.scan(
            crate::Pattern::new(Some(s.raw()), Some(p.raw()), Some(o.raw())),
            crate::Scope::All,
        )? {
            if r?.eid != me {
                dup = true;
                break;
            }
        }
        if dup {
            self.multi_seen.insert(p);
            if self.store.add_pred_multi(p.raw())? {
                self.c.multi_version += 1;
            }
        }
        Ok(())
    }

    /// Keeps the text index current for a new statement whose object is a string.
    fn index_text(&mut self, o: ObjectId) -> Result<()> {
        if self.text == text::Upkeep::Off || !tm_core::text::is_text(o) {
            return Ok(());
        }
        let (body, lang) = match self.dict.decode(&*self.store, o)? {
            Value::Str(s) => (s, None),
            Value::LangStr { lex, lang } => (lex, Some(lang)),
            _ => return Ok(()),
        };
        text::index_value(self.store, o, &body, lang.as_deref())
    }

    /// Sets the retraction of a live row. Returns false if it was not live.
    fn retract_row(&mut self, eid: Eid, kind: RetKind) -> Result<bool> {
        let done = self.store.retract(eid.oid().raw(), self.t as i64, kind)?;
        if done {
            if self.is_membership(eid)? {
                self.memberships_retracted.push((eid, kind));
            } else {
                self.retracted.push((eid, kind));
            }
            self.schema_cache.clear();
        }
        Ok(done)
    }

    /// The id of `sys:inGraph`, `None` while it has never been interned.
    fn in_graph_id(&mut self) -> Result<Option<ObjectId>> {
        if let Some(Some(id)) = self.in_graph {
            return Ok(Some(id));
        }
        let id = self.sys_lookup(tm_core::vocab::SYS_IN_GRAPH)?;
        self.in_graph = Some(id);
        Ok(id)
    }

    /// True when statement `eid` is a graph membership.
    fn is_membership(&mut self, eid: Eid) -> Result<bool> {
        let Some(ig) = self.in_graph_id()? else {
            return Ok(false);
        };
        Ok(self
            .store
            .statement(eid.oid().raw())?
            .is_some_and(|r| r.p == ig.raw()))
    }

    fn is_flag_predicate(&mut self, p: ObjectId) -> Result<bool> {
        Ok(self
            .iri_of(p)?
            .is_some_and(|i| tm_core::vocab::SCHEMA_FLAGS.contains(&i.as_str())))
    }

    /// The live statement rows matching the bound positions, in eid order.
    fn live_rows(
        &self,
        s: Option<ObjectId>,
        p: Option<ObjectId>,
        o: Option<ObjectId>,
    ) -> Result<Vec<StmtRow>> {
        self.store.scan_eid(
            crate::Pattern::new(
                s.map(ObjectId::raw),
                p.map(ObjectId::raw),
                o.map(ObjectId::raw),
            ),
            crate::Scope::Live,
        )
    }
}

impl IntoObject for tm_core::Obj {
    fn into_object(self, tx: &mut Tx<'_>) -> Result<ObjectId> {
        match self {
            tm_core::Obj::Id(id) => id.into_object(tx),
            tm_core::Obj::Val(v) => v.into_object(tx),
        }
    }
}

impl tm_core::TxOps for Tx<'_> {
    tm_core::forward_tx_ops!();

    fn read_exec(
        &mut self,
        _f: &mut dyn FnMut(&mut dyn tm_core::Executor) -> Result<()>,
    ) -> Result<()> {
        Err(Error::MissingCapability {
            capability: "sql".to_string(),
        })
    }

    fn read_store(
        &mut self,
        f: &mut dyn FnMut(&dyn ReadStore) -> Result<()>,
    ) -> Option<Result<()>> {
        Some(f(&*self.store))
    }
}
