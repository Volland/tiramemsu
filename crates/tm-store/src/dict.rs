//! The term dictionary over the storage traits.
//!
//! A port of `tm_core::term`: the writer side ([`TermDict`]) looks up or inserts
//! inside the write transaction, with a committed cache plus a per-transaction
//! overlay, so ids from failed or speculative transactions never leak. The read
//! side ([`TermCache`]) only looks up, with a shared LRU of committed terms.

use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::Mutex;

use lru::LruCache;
use tm_core::codec::{self, Encoded, TermSpec};
use tm_core::term::{Term, TermKey};
use tm_core::{Error, ObjectId, Position, Result, Tag, Value};

use crate::store::{ReadStore, TermRow, WriteStore};

fn to_term(id: i64, r: TermRow) -> Result<Term> {
    Ok(Term {
        id,
        tag: Tag::try_from(r.tag)?,
        lex: r.lex,
        dt: r.dt.map(ObjectId::from_raw),
        lang: r.lang,
        num: r.num,
    })
}

/// Reads a term by id.
pub fn load_term(store: &dyn ReadStore, id: i64) -> Result<Option<Term>> {
    store.term(id)?.map(|r| to_term(id, r)).transpose()
}

fn key_of(t: &Term) -> TermKey {
    TermKey {
        tag: t.tag as u8,
        lex: t.lex.clone(),
        dt: t.dt.map(ObjectId::raw),
        lang: t.lang.clone(),
    }
}

/// The scan-resistant cache of `tm_core::term` (segmented LRU in generations).
#[derive(Debug)]
struct Slru<K, V> {
    protected: HashMap<K, V>,
    probation: HashMap<K, V>,
    capacity: usize,
}

impl<K: std::hash::Hash + Eq, V: Clone> Slru<K, V> {
    fn new(capacity: usize) -> Self {
        Slru {
            protected: HashMap::new(),
            probation: HashMap::new(),
            capacity,
        }
    }

    fn get(&mut self, k: &K) -> Option<V> {
        if let Some(v) = self.protected.get(k) {
            return Some(v.clone());
        }
        let (k, v) = self.probation.remove_entry(k)?;
        if self.protected.len() >= self.capacity {
            self.probation = std::mem::take(&mut self.protected);
        }
        self.protected.insert(k, v.clone());
        Some(v)
    }

    fn insert(&mut self, k: K, v: V) {
        if let Some(slot) = self.protected.get_mut(&k) {
            *slot = v;
            return;
        }
        if self.probation.len() >= self.capacity {
            self.probation.clear();
        }
        self.probation.insert(k, v);
    }

    fn len(&self) -> usize {
        self.protected.len() + self.probation.len()
    }
}

/// Writer-side dictionary: lookup-or-insert with a committed cache and a
/// per-transaction overlay.
#[derive(Debug)]
pub struct TermDict {
    committed: Slru<TermKey, i64>,
    committed_rev: Slru<i64, Term>,
    overlay: HashMap<TermKey, i64>,
    overlay_rev: HashMap<i64, Term>,
    next_term: i64,
}

impl TermDict {
    /// A dictionary whose committed cache holds about `capacity` entries.
    pub fn new(capacity: usize) -> TermDict {
        let capacity = capacity.max(16);
        TermDict {
            committed: Slru::new(capacity),
            committed_rev: Slru::new(capacity),
            overlay: HashMap::new(),
            overlay_rev: HashMap::new(),
            next_term: 1,
        }
    }

    /// Starts a transaction whose next term id is `next_term`.
    pub fn begin(&mut self, next_term: i64) {
        self.overlay.clear();
        self.overlay_rev.clear();
        self.next_term = next_term;
    }

    /// The next term id of the running transaction.
    pub fn next_term(&self) -> i64 {
        self.next_term
    }

    /// Merges the overlay into the committed cache (after commit).
    pub fn commit(&mut self) {
        for (k, id) in self.overlay.drain() {
            self.committed.insert(k, id);
        }
        for (id, t) in self.overlay_rev.drain() {
            self.committed_rev.insert(id, t);
        }
    }

    /// Drops the overlay (after an abort).
    pub fn rollback(&mut self) {
        self.overlay.clear();
        self.overlay_rev.clear();
    }

    /// Number of committed terms cached (both directions).
    pub fn cached(&self) -> usize {
        self.committed_rev.len()
    }

    fn cached_id(&mut self, k: &TermKey) -> Option<i64> {
        match self.overlay.get(k) {
            Some(id) => Some(*id),
            None => self.committed.get(k),
        }
    }

    /// Looks a term up without inserting it.
    pub fn lookup(&mut self, store: &dyn ReadStore, k: &TermKey) -> Result<Option<i64>> {
        if let Some(id) = self.cached_id(k) {
            return Ok(Some(id));
        }
        let found = store.term_id(k)?;
        if let Some(id) = found {
            self.committed.insert(k.clone(), id);
        }
        Ok(found)
    }

    /// Looks a term up, inserting it with `next_term` when missing.
    pub fn lookup_or_insert(
        &mut self,
        store: &mut dyn WriteStore,
        tag: Tag,
        lex: &str,
        dt: Option<ObjectId>,
        lang: Option<&str>,
        num: Option<f64>,
    ) -> Result<i64> {
        let k = TermKey {
            tag: tag as u8,
            lex: lex.to_string(),
            dt: dt.map(ObjectId::raw),
            lang: lang.map(str::to_string),
        };
        if let Some(id) = self.lookup(&*store, &k)? {
            return Ok(id);
        }
        let id = self.next_term;
        store.insert_term(
            id,
            &TermRow {
                tag: tag as u8,
                lex: k.lex.clone(),
                dt: k.dt,
                lang: k.lang.clone(),
                num,
            },
        )?;
        self.next_term += 1;
        self.overlay_rev.insert(
            id,
            Term {
                id,
                tag,
                lex: k.lex.clone(),
                dt,
                lang: k.lang.clone(),
                num,
            },
        );
        self.overlay.insert(k, id);
        Ok(id)
    }

    /// The term with this id, visible to the running transaction.
    pub fn term(&mut self, store: &dyn ReadStore, id: i64) -> Result<Option<Term>> {
        if let Some(t) = self.overlay_rev.get(&id) {
            return Ok(Some(t.clone()));
        }
        if let Some(t) = self.committed_rev.get(&id) {
            return Ok(Some(t));
        }
        let t = load_term(store, id)?;
        if let Some(t) = &t {
            self.committed.insert(key_of(t), id);
            self.committed_rev.insert(id, t.clone());
        }
        Ok(t)
    }

    fn spec_key_read(
        &mut self,
        store: &dyn ReadStore,
        spec: &TermSpec,
    ) -> Result<Option<(TermKey, Option<ObjectId>)>> {
        let dt = match &spec.datatype {
            None => None,
            Some(iri) => {
                let k = TermKey {
                    tag: Tag::Iri as u8,
                    lex: iri.clone(),
                    dt: None,
                    lang: None,
                };
                match self.lookup(store, &k)? {
                    Some(id) => Some(ObjectId::from_unsigned(Tag::Iri, id as u64)),
                    None => return Ok(None),
                }
            }
        };
        Ok(Some((spec_key(spec, dt), dt)))
    }

    /// Encodes a value on the write path, interning dictionary terms.
    pub fn intern(&mut self, store: &mut dyn WriteStore, v: &Value) -> Result<ObjectId> {
        match codec::encode(v) {
            Encoded::Inline(id) => id.check_origin().map(|_| id),
            Encoded::Term(spec) => {
                let dt = match &spec.datatype {
                    None => None,
                    Some(iri) => {
                        let id = self.lookup_or_insert(store, Tag::Iri, iri, None, None, None)?;
                        Some(ObjectId::from_unsigned(Tag::Iri, id as u64))
                    }
                };
                let k = spec_key(&spec, dt);
                let id = self.lookup_or_insert(
                    store,
                    spec.tag,
                    &k.lex,
                    dt,
                    k.lang.as_deref(),
                    spec.num,
                )?;
                Ok(ObjectId::from_unsigned(spec.tag, id as u64))
            }
        }
    }

    /// Encodes a value without inserting; `None` if a needed term is absent.
    pub fn lookup_value(&mut self, store: &dyn ReadStore, v: &Value) -> Result<Option<ObjectId>> {
        match codec::encode(v) {
            Encoded::Inline(id) => id.check_origin().map(|_| Some(id)),
            Encoded::Term(spec) => {
                let Some((k, _)) = self.spec_key_read(store, &spec)? else {
                    return Ok(None);
                };
                Ok(self
                    .lookup(store, &k)?
                    .map(|id| ObjectId::from_unsigned(spec.tag, id as u64)))
            }
        }
    }

    /// The IRI text of an `IRI` id visible to the running transaction.
    pub fn iri(&mut self, store: &dyn ReadStore, id: ObjectId) -> Result<Option<String>> {
        if id.tag_bits() != Tag::Iri as u8 {
            return Ok(None);
        }
        Ok(self
            .term(store, id.unsigned_payload() as i64)?
            .filter(|t| t.tag == Tag::Iri)
            .map(|t| t.lex))
    }

    /// Decodes an id visible to the running transaction.
    pub fn decode(&mut self, store: &dyn ReadStore, id: ObjectId) -> Result<Value> {
        if let Some(v) = codec::decode_inline(id)? {
            return Ok(v);
        }
        let tag = id.tag()?;
        let t = self
            .term(store, id.unsigned_payload() as i64)?
            .filter(|t| t.tag == tag)
            .ok_or_else(|| unknown(id))?;
        let dt = match t.dt {
            Some(dt) => self.iri(store, dt)?,
            None => None,
        };
        codec::value_from_term(t.tag, t.lex, dt, t.lang)
    }
}

fn spec_key(spec: &TermSpec, dt: Option<ObjectId>) -> TermKey {
    TermKey {
        tag: spec.tag as u8,
        lex: spec.lex.clone(),
        dt: dt.map(ObjectId::raw),
        lang: spec.lang.clone(),
    }
}

fn unknown(id: ObjectId) -> Error {
    Error::InvalidTerm {
        position: Position::Value,
        reason: format!("unknown term id {id:?}"),
    }
}

/// Encodes a value for a read: lookup only, never inserts.
pub fn encode(store: &dyn ReadStore, v: &Value) -> Result<Option<ObjectId>> {
    match codec::encode(v) {
        Encoded::Inline(id) => id.check_origin().map(|_| Some(id)),
        Encoded::Term(spec) => {
            let dt = match &spec.datatype {
                None => None,
                Some(iri) => {
                    let k = TermKey {
                        tag: Tag::Iri as u8,
                        lex: iri.clone(),
                        dt: None,
                        lang: None,
                    };
                    match store.term_id(&k)? {
                        Some(id) => Some(ObjectId::from_unsigned(Tag::Iri, id as u64)),
                        None => return Ok(None),
                    }
                }
            };
            let k = spec_key(&spec, dt);
            Ok(store
                .term_id(&k)?
                .map(|id| ObjectId::from_unsigned(spec.tag, id as u64)))
        }
    }
}

/// Read-side term access with a shared LRU of committed terms.
#[derive(Debug)]
pub struct TermCache {
    cache: Mutex<LruCache<i64, Term>>,
}

impl TermCache {
    /// A cache holding `capacity` terms.
    pub fn new(capacity: usize) -> TermCache {
        TermCache {
            cache: Mutex::new(LruCache::new(
                NonZeroUsize::new(capacity.max(1)).expect("non-zero"),
            )),
        }
    }

    fn term(&self, store: &dyn ReadStore, id: i64, use_cache: bool) -> Result<Option<Term>> {
        if use_cache {
            if let Some(t) = self.cache.lock().expect("term cache").get(&id) {
                return Ok(Some(t.clone()));
            }
        }
        let t = load_term(store, id)?;
        if let (true, Some(t)) = (use_cache, &t) {
            self.cache.lock().expect("term cache").put(id, t.clone());
        }
        Ok(t)
    }

    /// Decodes an id. `use_cache` must be false when reading uncommitted state.
    pub fn decode(&self, store: &dyn ReadStore, id: ObjectId, use_cache: bool) -> Result<Value> {
        if let Some(v) = codec::decode_inline(id)? {
            return Ok(v);
        }
        let tag = id.tag()?;
        let t = self
            .term(store, id.unsigned_payload() as i64, use_cache)?
            .filter(|t| t.tag == tag)
            .ok_or_else(|| unknown(id))?;
        let dt = match t.dt {
            Some(dt) => self
                .term(store, dt.unsigned_payload() as i64, use_cache)?
                .map(|t| t.lex),
            None => None,
        };
        codec::value_from_term(t.tag, t.lex, dt, t.lang)
    }

    /// Number of cached terms.
    pub fn cached(&self) -> usize {
        self.cache.lock().expect("term cache").len()
    }
}
