//! The storage contract: typed rows and the `ReadStore` / `WriteStore` traits
//! (`lat.md/architecture#Storage Backends`). The transaction engine over them is
//! in the `tm-store` crate.
//!
//! Every value is the integer the SQLite file stores for it (ObjectId raws,
//! transaction numbers, epoch-ms instants), so two backends holding the same
//! history hold equal rows. The traits are named after the Lean classes of
//! `lean/Tiramemsu/Store/` and implement the access shapes of the transaction
//! engine: point reads, prefix scans with any bound positions, the log, and
//! appends. Nothing here deletes or rewrites a statement, a term or a
//! transaction; the only change to a statement is its single retraction.

use crate::term::TermKey;
use crate::{Error, Result, RetKind};

/// One statement row: the content of the `triple` table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StmtRow {
    /// Statement id (the full `STMT` ObjectId raw).
    pub eid: i64,
    /// Subject ObjectId raw.
    pub s: i64,
    /// Predicate ObjectId raw.
    pub p: i64,
    /// Object ObjectId raw.
    pub o: i64,
    /// Transaction that inserted it.
    pub t_add: i64,
    /// Transaction that retracted it; `None` while live.
    pub t_ret: Option<i64>,
    /// Valid-time start (inclusive); `None` is unbounded.
    pub v_from: Option<i64>,
    /// Valid-time end (exclusive); `None` is unbounded.
    pub v_to: Option<i64>,
    /// Why it was retracted (a [`RetKind`] number).
    pub ret_kind: Option<i64>,
}

impl StmtRow {
    /// True while the statement is not retracted.
    pub fn live(&self) -> bool {
        self.t_ret.is_none()
    }

    /// The retraction kind, decoded.
    pub fn kind(&self) -> Option<RetKind> {
        self.ret_kind.and_then(RetKind::from_i64)
    }
}

/// One dictionary term (the `term` table without its id).
#[derive(Clone, Debug, PartialEq)]
pub struct TermRow {
    /// Tag number.
    pub tag: u8,
    /// Lexical form.
    pub lex: String,
    /// Datatype IRI ObjectId raw.
    pub dt: Option<i64>,
    /// Language tag (lower-cased).
    pub lang: Option<String>,
    /// Numeric value of `DOUBLE` and `DECIMAL` terms.
    pub num: Option<f64>,
}

impl TermRow {
    /// The identity key of this term.
    pub fn key(&self) -> TermKey {
        TermKey {
            tag: self.tag,
            lex: self.lex.clone(),
            dt: self.dt,
            lang: self.lang.clone(),
        }
    }
}

/// One volatile value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VolatileRow {
    /// The value's ObjectId raw.
    pub value: i64,
    /// Instant of the transaction that last set it.
    pub updated_at: i64,
}

/// One stored embedding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmbeddingRow {
    /// Model IRI ObjectId raw.
    pub model: i64,
    /// Statement eid raw.
    pub eid: i64,
    /// Transaction that stored it.
    pub t: i64,
    /// Sign bits, one per dimension.
    pub bits: Vec<u8>,
    /// The packed vector.
    pub vec: Vec<u8>,
}

/// The text-index record of one indexed string value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextDoc {
    /// Number of tokens.
    pub len: u32,
    /// Language tag of a `LANG_STR` value.
    pub lang: Option<String>,
}

/// Bound positions of a scan; `None` is a wildcard.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Pattern {
    /// Subject.
    pub s: Option<i64>,
    /// Predicate.
    pub p: Option<i64>,
    /// Object.
    pub o: Option<i64>,
}

impl Pattern {
    /// Every statement.
    pub const ANY: Pattern = Pattern {
        s: None,
        p: None,
        o: None,
    };

    /// A pattern from optional positions.
    pub fn new(s: Option<i64>, p: Option<i64>, o: Option<i64>) -> Pattern {
        Pattern { s, p, o }
    }

    /// True when `row` matches the bound positions.
    pub fn matches(&self, row: &StmtRow) -> bool {
        self.s.is_none_or(|s| s == row.s)
            && self.p.is_none_or(|p| p == row.p)
            && self.o.is_none_or(|o| o == row.o)
    }

    /// Number of bound positions.
    pub fn bound(&self) -> usize {
        [self.s, self.p, self.o]
            .iter()
            .filter(|x| x.is_some())
            .count()
    }
}

/// Which statements a scan returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Scope {
    /// Only live statements.
    Live,
    /// Every statement ever inserted, live or retracted.
    All,
}

/// A fallible row iterator borrowed from a store.
pub type RowIter<'a> = Box<dyn Iterator<Item = Result<StmtRow>> + 'a>;

/// Read access to one consistent snapshot (or, for a [`WriteStore`], to the
/// transaction's own state including its uncommitted writes).
///
/// Scans return rows in the order of the permutation the backend picks for the
/// bound positions; callers that need eid order sort, or use [`ReadStore::scan_eid`].
// @tg: implements:: [[openspec:storage-trait#Typed storage operations]] {since: "2026-10-27"}
pub trait ReadStore {
    /// The value of a `meta` key (`format_version`, the counters, `text_index`, ...).
    fn meta(&self, key: &str) -> Result<Option<i64>>;

    /// The term with this id.
    fn term(&self, id: i64) -> Result<Option<TermRow>>;

    /// The id of the term with this key.
    fn term_id(&self, key: &TermKey) -> Result<Option<i64>>;

    /// The instant of transaction `t`.
    fn tx_instant(&self, t: i64) -> Result<Option<i64>>;

    /// The largest transaction number whose instant is at most `instant`, 0 when none.
    fn tx_at_or_before(&self, instant: i64) -> Result<i64>;

    /// The statement with this eid.
    fn statement(&self, eid: i64) -> Result<Option<StmtRow>>;

    /// The statements matching `pat` in `scope`, in the backend's permutation order.
    fn scan(&self, pat: Pattern, scope: Scope) -> Result<RowIter<'_>>;

    /// The statements matching `pat` in `scope`, in ascending eid order.
    fn scan_eid(&self, pat: Pattern, scope: Scope) -> Result<Vec<StmtRow>> {
        let mut rows = self.scan(pat, scope)?.collect::<Result<Vec<_>>>()?;
        rows.sort_unstable_by_key(|r| r.eid);
        Ok(rows)
    }

    /// The number of statements matching `pat` in `scope`, exact up to `cap` and
    /// `cap` when there are more.
    fn count(&self, pat: Pattern, scope: Scope, cap: u64) -> Result<u64> {
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

    /// The number of dictionary terms, when the store can tell without a scan.
    fn term_count(&self) -> Result<Option<u64>> {
        Ok(None)
    }

    /// This store, when it can be read from several threads at once (the
    /// native runner's parallel morsels).
    fn as_sync(&self) -> Option<&(dyn ReadStore + Sync)> {
        None
    }

    /// The index a scan of `pat` in `scope` reads, for explain (`"pos live"`),
    /// when the store has more than one.
    fn access_path(&self, pat: Pattern, scope: Scope) -> Option<String> {
        let _ = (pat, scope);
        None
    }

    /// Exact statistics of predicate `p` when the store keeps them: live
    /// statements, distinct live subjects, distinct live objects.
    fn predicate_stats(&self, p: i64) -> Result<Option<(u64, u64, u64)>> {
        let _ = p;
        Ok(None)
    }

    /// Statements with `t_add > t`, ordered by `(t_add, eid)`.
    fn added_since(&self, t: i64) -> Result<RowIter<'_>>;

    /// Statements with `t_ret > t`, ordered by `(t_ret, eid)`.
    fn retracted_since(&self, t: i64) -> Result<RowIter<'_>>;

    /// True when `p` is recorded in `pred_multi`.
    fn pred_multi(&self, p: i64) -> Result<bool>;

    /// Every predicate in `pred_multi`, ascending.
    fn pred_multi_all(&self) -> Result<Vec<i64>>;

    /// The volatile value of `(s, key)`.
    fn volatile(&self, s: i64, key: i64) -> Result<Option<VolatileRow>>;

    /// Every volatile value as `(s, key, row)`, ascending by `(s, key)`.
    fn volatile_all(&self) -> Result<Vec<(i64, i64, VolatileRow)>>;

    /// The embedding of `eid` under `model`.
    fn embedding(&self, model: i64, eid: i64) -> Result<Option<EmbeddingRow>>;

    /// Every embedding, ascending by `(model, eid)`.
    fn embeddings_all(&self) -> Result<Vec<EmbeddingRow>>;

    /// Every embedding of `model`, in eid order.
    fn embeddings(&self, model: i64)
        -> Result<Box<dyn Iterator<Item = Result<EmbeddingRow>> + '_>>;

    /// The text-index record of string value `value`.
    fn text_doc(&self, value: i64) -> Result<Option<TextDoc>>;

    /// The postings of `token`: `(value, positions)` in value order.
    fn text_postings(&self, token: &str) -> Result<Vec<(i64, Vec<u32>)>>;

    /// The indexed tokens starting with `prefix`, ascending.
    fn text_tokens_with_prefix(&self, prefix: &str) -> Result<Vec<String>>;

    /// Number of indexed values and their total token count.
    fn text_stats(&self) -> Result<(u64, u64)>;
}

/// One write transaction. Reads see the transaction's own writes.
// @tg: implements:: [[openspec:storage-trait#Never forget by construction]] {since: "2026-10-27"}
pub trait WriteStore: ReadStore {
    /// Sets a `meta` key.
    fn set_meta(&mut self, key: &str, value: i64) -> Result<()>;

    /// Inserts a term. Fails when the id or the key exists.
    fn insert_term(&mut self, id: i64, row: &TermRow) -> Result<()>;

    /// Inserts a transaction. Fails when `t` exists.
    fn insert_tx(&mut self, t: i64, instant: i64) -> Result<()>;

    /// Inserts a live statement. Fails when its eid exists or it is not live.
    fn insert_statement(&mut self, row: &StmtRow) -> Result<()>;

    /// Retracts a live statement at `t` with `kind`. Returns false (and changes
    /// nothing) when it is unknown or already retracted.
    fn retract(&mut self, eid: i64, t: i64, kind: RetKind) -> Result<bool>;

    /// Records `p` in `pred_multi`; returns false when it was there.
    fn add_pred_multi(&mut self, p: i64) -> Result<bool>;

    /// Upserts a volatile value.
    fn put_volatile(&mut self, s: i64, key: i64, row: VolatileRow) -> Result<()>;

    /// Removes a volatile value; returns whether one existed.
    fn delete_volatile(&mut self, s: i64, key: i64) -> Result<bool>;

    /// Inserts an embedding. Fails when `(model, eid)` exists.
    fn insert_embedding(&mut self, row: &EmbeddingRow) -> Result<()>;

    /// Indexes string value `value` with its tokens and their positions. Returns
    /// false (and changes nothing) when the value is already indexed.
    fn text_insert(
        &mut self,
        value: i64,
        doc: &TextDoc,
        tokens: &[(String, Vec<u32>)],
    ) -> Result<bool>;

    /// Drops the whole text index (derived data, rebuilt from the statements).
    fn text_clear(&mut self) -> Result<()>;
}

/// A write transaction that can be committed or aborted.
pub trait WriteTxn: WriteStore + Send {
    /// Commits every write.
    fn commit(self: Box<Self>) -> Result<()>;
    /// Discards every write.
    fn abort(self: Box<Self>) -> Result<()>;
}

/// A store: hands out snapshot readers and one writer at a time.
pub trait Backend: Send + Sync {
    /// A short name for errors and explain (`"redb"`, `"memory"`).
    fn name(&self) -> &'static str;
    /// A reader on the last committed state.
    fn read(&self) -> Result<Box<dyn ReadStore + Send>>;
    /// A write transaction. Callers serialise writers; a backend may block.
    fn write(&self) -> Result<Box<dyn WriteTxn>>;
}

/// The error of a refused duplicate key.
pub fn duplicate(what: &str, key: impl std::fmt::Display) -> Error {
    Error::storage(format!("{what} {key} already exists; it is never replaced"))
}
