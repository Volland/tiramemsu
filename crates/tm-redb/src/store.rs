//! `ReadStore` / `WriteStore` on redb, the backend handle, durability and
//! maintenance.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use redb::{
    Database, ReadOnlyTable, ReadTransaction, ReadableDatabase, ReadableTable,
    ReadableTableMetadata, Table, WriteTransaction,
};
use tm_core::term::TermKey;
use tm_core::{Error, Result, RetKind};
use tm_store::{
    store::duplicate, Backend, EmbeddingRow, Pattern, ReadStore, RowIter, Scope, StmtRow, TermRow,
    TextDoc, VolatileRow, WriteStore, WriteTxn,
};

use crate::keys::{at, enc, key, prefix_end};
use crate::rows::*;
use crate::tables::*;

/// The layout version this build writes and reads.
pub const LAYOUT: &str = "1";

/// When commits become durable (`OpenOptions::durability` in the facade).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Durability {
    /// Commits are atomic and visible at once; a background flusher makes them
    /// durable at most `flush_interval` later or every `flush_commits` commits.
    #[default]
    Normal,
    /// Every commit is durable when it returns.
    Immediate,
    /// Durable only on [`RedbStore::sync`] and when the store is dropped.
    None,
}

/// Options of a [`RedbStore`].
#[derive(Clone, Debug)]
pub struct RedbOptions {
    /// When commits become durable.
    pub durability: Durability,
    /// The longest a `Normal` commit stays non-durable while the store is open.
    pub flush_interval: Duration,
    /// A `Normal` commit after this many non-durable commits is durable.
    pub flush_commits: u64,
    /// redb page cache size in bytes.
    pub cache_size: usize,
}

impl Default for RedbOptions {
    fn default() -> Self {
        RedbOptions {
            durability: Durability::Normal,
            flush_interval: Duration::from_millis(50),
            flush_commits: 1_000,
            cache_size: 256 << 20,
        }
    }
}

struct Inner {
    /// Read-locked to begin a transaction, write-locked to compact.
    db: RwLock<Database>,
    path: PathBuf,
    opts: RedbOptions,
    writing: AtomicBool,
    /// Non-durable commits since the last durable one.
    pending: AtomicU64,
    last_durable: Mutex<Instant>,
    stop: Mutex<bool>,
    wake: Condvar,
}

/// A tiramemsu database in one redb file.
// @tg: implements:: [[openspec:redb-storage#Same logical database]] {since: "2026-10-27"}
pub struct RedbStore {
    inner: Arc<Inner>,
    flusher: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for RedbStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedbStore")
            .field("path", &self.inner.path)
            .finish()
    }
}

/// The first bytes of every redb file.
pub const MAGIC: [u8; 9] = [b'r', b'e', b'd', b'b', 0x1A, 0x0A, 0xA9, 0x0D, 0x0A];

fn create_tables(w: &WriteTransaction) -> Result<()> {
    w.open_table(INFO).map_err(st)?;
    w.open_table(META).map_err(st)?;
    w.open_table(TERM).map_err(st)?;
    w.open_table(TERM_KEY).map_err(st)?;
    w.open_table(TX).map_err(st)?;
    w.open_table(TX_INSTANT).map_err(st)?;
    w.open_table(STMT).map_err(st)?;
    w.open_table(LOG_ADD).map_err(st)?;
    w.open_table(LOG_RET).map_err(st)?;
    w.open_table(PRED_MULTI).map_err(st)?;
    w.open_table(PRED_STATS).map_err(st)?;
    w.open_table(VOLATILE).map_err(st)?;
    w.open_table(EMBEDDING).map_err(st)?;
    w.open_table(TEXT_DOC).map_err(st)?;
    w.open_table(TEXT_TOKEN).map_err(st)?;
    w.open_table(TEXT_POST).map_err(st)?;
    for (t, _) in LIVE {
        w.open_table(t).map_err(st)?;
    }
    for (t, _) in HIST {
        w.open_table(t).map_err(st)?;
    }
    Ok(())
}

impl RedbStore {
    /// Creates a new database file. Fails when `path` exists.
    pub fn create(path: &Path, opts: RedbOptions) -> Result<RedbStore> {
        if path.exists() {
            return Err(Error::storage(format!("{} already exists", path.display())));
        }
        let db = Database::builder()
            .set_cache_size(opts.cache_size)
            .create(path)
            .map_err(st)?;
        let w = db.begin_write().map_err(st)?;
        create_tables(&w)?;
        {
            let mut info = w.open_table(INFO).map_err(st)?;
            info.insert("engine", "tiramemsu").map_err(st)?;
            info.insert("layout", LAYOUT).map_err(st)?;
        }
        w.commit().map_err(st)?;
        Ok(RedbStore::start(db, path, opts))
    }

    /// Opens an existing database file.
    ///
    /// # Errors
    ///
    /// [`Error::ForeignFile`] for a redb file that is not a tiramemsu database,
    /// [`Error::FormatVersion`] for an unknown layout.
    pub fn open(path: &Path, opts: RedbOptions) -> Result<RedbStore> {
        let db = Database::builder()
            .set_cache_size(opts.cache_size)
            .open(path)
            .map_err(st)?;
        {
            let r = db.begin_read().map_err(st)?;
            let info = r
                .open_table(INFO)
                .map_err(|_| Error::ForeignFile(path.to_path_buf()))?;
            let engine = info
                .get("engine")
                .map_err(st)?
                .map(|v| v.value().to_string());
            if engine.as_deref() != Some("tiramemsu") {
                return Err(Error::ForeignFile(path.to_path_buf()));
            }
            let layout = info
                .get("layout")
                .map_err(st)?
                .map(|v| v.value().to_string());
            if layout.as_deref() != Some(LAYOUT) {
                return Err(Error::FormatVersion {
                    found: layout.and_then(|l| l.parse().ok()).unwrap_or(-1),
                    supported: 1,
                });
            }
        }
        Ok(RedbStore::start(db, path, opts))
    }

    fn start(db: Database, path: &Path, opts: RedbOptions) -> RedbStore {
        let inner = Arc::new(Inner {
            db: RwLock::new(db),
            path: path.to_path_buf(),
            opts: opts.clone(),
            writing: AtomicBool::new(false),
            pending: AtomicU64::new(0),
            last_durable: Mutex::new(Instant::now()),
            stop: Mutex::new(false),
            wake: Condvar::new(),
        });
        let flusher = (opts.durability == Durability::Normal).then(|| {
            let weak = Arc::downgrade(&inner);
            std::thread::Builder::new()
                .name("tm-redb-flush".into())
                .spawn(move || flush_loop(weak))
                .expect("spawn the redb flusher")
        });
        RedbStore { inner, flusher }
    }

    /// The database file.
    pub fn path(&self) -> &Path {
        &self.inner.path
    }

    /// Makes every committed transaction durable.
    pub fn sync(&self) -> Result<()> {
        sync_inner(&self.inner)
    }

    /// Reclaims free pages. Fails while a reader or writer is open.
    // @tg: implements:: [[openspec:redb-storage#Compaction]] {since: "2026-10-27"}
    pub fn compact(&self) -> Result<bool> {
        self.sync()?;
        let mut db = self.inner.db.write().expect("redb database lock");
        db.compact()
            .map_err(|e| Error::storage(format!("redb: compaction refused: {e}")))
    }

    /// The exact statistics of predicate `p`: live statements, distinct live
    /// subjects, distinct live objects.
    pub fn predicate_stats(&self, p: i64) -> Result<(u64, u64, u64)> {
        let r = self
            .inner
            .db
            .read()
            .expect("redb database lock")
            .begin_read()
            .map_err(st)?;
        let t = r.open_table(PRED_STATS).map_err(st)?;
        Ok(stats_of(t.get(p).map_err(st)?.map(|v| *v.value())))
    }

    /// Which of the six live and three history permutations hold statement `eid`
    /// (tests of the layout).
    #[doc(hidden)]
    pub fn presence(&self, eid: i64) -> Result<(Vec<bool>, Vec<bool>)> {
        let r = self
            .inner
            .db
            .read()
            .expect("redb database lock")
            .begin_read()
            .map_err(st)?;
        let stmt = r.open_table(STMT).map_err(st)?;
        let Some(v) = stmt.get(eid).map_err(st)? else {
            return Ok((vec![false; 6], vec![false; 3]));
        };
        let row = stmt_row(eid, v.value());
        let mut live = Vec::new();
        for (def, order) in LIVE {
            let t = r.open_table(def).map_err(st)?;
            live.push(
                t.get(live_key(order, &row).as_slice())
                    .map_err(st)?
                    .is_some(),
            );
        }
        let mut hist = Vec::new();
        for (def, order) in HIST {
            let t = r.open_table(def).map_err(st)?;
            hist.push(
                t.get(hist_key(order, &row).as_slice())
                    .map_err(st)?
                    .is_some(),
            );
        }
        Ok((live, hist))
    }

    /// File size and redb's own statistics, for `tm info`.
    pub fn file_size(&self) -> u64 {
        std::fs::metadata(&self.inner.path).map_or(0, |m| m.len())
    }
}

fn sync_inner(inner: &Inner) -> Result<()> {
    // the lock is held until the commit, so `compact` (which takes it
    // exclusively) never finds this transaction still open
    let db = inner.db.read().expect("redb database lock");
    let mut w = db.begin_write().map_err(st)?;
    w.set_durability(redb::Durability::Immediate).map_err(st)?;
    w.commit().map_err(st)?;
    drop(db);
    inner.pending.store(0, Ordering::Release);
    *inner.last_durable.lock().expect("redb flush") = Instant::now();
    Ok(())
}

fn flush_loop(weak: Weak<Inner>) {
    loop {
        let Some(inner) = weak.upgrade() else { return };
        let interval = inner.opts.flush_interval;
        {
            let stop = inner.stop.lock().expect("redb flush");
            let (stop, _) = inner.wake.wait_timeout(stop, interval).expect("redb flush");
            if *stop {
                return;
            }
        }
        if inner.pending.load(Ordering::Acquire) > 0
            && inner.last_durable.lock().expect("redb flush").elapsed() >= interval
        {
            let _ = sync_inner(&inner);
        }
    }
}

impl Drop for RedbStore {
    fn drop(&mut self) {
        *self.inner.stop.lock().expect("redb flush") = true;
        self.inner.wake.notify_all();
        if let Some(h) = self.flusher.take() {
            let _ = h.join();
        }
        if self.inner.pending.load(Ordering::Acquire) > 0 {
            let _ = sync_inner(&self.inner);
        }
    }
}

fn stats_of(v: Option<[u8; 24]>) -> (u64, u64, u64) {
    match v {
        Some(v) => (at(&v, 0) as u64, at(&v, 1) as u64, at(&v, 2) as u64),
        None => (0, 0, 0),
    }
}

fn stats_value(s: (u64, u64, u64)) -> [u8; 24] {
    let mut v = [0u8; 24];
    v[0..8].copy_from_slice(&enc(s.0 as i64));
    v[8..16].copy_from_slice(&enc(s.1 as i64));
    v[16..24].copy_from_slice(&enc(s.2 as i64));
    v
}

impl Backend for RedbStore {
    fn name(&self) -> &'static str {
        "redb"
    }

    fn read(&self) -> Result<Box<dyn ReadStore + Send>> {
        let txn = self
            .inner
            .db
            .read()
            .expect("redb database lock")
            .begin_read()
            .map_err(st)?;
        Ok(Box::new(RedbRead::new(txn)?))
    }

    fn write(&self) -> Result<Box<dyn WriteTxn>> {
        if self.inner.writing.swap(true, Ordering::AcqRel) {
            return Err(Error::storage("a write transaction is already open"));
        }
        let begun = self
            .inner
            .db
            .read()
            .expect("redb database lock")
            .begin_write();
        let txn = match begun {
            Ok(t) => t,
            Err(e) => {
                self.inner.writing.store(false, Ordering::Release);
                return Err(st(e));
            }
        };
        Ok(Box::new(RedbWrite {
            txn: Some(txn),
            inner: self.inner.clone(),
        }))
    }
}

/// A read snapshot: one redb read transaction with its tables open.
// @tg: implements:: [[openspec:redb-storage#Readers without a pool]] {since: "2026-10-27"}
pub struct RedbRead {
    _txn: ReadTransaction,
    meta: ReadOnlyTable<&'static str, i64>,
    term: ReadOnlyTable<i64, &'static [u8]>,
    term_key: ReadOnlyTable<&'static [u8], i64>,
    tx: ReadOnlyTable<i64, i64>,
    tx_instant: ReadOnlyTable<i64, i64>,
    stmt: ReadOnlyTable<i64, &'static [u8; 64]>,
    live: Vec<ReadOnlyTable<&'static [u8], &'static [u8; 24]>>,
    hist: Vec<ReadOnlyTable<&'static [u8], &'static [u8; 32]>>,
    log_add: ReadOnlyTable<&'static [u8], ()>,
    log_ret: ReadOnlyTable<&'static [u8], ()>,
    pred_multi: ReadOnlyTable<i64, ()>,
    pred_stats: ReadOnlyTable<i64, &'static [u8; 24]>,
    volatile: ReadOnlyTable<&'static [u8], &'static [u8; 16]>,
    embedding: ReadOnlyTable<&'static [u8], &'static [u8]>,
    text_doc: ReadOnlyTable<i64, &'static [u8]>,
    text_token: ReadOnlyTable<&'static str, u64>,
    text_post: ReadOnlyTable<&'static [u8], &'static [u8]>,
}

impl RedbRead {
    fn new(txn: ReadTransaction) -> Result<RedbRead> {
        let live = LIVE
            .iter()
            .map(|(t, _)| txn.open_table(*t).map_err(st))
            .collect::<Result<_>>()?;
        let hist = HIST
            .iter()
            .map(|(t, _)| txn.open_table(*t).map_err(st))
            .collect::<Result<_>>()?;
        Ok(RedbRead {
            meta: txn.open_table(META).map_err(st)?,
            term: txn.open_table(TERM).map_err(st)?,
            term_key: txn.open_table(TERM_KEY).map_err(st)?,
            tx: txn.open_table(TX).map_err(st)?,
            tx_instant: txn.open_table(TX_INSTANT).map_err(st)?,
            stmt: txn.open_table(STMT).map_err(st)?,
            live,
            hist,
            log_add: txn.open_table(LOG_ADD).map_err(st)?,
            log_ret: txn.open_table(LOG_RET).map_err(st)?,
            pred_multi: txn.open_table(PRED_MULTI).map_err(st)?,
            pred_stats: txn.open_table(PRED_STATS).map_err(st)?,
            volatile: txn.open_table(VOLATILE).map_err(st)?,
            embedding: txn.open_table(EMBEDDING).map_err(st)?,
            text_doc: txn.open_table(TEXT_DOC).map_err(st)?,
            text_token: txn.open_table(TEXT_TOKEN).map_err(st)?,
            text_post: txn.open_table(TEXT_POST).map_err(st)?,
            _txn: txn,
        })
    }

    /// The exact statistics of predicate `p` in this snapshot.
    pub fn predicate_stats(&self, p: i64) -> Result<(u64, u64, u64)> {
        Ok(stats_of(
            self.pred_stats.get(p).map_err(st)?.map(|v| *v.value()),
        ))
    }
}

fn log_after<T: ReadableTable<&'static [u8], ()>>(t: &T, after: i64) -> Result<Vec<i64>> {
    let lo = key(&[after, i64::MAX]);
    let mut out = Vec::new();
    for e in t.range::<&[u8]>(lo.as_slice()..).map_err(st)? {
        let (k, _) = e.map_err(st)?;
        if k.value() == lo.as_slice() {
            continue;
        }
        out.push(at(k.value(), 1));
    }
    Ok(out)
}

fn volatile_rows<T: ReadableTable<&'static [u8], &'static [u8; 16]>>(
    t: &T,
) -> Result<Vec<(i64, i64, VolatileRow)>> {
    let mut out = Vec::new();
    for e in t.range::<&[u8]>(..).map_err(st)? {
        let (k, v) = e.map_err(st)?;
        let v = v.value();
        out.push((
            at(k.value(), 0),
            at(k.value(), 1),
            VolatileRow {
                value: at(v, 0),
                updated_at: at(v, 1),
            },
        ));
    }
    Ok(out)
}

fn embedding_rows<T: ReadableTable<&'static [u8], &'static [u8]>>(
    t: &T,
) -> Result<Vec<EmbeddingRow>> {
    let mut out = Vec::new();
    for e in t.range::<&[u8]>(..).map_err(st)? {
        let (k, v) = e.map_err(st)?;
        out.push(embedding_row(at(k.value(), 0), at(k.value(), 1), v.value()));
    }
    Ok(out)
}

fn tokens_with_prefix<T: ReadableTable<&'static str, u64>>(
    t: &T,
    prefix: &str,
) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for e in t.range::<&str>(prefix..).map_err(st)? {
        let (k, _) = e.map_err(st)?;
        if !k.value().starts_with(prefix) {
            break;
        }
        out.push(k.value().to_string());
    }
    Ok(out)
}

fn postings<T: ReadableTable<&'static [u8], &'static [u8]>>(
    t: &T,
    token: &str,
) -> Result<Vec<(i64, Vec<u32>)>> {
    let mut pre = token.as_bytes().to_vec();
    pre.push(0);
    let end = prefix_end(&pre).expect("a token prefix ends");
    let mut out = Vec::new();
    for e in t
        .range::<&[u8]>(pre.as_slice()..end.as_slice())
        .map_err(st)?
    {
        let (k, v) = e.map_err(st)?;
        out.push((at(&k.value()[pre.len()..], 0), positions(v.value())));
    }
    Ok(out)
}

impl ReadStore for RedbRead {
    fn meta(&self, key: &str) -> Result<Option<i64>> {
        Ok(self.meta.get(key).map_err(st)?.map(|v| v.value()))
    }

    fn term(&self, id: i64) -> Result<Option<TermRow>> {
        self.term
            .get(id)
            .map_err(st)?
            .map(|v| term_row(v.value()))
            .transpose()
    }

    fn term_id(&self, k: &TermKey) -> Result<Option<i64>> {
        Ok(self
            .term_key
            .get(term_key(k).as_slice())
            .map_err(st)?
            .map(|v| v.value()))
    }

    fn tx_instant(&self, t: i64) -> Result<Option<i64>> {
        Ok(self.tx.get(t).map_err(st)?.map(|v| v.value()))
    }

    fn tx_at_or_before(&self, instant: i64) -> Result<i64> {
        Ok(self
            .tx_instant
            .range::<i64>(..=instant)
            .map_err(st)?
            .next_back()
            .transpose()
            .map_err(st)?
            .map_or(0, |(_, t)| t.value()))
    }

    fn statement(&self, eid: i64) -> Result<Option<StmtRow>> {
        Ok(self
            .stmt
            .get(eid)
            .map_err(st)?
            .map(|v| stmt_row(eid, v.value())))
    }

    fn scan(&self, pat: Pattern, scope: Scope) -> Result<RowIter<'_>> {
        let (s, p, o) = (pat.s.is_some(), pat.p.is_some(), pat.o.is_some());
        if !(s || p || o) {
            return Ok(Box::new(scan_stmt(&self.stmt, scope == Scope::Live)?));
        }
        Ok(match scope {
            Scope::Live => {
                let i = live_for(s, p, o);
                let order = LIVE[i].1;
                Box::new(scan_live(&self.live[i], order, prefix(order, &pat), pat)?)
            }
            Scope::All => {
                let i = hist_for(s, p, o);
                let order = HIST[i].1;
                Box::new(scan_hist(&self.hist[i], order, prefix(order, &pat), pat)?)
            }
        })
    }

    fn predicate_stats(&self, p: i64) -> Result<Option<(u64, u64, u64)>> {
        Ok(Some(RedbRead::predicate_stats(self, p)?))
    }

    fn access_path(&self, pat: Pattern, scope: Scope) -> Option<String> {
        Some(access_path(pat, scope))
    }

    fn as_sync(&self) -> Option<&(dyn ReadStore + Sync)> {
        Some(self)
    }

    fn term_count(&self) -> Result<Option<u64>> {
        Ok(Some(self.term.len().map_err(st)?))
    }

    fn count(&self, pat: Pattern, scope: Scope, cap: u64) -> Result<u64> {
        if let (Scope::Live, None, Some(p), None) = (scope, pat.s, pat.p, pat.o) {
            return Ok(RedbRead::predicate_stats(self, p)?.0.min(cap));
        }
        if pat == Pattern::ANY && scope == Scope::All {
            return Ok(self.stmt.len().map_err(st)?.min(cap));
        }
        let mut n = 0;
        for r in self.scan(pat, scope)? {
            r?;
            n += 1;
            if n >= cap {
                break;
            }
        }
        Ok(n)
    }

    fn added_since(&self, t: i64) -> Result<RowIter<'_>> {
        let eids = log_after(&self.log_add, t)?;
        Ok(Box::new(
            eids.into_iter()
                .filter_map(|e| self.statement(e).transpose()),
        ))
    }

    fn retracted_since(&self, t: i64) -> Result<RowIter<'_>> {
        let eids = log_after(&self.log_ret, t)?;
        Ok(Box::new(
            eids.into_iter()
                .filter_map(|e| self.statement(e).transpose()),
        ))
    }

    fn pred_multi(&self, p: i64) -> Result<bool> {
        Ok(self.pred_multi.get(p).map_err(st)?.is_some())
    }

    fn pred_multi_all(&self) -> Result<Vec<i64>> {
        self.pred_multi
            .range::<i64>(..)
            .map_err(st)?
            .map(|e| e.map(|(k, _)| k.value()).map_err(st))
            .collect()
    }

    fn volatile(&self, s: i64, k: i64) -> Result<Option<VolatileRow>> {
        Ok(self
            .volatile
            .get(key(&[s, k]).as_slice())
            .map_err(st)?
            .map(|v| {
                let v = v.value();
                VolatileRow {
                    value: at(v, 0),
                    updated_at: at(v, 1),
                }
            }))
    }

    fn volatile_all(&self) -> Result<Vec<(i64, i64, VolatileRow)>> {
        volatile_rows(&self.volatile)
    }

    fn embeddings_all(&self) -> Result<Vec<EmbeddingRow>> {
        embedding_rows(&self.embedding)
    }

    fn embedding(&self, model: i64, eid: i64) -> Result<Option<EmbeddingRow>> {
        Ok(self
            .embedding
            .get(key(&[model, eid]).as_slice())
            .map_err(st)?
            .map(|v| embedding_row(model, eid, v.value())))
    }

    fn embeddings(
        &self,
        model: i64,
    ) -> Result<Box<dyn Iterator<Item = Result<EmbeddingRow>> + '_>> {
        let pre = key(&[model]);
        let end = prefix_end(&pre).expect("an id prefix ends");
        let range = self
            .embedding
            .range::<&[u8]>(pre.as_slice()..end.as_slice())
            .map_err(st)?;
        Ok(Box::new(range.map(move |e| {
            let (k, v) = e.map_err(st)?;
            Ok(embedding_row(model, at(k.value(), 1), v.value()))
        })))
    }

    fn text_doc(&self, value: i64) -> Result<Option<TextDoc>> {
        Ok(self
            .text_doc
            .get(value)
            .map_err(st)?
            .map(|v| text_doc(v.value())))
    }

    fn text_postings(&self, token: &str) -> Result<Vec<(i64, Vec<u32>)>> {
        postings(&self.text_post, token)
    }

    fn text_tokens_with_prefix(&self, prefix: &str) -> Result<Vec<String>> {
        tokens_with_prefix(&self.text_token, prefix)
    }

    fn text_stats(&self) -> Result<(u64, u64)> {
        Ok((
            self.meta("text_docs")?.unwrap_or(0) as u64,
            self.meta("text_len")?.unwrap_or(0) as u64,
        ))
    }
}

/// A write transaction. Tables are opened per operation, so the transaction
/// never borrows from itself.
pub struct RedbWrite {
    txn: Option<WriteTransaction>,
    inner: Arc<Inner>,
}

impl Drop for RedbWrite {
    fn drop(&mut self) {
        self.inner.writing.store(false, Ordering::Release);
    }
}

impl RedbWrite {
    fn txn(&self) -> &WriteTransaction {
        self.txn.as_ref().expect("open until commit or abort")
    }

    fn table<K: redb::Key + 'static, V: redb::Value + 'static>(
        &self,
        def: redb::TableDefinition<'static, K, V>,
    ) -> Result<Table<'_, K, V>> {
        self.txn().open_table(def).map_err(st)
    }

    fn get_stats(&self, p: i64) -> Result<(u64, u64, u64)> {
        let t = self.table(PRED_STATS)?;
        let v = t.get(p).map_err(st)?.map(|v| *v.value());
        Ok(stats_of(v))
    }

    // @tg: implements:: [[openspec:redb-storage#Exact predicate statistics]] {since: "2026-10-27"}
    fn put_stats(&self, p: i64, s: (u64, u64, u64)) -> Result<()> {
        self.table(PRED_STATS)?
            .insert(p, &stats_value(s))
            .map_err(st)?;
        Ok(())
    }

    /// Number of live keys with the given two-position prefix in permutation `i`, up to 2.
    fn prefix_count(&self, i: usize, a: i64, b: i64) -> Result<usize> {
        let t = self.table(LIVE[i].0)?;
        let pre = key(&[a, b]);
        let end = prefix_end(&pre).expect("an id prefix ends");
        let n = t
            .range::<&[u8]>(pre.as_slice()..end.as_slice())
            .map_err(st)?
            .take(2)
            .count();
        Ok(n)
    }
}

impl ReadStore for RedbWrite {
    fn meta(&self, k: &str) -> Result<Option<i64>> {
        Ok(self.table(META)?.get(k).map_err(st)?.map(|v| v.value()))
    }

    fn term(&self, id: i64) -> Result<Option<TermRow>> {
        let t = self.table(TERM)?;
        let v = t.get(id).map_err(st)?;
        v.map(|v| term_row(v.value())).transpose()
    }

    fn term_id(&self, k: &TermKey) -> Result<Option<i64>> {
        let t = self.table(TERM_KEY)?;
        let v = t
            .get(term_key(k).as_slice())
            .map_err(st)?
            .map(|v| v.value());
        Ok(v)
    }

    fn tx_instant(&self, t: i64) -> Result<Option<i64>> {
        Ok(self.table(TX)?.get(t).map_err(st)?.map(|v| v.value()))
    }

    fn tx_at_or_before(&self, instant: i64) -> Result<i64> {
        let t = self.table(TX_INSTANT)?;
        let last = t
            .range::<i64>(..=instant)
            .map_err(st)?
            .next_back()
            .transpose()
            .map_err(st)?
            .map_or(0, |(_, t)| t.value());
        Ok(last)
    }

    fn statement(&self, eid: i64) -> Result<Option<StmtRow>> {
        let t = self.table(STMT)?;
        let v = t.get(eid).map_err(st)?.map(|v| stmt_row(eid, v.value()));
        Ok(v)
    }

    fn scan(&self, pat: Pattern, scope: Scope) -> Result<RowIter<'_>> {
        let (s, p, o) = (pat.s.is_some(), pat.p.is_some(), pat.o.is_some());
        let rows: Vec<Result<StmtRow>> = if !(s || p || o) {
            let t = self.table(STMT)?;
            let v: Vec<Result<StmtRow>> = scan_stmt(&t, scope == Scope::Live)?.collect();
            v
        } else {
            match scope {
                Scope::Live => {
                    let i = live_for(s, p, o);
                    let order = LIVE[i].1;
                    let t = self.table(LIVE[i].0)?;
                    let v: Vec<Result<StmtRow>> =
                        scan_live(&t, order, prefix(order, &pat), pat)?.collect();
                    v
                }
                Scope::All => {
                    let i = hist_for(s, p, o);
                    let order = HIST[i].1;
                    let t = self.table(HIST[i].0)?;
                    let v: Vec<Result<StmtRow>> =
                        scan_hist(&t, order, prefix(order, &pat), pat)?.collect();
                    v
                }
            }
        };
        Ok(Box::new(rows.into_iter()))
    }

    fn added_since(&self, t: i64) -> Result<RowIter<'_>> {
        let eids = log_after(&self.table(LOG_ADD)?, t)?;
        let rows: Vec<Result<StmtRow>> = eids
            .into_iter()
            .filter_map(|e| self.statement(e).transpose())
            .collect();
        Ok(Box::new(rows.into_iter()))
    }

    fn retracted_since(&self, t: i64) -> Result<RowIter<'_>> {
        let eids = log_after(&self.table(LOG_RET)?, t)?;
        let rows: Vec<Result<StmtRow>> = eids
            .into_iter()
            .filter_map(|e| self.statement(e).transpose())
            .collect();
        Ok(Box::new(rows.into_iter()))
    }

    fn pred_multi(&self, p: i64) -> Result<bool> {
        Ok(self.table(PRED_MULTI)?.get(p).map_err(st)?.is_some())
    }

    fn pred_multi_all(&self) -> Result<Vec<i64>> {
        let t = self.table(PRED_MULTI)?;
        let v: Result<Vec<i64>> = t
            .range::<i64>(..)
            .map_err(st)?
            .map(|e| e.map(|(k, _)| k.value()).map_err(st))
            .collect();
        v
    }

    fn volatile(&self, s: i64, k: i64) -> Result<Option<VolatileRow>> {
        let t = self.table(VOLATILE)?;
        let v = t.get(key(&[s, k]).as_slice()).map_err(st)?.map(|v| {
            let v = v.value();
            VolatileRow {
                value: at(v, 0),
                updated_at: at(v, 1),
            }
        });
        Ok(v)
    }

    fn volatile_all(&self) -> Result<Vec<(i64, i64, VolatileRow)>> {
        volatile_rows(&self.table(VOLATILE)?)
    }

    fn embeddings_all(&self) -> Result<Vec<EmbeddingRow>> {
        embedding_rows(&self.table(EMBEDDING)?)
    }

    fn embedding(&self, model: i64, eid: i64) -> Result<Option<EmbeddingRow>> {
        let t = self.table(EMBEDDING)?;
        let v = t
            .get(key(&[model, eid]).as_slice())
            .map_err(st)?
            .map(|v| embedding_row(model, eid, v.value()));
        Ok(v)
    }

    fn embeddings(
        &self,
        model: i64,
    ) -> Result<Box<dyn Iterator<Item = Result<EmbeddingRow>> + '_>> {
        let t = self.table(EMBEDDING)?;
        let pre = key(&[model]);
        let end = prefix_end(&pre).expect("an id prefix ends");
        let rows: Vec<Result<EmbeddingRow>> = t
            .range::<&[u8]>(pre.as_slice()..end.as_slice())
            .map_err(st)?
            .map(|e| {
                let (k, v) = e.map_err(st)?;
                Ok(embedding_row(model, at(k.value(), 1), v.value()))
            })
            .collect();
        Ok(Box::new(rows.into_iter()))
    }

    fn text_doc(&self, value: i64) -> Result<Option<TextDoc>> {
        let t = self.table(TEXT_DOC)?;
        let v = t.get(value).map_err(st)?.map(|v| text_doc(v.value()));
        Ok(v)
    }

    fn text_postings(&self, token: &str) -> Result<Vec<(i64, Vec<u32>)>> {
        postings(&self.table(TEXT_POST)?, token)
    }

    fn text_tokens_with_prefix(&self, prefix: &str) -> Result<Vec<String>> {
        tokens_with_prefix(&self.table(TEXT_TOKEN)?, prefix)
    }

    fn text_stats(&self) -> Result<(u64, u64)> {
        Ok((
            self.meta("text_docs")?.unwrap_or(0) as u64,
            self.meta("text_len")?.unwrap_or(0) as u64,
        ))
    }
}

impl WriteStore for RedbWrite {
    fn set_meta(&mut self, k: &str, value: i64) -> Result<()> {
        self.table(META)?.insert(k, value).map_err(st)?;
        Ok(())
    }

    fn insert_term(&mut self, id: i64, row: &TermRow) -> Result<()> {
        let k = term_key(&row.key());
        {
            let mut keys = self.table(TERM_KEY)?;
            if keys.get(k.as_slice()).map_err(st)?.is_some() {
                return Err(duplicate("term", format!("{:?}", row.lex)));
            }
            keys.insert(k.as_slice(), id).map_err(st)?;
        }
        let mut t = self.table(TERM)?;
        if t.get(id).map_err(st)?.is_some() {
            return Err(duplicate("term id", id));
        }
        t.insert(id, term_value(row).as_slice()).map_err(st)?;
        Ok(())
    }

    fn insert_tx(&mut self, t: i64, instant: i64) -> Result<()> {
        {
            let mut tx = self.table(TX)?;
            if tx.get(t).map_err(st)?.is_some() {
                return Err(duplicate("transaction", t));
            }
            tx.insert(t, instant).map_err(st)?;
        }
        let mut by = self.table(TX_INSTANT)?;
        if by.get(instant).map_err(st)?.is_some() {
            return Err(duplicate("transaction instant", instant));
        }
        by.insert(instant, t).map_err(st)?;
        Ok(())
    }

    fn insert_statement(&mut self, r: &StmtRow) -> Result<()> {
        if !r.live() {
            return Err(Error::storage("a statement is inserted live"));
        }
        {
            let mut stmt = self.table(STMT)?;
            if stmt.get(r.eid).map_err(st)?.is_some() {
                return Err(duplicate("statement", r.eid));
            }
            stmt.insert(r.eid, &stmt_value(r)).map_err(st)?;
        }
        let lv = live_value(r);
        for (def, order) in LIVE {
            self.table(def)?
                .insert(live_key(order, r).as_slice(), &lv)
                .map_err(st)?;
        }
        let hv = hist_value(r);
        for (def, order) in HIST {
            self.table(def)?
                .insert(hist_key(order, r).as_slice(), &hv)
                .map_err(st)?;
        }
        self.table(LOG_ADD)?
            .insert(key(&[r.t_add, r.eid]).as_slice(), ())
            .map_err(st)?;
        // statistics: one more live statement; a new subject or object of p when
        // the (p, s) or (p, o) prefix holds only this statement
        let (mut live, mut ds, mut dobj) = self.get_stats(r.p)?;
        live += 1;
        if self.prefix_count(2, r.p, r.s)? == 1 {
            ds += 1;
        }
        if self.prefix_count(3, r.p, r.o)? == 1 {
            dobj += 1;
        }
        self.put_stats(r.p, (live, ds, dobj))
    }

    fn retract(&mut self, eid: i64, t: i64, kind: RetKind) -> Result<bool> {
        let Some(old) = self.statement(eid)? else {
            return Ok(false);
        };
        if !old.live() {
            return Ok(false);
        }
        let new = StmtRow {
            t_ret: Some(t),
            ret_kind: Some(kind as i64),
            ..old
        };
        self.table(STMT)?
            .insert(eid, &stmt_value(&new))
            .map_err(st)?;
        for (def, order) in LIVE {
            self.table(def)?
                .remove(live_key(order, &old).as_slice())
                .map_err(st)?;
        }
        let hv = hist_value(&new);
        for (def, order) in HIST {
            self.table(def)?
                .insert(hist_key(order, &old).as_slice(), &hv)
                .map_err(st)?;
        }
        self.table(LOG_RET)?
            .insert(key(&[t, eid]).as_slice(), ())
            .map_err(st)?;
        let (mut live, mut ds, mut dobj) = self.get_stats(old.p)?;
        live = live.saturating_sub(1);
        if self.prefix_count(2, old.p, old.s)? == 0 {
            ds = ds.saturating_sub(1);
        }
        if self.prefix_count(3, old.p, old.o)? == 0 {
            dobj = dobj.saturating_sub(1);
        }
        self.put_stats(old.p, (live, ds, dobj))?;
        Ok(true)
    }

    fn add_pred_multi(&mut self, p: i64) -> Result<bool> {
        let mut t = self.table(PRED_MULTI)?;
        let had = t.insert(p, ()).map_err(st)?.is_some();
        Ok(!had)
    }

    fn put_volatile(&mut self, s: i64, k: i64, row: VolatileRow) -> Result<()> {
        let mut v = [0u8; 16];
        v[0..8].copy_from_slice(&enc(row.value));
        v[8..16].copy_from_slice(&enc(row.updated_at));
        self.table(VOLATILE)?
            .insert(key(&[s, k]).as_slice(), &v)
            .map_err(st)?;
        Ok(())
    }

    fn delete_volatile(&mut self, s: i64, k: i64) -> Result<bool> {
        let mut t = self.table(VOLATILE)?;
        let had = t.remove(key(&[s, k]).as_slice()).map_err(st)?.is_some();
        Ok(had)
    }

    fn insert_embedding(&mut self, row: &EmbeddingRow) -> Result<()> {
        let k = key(&[row.model, row.eid]);
        let mut t = self.table(EMBEDDING)?;
        if t.get(k.as_slice()).map_err(st)?.is_some() {
            return Err(duplicate(
                "embedding",
                format!("({}, {})", row.model, row.eid),
            ));
        }
        t.insert(k.as_slice(), embedding_value(row).as_slice())
            .map_err(st)?;
        Ok(())
    }

    fn text_insert(
        &mut self,
        value: i64,
        doc: &TextDoc,
        tokens: &[(String, Vec<u32>)],
    ) -> Result<bool> {
        {
            let mut docs = self.table(TEXT_DOC)?;
            if docs.get(value).map_err(st)?.is_some() {
                return Ok(false);
            }
            docs.insert(value, text_doc_value(doc).as_slice())
                .map_err(st)?;
        }
        {
            let mut post = self.table(TEXT_POST)?;
            for (tok, pos) in tokens {
                post.insert(
                    post_key(tok, value).as_slice(),
                    positions_value(pos).as_slice(),
                )
                .map_err(st)?;
            }
        }
        {
            let mut toks = self.table(TEXT_TOKEN)?;
            for (tok, _) in tokens {
                let n = toks.get(tok.as_str()).map_err(st)?.map_or(0, |v| v.value());
                toks.insert(tok.as_str(), n + 1).map_err(st)?;
            }
        }
        let (docs, len) = self.text_stats()?;
        self.set_meta("text_docs", docs as i64 + 1)?;
        self.set_meta("text_len", (len + u64::from(doc.len)) as i64)?;
        Ok(true)
    }

    fn text_clear(&mut self) -> Result<()> {
        let txn = self.txn();
        txn.delete_table(TEXT_DOC).map_err(st)?;
        txn.delete_table(TEXT_TOKEN).map_err(st)?;
        txn.delete_table(TEXT_POST).map_err(st)?;
        txn.open_table(TEXT_DOC).map_err(st)?;
        txn.open_table(TEXT_TOKEN).map_err(st)?;
        txn.open_table(TEXT_POST).map_err(st)?;
        self.set_meta("text_docs", 0)?;
        self.set_meta("text_len", 0)
    }
}

impl WriteTxn for RedbWrite {
    fn commit(mut self: Box<Self>) -> Result<()> {
        let mut txn = self.txn.take().expect("open until commit or abort");
        let inner = &self.inner;
        let durable = match inner.opts.durability {
            Durability::Immediate => true,
            Durability::None => false,
            Durability::Normal => {
                inner.pending.load(Ordering::Acquire) + 1 >= inner.opts.flush_commits
                    || inner.last_durable.lock().expect("redb flush").elapsed()
                        >= inner.opts.flush_interval
            }
        };
        txn.set_durability(if durable {
            redb::Durability::Immediate
        } else {
            redb::Durability::None
        })
        .map_err(st)?;
        txn.commit().map_err(st)?;
        if durable {
            inner.pending.store(0, Ordering::Release);
            *inner.last_durable.lock().expect("redb flush") = Instant::now();
        } else {
            inner.pending.fetch_add(1, Ordering::AcqRel);
        }
        Ok(())
    }

    fn abort(mut self: Box<Self>) -> Result<()> {
        if let Some(txn) = self.txn.take() {
            txn.abort().map_err(st)?;
        }
        Ok(())
    }
}

/// The permutation and scope a scan of `pat` reads, as explain names it.
fn access_path(pat: Pattern, scope: Scope) -> String {
    let (s, p, o) = (pat.s.is_some(), pat.p.is_some(), pat.o.is_some());
    if !(s || p || o) {
        return format!(
            "stmt {}",
            if scope == Scope::Live {
                "live"
            } else {
                "history"
            }
        );
    }
    let (order, kind) = match scope {
        Scope::Live => (LIVE[live_for(s, p, o)].1, "live"),
        Scope::All => (HIST[hist_for(s, p, o)].1, "history"),
    };
    let name: String = order.iter().map(|i| ['s', 'p', 'o'][*i]).collect();
    format!("{name} {kind}")
}
