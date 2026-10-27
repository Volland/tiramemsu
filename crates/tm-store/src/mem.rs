//! The in-memory reference store.
//!
//! Ordered maps behind one lock: the executable statement of the storage
//! contract and the fast backend for property tests. A reader holds an `Arc` of
//! the committed state; a writer works on a private copy that replaces the
//! committed state on commit. Copying makes writes O(size), which is fine for a
//! reference store and keeps snapshot isolation obvious.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ops::Bound;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tm_core::term::TermKey;
use tm_core::{Error, Result, RetKind};

use crate::store::{
    duplicate, Backend, EmbeddingRow, Pattern, ReadStore, RowIter, Scope, StmtRow, TermRow,
    TextDoc, VolatileRow, WriteStore, WriteTxn,
};

type Key4 = (i64, i64, i64, i64);

/// The three permutations: `spo`, `pos`, `osp` (each key ends with the eid).
#[derive(Clone, Debug, Default)]
struct Perms {
    spo: BTreeSet<Key4>,
    pos: BTreeSet<Key4>,
    osp: BTreeSet<Key4>,
}

impl Perms {
    fn insert(&mut self, r: &StmtRow) {
        self.spo.insert((r.s, r.p, r.o, r.eid));
        self.pos.insert((r.p, r.o, r.s, r.eid));
        self.osp.insert((r.o, r.s, r.p, r.eid));
    }

    fn remove(&mut self, r: &StmtRow) {
        self.spo.remove(&(r.s, r.p, r.o, r.eid));
        self.pos.remove(&(r.p, r.o, r.s, r.eid));
        self.osp.remove(&(r.o, r.s, r.p, r.eid));
    }

    /// The eids whose key starts with the bound positions, using the permutation
    /// whose prefix covers them. `None` when nothing is bound.
    fn eids(&self, pat: Pattern) -> Option<Vec<i64>> {
        let (set, prefix): (&BTreeSet<Key4>, Vec<i64>) = match (pat.s, pat.p, pat.o) {
            (None, None, None) => return None,
            (Some(s), None, None) => (&self.spo, vec![s]),
            (Some(s), Some(p), None) => (&self.spo, vec![s, p]),
            (Some(s), Some(p), Some(o)) => (&self.spo, vec![s, p, o]),
            (None, Some(p), None) => (&self.pos, vec![p]),
            (None, Some(p), Some(o)) => (&self.pos, vec![p, o]),
            (None, None, Some(o)) => (&self.osp, vec![o]),
            (Some(s), None, Some(o)) => (&self.osp, vec![o, s]),
        };
        let lo = pad(&prefix, i64::MIN);
        let hi = pad(&prefix, i64::MAX);
        Some(set.range(lo..=hi).map(|k| k.3).collect())
    }
}

fn pad(prefix: &[i64], fill: i64) -> Key4 {
    let at = |i: usize| prefix.get(i).copied().unwrap_or(fill);
    (at(0), at(1), at(2), at(3))
}

/// The whole state of a [`MemStore`].
#[derive(Clone, Debug, Default)]
struct State {
    meta: BTreeMap<String, i64>,
    terms: BTreeMap<i64, TermRow>,
    term_keys: HashMap<TermKey, i64>,
    txs: BTreeMap<i64, i64>,
    tx_by_instant: BTreeMap<i64, i64>,
    stmts: BTreeMap<i64, StmtRow>,
    live: Perms,
    all: Perms,
    log_add: BTreeSet<(i64, i64)>,
    log_ret: BTreeSet<(i64, i64)>,
    pred_multi: BTreeSet<i64>,
    volatile: BTreeMap<(i64, i64), VolatileRow>,
    embeddings: BTreeMap<(i64, i64), EmbeddingRow>,
    text_docs: BTreeMap<i64, TextDoc>,
    postings: BTreeMap<(String, i64), Vec<u32>>,
    text_total: u64,
}

impl State {
    fn rows(&self, eids: impl IntoIterator<Item = i64>) -> Vec<StmtRow> {
        eids.into_iter()
            .filter_map(|e| self.stmts.get(&e).copied())
            .collect()
    }

    fn scan(&self, pat: Pattern, scope: Scope) -> Vec<StmtRow> {
        let perms = match scope {
            Scope::Live => &self.live,
            Scope::All => &self.all,
        };
        let rows = match perms.eids(pat) {
            Some(eids) => self.rows(eids),
            None => self
                .stmts
                .values()
                .filter(|r| scope == Scope::All || r.live())
                .copied()
                .collect(),
        };
        rows.into_iter().filter(|r| pat.matches(r)).collect()
    }

    fn log(&self, set: &BTreeSet<(i64, i64)>, t: i64) -> Vec<StmtRow> {
        let eids: Vec<i64> = set
            .range((Bound::Excluded((t, i64::MAX)), Bound::Unbounded))
            .map(|k| k.1)
            .collect();
        self.rows(eids)
    }
}

macro_rules! read_impl {
    ($ty:ty, $state:ident, sync) => {
        read_impl!($ty, $state, {
            fn as_sync(&self) -> Option<&(dyn ReadStore + Sync)> {
                Some(self)
            }
        });
    };
    ($ty:ty, $state:ident) => {
        read_impl!($ty, $state, {});
    };
    ($ty:ty, $state:ident, { $($extra:tt)* }) => {
        impl ReadStore for $ty {
            $($extra)*

            fn meta(&self, key: &str) -> Result<Option<i64>> {
                Ok(self.$state().meta.get(key).copied())
            }

            fn term(&self, id: i64) -> Result<Option<TermRow>> {
                Ok(self.$state().terms.get(&id).cloned())
            }

            fn term_count(&self) -> Result<Option<u64>> {
                Ok(Some(self.$state().terms.len() as u64))
            }

            fn term_id(&self, key: &TermKey) -> Result<Option<i64>> {
                Ok(self.$state().term_keys.get(key).copied())
            }

            fn tx_instant(&self, t: i64) -> Result<Option<i64>> {
                Ok(self.$state().txs.get(&t).copied())
            }

            fn tx_at_or_before(&self, instant: i64) -> Result<i64> {
                Ok(self
                    .$state()
                    .tx_by_instant
                    .range(..=instant)
                    .map(|(_, t)| *t)
                    .max()
                    .unwrap_or(0))
            }

            fn statement(&self, eid: i64) -> Result<Option<StmtRow>> {
                Ok(self.$state().stmts.get(&eid).copied())
            }

            fn scan(&self, pat: Pattern, scope: Scope) -> Result<RowIter<'_>> {
                Ok(Box::new(self.$state().scan(pat, scope).into_iter().map(Ok)))
            }

            fn added_since(&self, t: i64) -> Result<RowIter<'_>> {
                let st = self.$state();
                Ok(Box::new(st.log(&st.log_add, t).into_iter().map(Ok)))
            }

            fn retracted_since(&self, t: i64) -> Result<RowIter<'_>> {
                let st = self.$state();
                Ok(Box::new(st.log(&st.log_ret, t).into_iter().map(Ok)))
            }

            fn pred_multi(&self, p: i64) -> Result<bool> {
                Ok(self.$state().pred_multi.contains(&p))
            }

            fn pred_multi_all(&self) -> Result<Vec<i64>> {
                Ok(self.$state().pred_multi.iter().copied().collect())
            }

            fn volatile(&self, s: i64, key: i64) -> Result<Option<VolatileRow>> {
                Ok(self.$state().volatile.get(&(s, key)).copied())
            }

            fn volatile_all(&self) -> Result<Vec<(i64, i64, VolatileRow)>> {
                Ok(self
                    .$state()
                    .volatile
                    .iter()
                    .map(|((s, k), v)| (*s, *k, *v))
                    .collect())
            }

            fn embedding(&self, model: i64, eid: i64) -> Result<Option<EmbeddingRow>> {
                Ok(self.$state().embeddings.get(&(model, eid)).cloned())
            }

            fn embeddings_all(&self) -> Result<Vec<EmbeddingRow>> {
                Ok(self.$state().embeddings.values().cloned().collect())
            }

            fn embeddings(
                &self,
                model: i64,
            ) -> Result<Box<dyn Iterator<Item = Result<EmbeddingRow>> + '_>> {
                let rows: Vec<EmbeddingRow> = self
                    .$state()
                    .embeddings
                    .range((model, i64::MIN)..=(model, i64::MAX))
                    .map(|(_, r)| r.clone())
                    .collect();
                Ok(Box::new(rows.into_iter().map(Ok)))
            }

            fn text_doc(&self, value: i64) -> Result<Option<TextDoc>> {
                Ok(self.$state().text_docs.get(&value).cloned())
            }

            fn text_postings(&self, token: &str) -> Result<Vec<(i64, Vec<u32>)>> {
                Ok(self
                    .$state()
                    .postings
                    .range((token.to_string(), i64::MIN)..=(token.to_string(), i64::MAX))
                    .map(|((_, v), pos)| (*v, pos.clone()))
                    .collect())
            }

            fn text_tokens_with_prefix(&self, prefix: &str) -> Result<Vec<String>> {
                let mut out: Vec<String> = Vec::new();
                for ((tok, _), _) in self.$state().postings.range((prefix.to_string(), i64::MIN)..) {
                    if !tok.starts_with(prefix) {
                        break;
                    }
                    if out.last() != Some(tok) {
                        out.push(tok.clone());
                    }
                }
                Ok(out)
            }

            fn text_stats(&self) -> Result<(u64, u64)> {
                let st = self.$state();
                Ok((st.text_docs.len() as u64, st.text_total))
            }
        }
    };
}

/// A snapshot reader of a [`MemStore`].
#[derive(Debug)]
// @tg: implements:: [[openspec:storage-trait#Snapshot reads]] {since: "2026-10-27"}
pub struct MemReader {
    state: Arc<State>,
}

impl MemReader {
    fn state(&self) -> &State {
        &self.state
    }
}

read_impl!(MemReader, state, sync);

/// Clears the store's writer flag when the transaction ends, however it ends.
#[derive(Debug)]
struct WriteGuard(Arc<Shared>);

impl Drop for WriteGuard {
    fn drop(&mut self) {
        self.0.writing.store(false, Ordering::Release);
    }
}

/// The write transaction of a [`MemStore`].
#[derive(Debug)]
pub struct MemWriter {
    state: State,
    guard: WriteGuard,
}

impl MemWriter {
    fn state(&self) -> &State {
        &self.state
    }
}

read_impl!(MemWriter, state);

impl WriteStore for MemWriter {
    fn set_meta(&mut self, key: &str, value: i64) -> Result<()> {
        self.state.meta.insert(key.to_string(), value);
        Ok(())
    }

    fn insert_term(&mut self, id: i64, row: &TermRow) -> Result<()> {
        let key = row.key();
        if self.state.terms.contains_key(&id) {
            return Err(duplicate("term id", id));
        }
        if self.state.term_keys.contains_key(&key) {
            return Err(duplicate("term", format!("{:?}", row.lex)));
        }
        self.state.terms.insert(id, row.clone());
        self.state.term_keys.insert(key, id);
        Ok(())
    }

    fn insert_tx(&mut self, t: i64, instant: i64) -> Result<()> {
        if self.state.txs.contains_key(&t) {
            return Err(duplicate("transaction", t));
        }
        if self.state.tx_by_instant.contains_key(&instant) {
            return Err(duplicate("transaction instant", instant));
        }
        self.state.txs.insert(t, instant);
        self.state.tx_by_instant.insert(instant, t);
        Ok(())
    }

    fn insert_statement(&mut self, row: &StmtRow) -> Result<()> {
        if !row.live() {
            return Err(Error::storage("a statement is inserted live"));
        }
        if self.state.stmts.contains_key(&row.eid) {
            return Err(duplicate("statement", row.eid));
        }
        self.state.stmts.insert(row.eid, *row);
        self.state.live.insert(row);
        self.state.all.insert(row);
        self.state.log_add.insert((row.t_add, row.eid));
        Ok(())
    }

    fn retract(&mut self, eid: i64, t: i64, kind: RetKind) -> Result<bool> {
        let Some(row) = self.state.stmts.get_mut(&eid) else {
            return Ok(false);
        };
        if !row.live() {
            return Ok(false);
        }
        row.t_ret = Some(t);
        row.ret_kind = Some(kind as i64);
        let row = *row;
        self.state.live.remove(&row);
        self.state.log_ret.insert((t, eid));
        Ok(true)
    }

    fn add_pred_multi(&mut self, p: i64) -> Result<bool> {
        Ok(self.state.pred_multi.insert(p))
    }

    fn put_volatile(&mut self, s: i64, key: i64, row: VolatileRow) -> Result<()> {
        self.state.volatile.insert((s, key), row);
        Ok(())
    }

    fn delete_volatile(&mut self, s: i64, key: i64) -> Result<bool> {
        Ok(self.state.volatile.remove(&(s, key)).is_some())
    }

    fn insert_embedding(&mut self, row: &EmbeddingRow) -> Result<()> {
        let k = (row.model, row.eid);
        if self.state.embeddings.contains_key(&k) {
            return Err(duplicate("embedding", format!("{k:?}")));
        }
        self.state.embeddings.insert(k, row.clone());
        Ok(())
    }

    fn text_insert(
        &mut self,
        value: i64,
        doc: &TextDoc,
        tokens: &[(String, Vec<u32>)],
    ) -> Result<bool> {
        if self.state.text_docs.contains_key(&value) {
            return Ok(false);
        }
        self.state.text_docs.insert(value, doc.clone());
        self.state.text_total += u64::from(doc.len);
        for (tok, pos) in tokens {
            self.state
                .postings
                .insert((tok.clone(), value), pos.clone());
        }
        Ok(true)
    }

    fn text_clear(&mut self) -> Result<()> {
        self.state.text_docs.clear();
        self.state.postings.clear();
        self.state.text_total = 0;
        Ok(())
    }
}

impl WriteTxn for MemWriter {
    fn commit(self: Box<Self>) -> Result<()> {
        let MemWriter { state, guard } = *self;
        *guard.0.state.lock().expect("mem store") = Arc::new(state);
        Ok(())
    }

    fn abort(self: Box<Self>) -> Result<()> {
        Ok(())
    }
}

#[derive(Debug, Default)]
struct Shared {
    state: Mutex<Arc<State>>,
    writing: AtomicBool,
}

/// The in-memory reference store.
///
/// ```
/// use tm_store::{mem::MemStore, Backend, Pattern, Scope, StmtRow, ReadStore};
///
/// let store = MemStore::new();
/// let mut w = store.write()?;
/// w.insert_statement(&StmtRow { eid: 1, s: 2, p: 3, o: 4, t_add: 1, t_ret: None, v_from: None, v_to: None, ret_kind: None })?;
/// w.commit()?;
/// let r = store.read()?;
/// assert_eq!(r.scan(Pattern::new(Some(2), None, None), Scope::Live)?.count(), 1);
/// # Ok::<(), tm_core::Error>(())
/// ```
#[derive(Clone, Debug, Default)]
pub struct MemStore {
    shared: Arc<Shared>,
}

impl MemStore {
    /// An empty store.
    pub fn new() -> MemStore {
        MemStore::default()
    }
}

impl Backend for MemStore {
    fn name(&self) -> &'static str {
        "memory"
    }

    fn read(&self) -> Result<Box<dyn ReadStore + Send>> {
        let state = self.shared.state.lock().expect("mem store").clone();
        Ok(Box::new(MemReader { state }))
    }

    fn write(&self) -> Result<Box<dyn WriteTxn>> {
        if self.shared.writing.swap(true, Ordering::AcqRel) {
            return Err(Error::storage("a write transaction is already open"));
        }
        let state = (**self.shared.state.lock().expect("mem store")).clone();
        Ok(Box::new(MemWriter {
            state,
            guard: WriteGuard(self.shared.clone()),
        }))
    }
}
