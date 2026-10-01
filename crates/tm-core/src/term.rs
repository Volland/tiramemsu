//! The term dictionary: values that do not fit inline.
//!
//! The writer side ([`TermDict`]) looks up or inserts inside the writer
//! transaction and keeps a committed cache plus a per-transaction overlay, so ids
//! from failed or speculative transactions never leak. The read side
//! ([`TermReader`]) only looks up, and caches `id -> term` in a shared LRU.

use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::Mutex;

use lru::LruCache;

use crate::codec::{self, Encoded, TermSpec};
use crate::error::{Error, Position, Result};
use crate::exec::{Executor, SqlValue};
use crate::id::{ObjectId, Tag};
use crate::value::Value;

const LOOKUP_SQL: &str = "SELECT id FROM term WHERE tag = ?1 AND lex = ?2 \
     AND ifnull(dt, 0) = ifnull(?3, 0) AND ifnull(lang, '') = ifnull(?4, '')";
const BY_ID_SQL: &str = "SELECT tag, lex, dt, lang, num FROM term WHERE id = ?1";
const INSERT_SQL: &str =
    "INSERT INTO term(id, tag, lex, dt, lang, num) VALUES (?1, ?2, ?3, ?4, ?5, ?6)";

/// The identity of a dictionary term: `(tag, lex, dt, lang)`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TermKey {
    /// Tag number.
    pub tag: u8,
    /// Lexical form.
    pub lex: String,
    /// Datatype IRI ObjectId (raw).
    pub dt: Option<i64>,
    /// Language tag.
    pub lang: Option<String>,
}

/// A stored term row.
#[derive(Clone, Debug, PartialEq)]
pub struct Term {
    /// Term id (the ObjectId payload).
    pub id: i64,
    /// Tag.
    pub tag: Tag,
    /// Lexical form.
    pub lex: String,
    /// Datatype IRI ObjectId.
    pub dt: Option<ObjectId>,
    /// Language tag.
    pub lang: Option<String>,
    /// Numeric value.
    pub num: Option<f64>,
}

impl Term {
    fn key(&self) -> TermKey {
        TermKey {
            tag: self.tag as u8,
            lex: self.lex.clone(),
            dt: self.dt.map(ObjectId::raw),
            lang: self.lang.clone(),
        }
    }

    /// The ObjectId of this term.
    pub fn oid(&self) -> ObjectId {
        ObjectId::from_unsigned(self.tag, self.id as u64)
    }
}

fn key_params(k: &TermKey) -> [SqlValue; 4] {
    [
        SqlValue::Integer(k.tag as i64),
        SqlValue::Text(k.lex.clone()),
        SqlValue::from(k.dt),
        k.lang.clone().map_or(SqlValue::Null, SqlValue::Text),
    ]
}

fn sql_lookup(exec: &mut dyn Executor, k: &TermKey) -> Result<Option<i64>> {
    exec.query_i64(LOOKUP_SQL, &key_params(k))
}

/// Reads a term row by id.
pub fn load_term(exec: &mut dyn Executor, id: i64) -> Result<Option<Term>> {
    let Some(r) = exec.first_row(BY_ID_SQL, &[SqlValue::Integer(id)])? else {
        return Ok(None);
    };
    let tag = Tag::try_from(r[0].as_i64().unwrap_or(-1).clamp(0, 255) as u8)?;
    Ok(Some(Term {
        id,
        tag,
        lex: r[1].as_str().unwrap_or_default().to_string(),
        dt: r[2].as_i64().map(ObjectId::from_raw),
        lang: r[3].as_str().map(str::to_string),
        num: r[4].as_f64(),
    }))
}

/// Writer-side dictionary: lookup-or-insert with a committed cache and a
/// per-transaction overlay.
// @lat: [[storage#Term Dictionary]]
#[derive(Debug)]
pub struct TermDict {
    committed: HashMap<TermKey, i64>,
    committed_rev: HashMap<i64, Term>,
    overlay: HashMap<TermKey, i64>,
    overlay_rev: HashMap<i64, Term>,
    next_term: i64,
    capacity: usize,
}

impl TermDict {
    /// A dictionary whose committed cache holds about `capacity` entries.
    pub fn new(capacity: usize) -> TermDict {
        TermDict {
            committed: HashMap::new(),
            committed_rev: HashMap::new(),
            overlay: HashMap::new(),
            overlay_rev: HashMap::new(),
            next_term: 1,
            capacity: capacity.max(16),
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

    /// Merges the overlay into the committed cache (after `COMMIT`).
    pub fn commit(&mut self) {
        if self.committed.len() + self.overlay.len() > self.capacity * 2 {
            self.committed.clear();
            self.committed_rev.clear();
        }
        self.committed.extend(self.overlay.drain());
        self.committed_rev.extend(self.overlay_rev.drain());
    }

    /// Drops the overlay (after `ROLLBACK` or a savepoint rollback).
    pub fn rollback(&mut self) {
        self.overlay.clear();
        self.overlay_rev.clear();
    }

    fn cached(&self, k: &TermKey) -> Option<i64> {
        self.overlay
            .get(k)
            .or_else(|| self.committed.get(k))
            .copied()
    }

    /// Looks a term up without inserting it.
    pub fn lookup(&mut self, exec: &mut dyn Executor, k: &TermKey) -> Result<Option<i64>> {
        if let Some(id) = self.cached(k) {
            return Ok(Some(id));
        }
        let found = sql_lookup(exec, k)?;
        if let Some(id) = found {
            // not in the overlay, so the row was committed before this transaction
            self.committed.insert(k.clone(), id);
        }
        Ok(found)
    }

    /// Looks a term up, inserting it with `next_term` when missing.
    pub fn lookup_or_insert(
        &mut self,
        exec: &mut dyn Executor,
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
        if let Some(id) = self.lookup(exec, &k)? {
            return Ok(id);
        }
        let id = self.next_term;
        exec.execute(
            INSERT_SQL,
            &[
                SqlValue::Integer(id),
                SqlValue::Integer(tag as i64),
                SqlValue::Text(k.lex.clone()),
                SqlValue::from(k.dt),
                k.lang.clone().map_or(SqlValue::Null, SqlValue::Text),
                SqlValue::from(num),
            ],
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
    pub fn term(&mut self, exec: &mut dyn Executor, id: i64) -> Result<Option<Term>> {
        if let Some(t) = self
            .overlay_rev
            .get(&id)
            .or_else(|| self.committed_rev.get(&id))
        {
            return Ok(Some(t.clone()));
        }
        let t = load_term(exec, id)?;
        if let Some(t) = &t {
            self.committed.insert(t.key(), id);
            self.committed_rev.insert(id, t.clone());
        }
        Ok(t)
    }

    fn spec_key(
        &mut self,
        exec: &mut dyn Executor,
        spec: &TermSpec,
        insert: bool,
    ) -> Result<Option<(TermKey, Option<ObjectId>)>> {
        let dt = match &spec.datatype {
            None => None,
            Some(iri) => {
                let id = if insert {
                    Some(self.lookup_or_insert(exec, Tag::Iri, iri, None, None, None)?)
                } else {
                    self.lookup(
                        exec,
                        &TermKey {
                            tag: Tag::Iri as u8,
                            lex: iri.clone(),
                            dt: None,
                            lang: None,
                        },
                    )?
                };
                match id {
                    Some(id) => Some(ObjectId::from_unsigned(Tag::Iri, id as u64)),
                    None => return Ok(None),
                }
            }
        };
        Ok(Some((
            TermKey {
                tag: spec.tag as u8,
                lex: spec.lex.clone(),
                dt: dt.map(ObjectId::raw),
                lang: spec.lang.clone(),
            },
            dt,
        )))
    }

    /// Encodes a value on the write path, interning dictionary terms.
    pub fn intern(&mut self, exec: &mut dyn Executor, v: &Value) -> Result<ObjectId> {
        match codec::encode(v) {
            Encoded::Inline(id) => id.check_origin().map(|_| id),
            Encoded::Term(spec) => {
                let (k, dt) = self
                    .spec_key(exec, &spec, true)?
                    .expect("interned datatype");
                let id =
                    self.lookup_or_insert(exec, spec.tag, &k.lex, dt, k.lang.as_deref(), spec.num)?;
                Ok(ObjectId::from_unsigned(spec.tag, id as u64))
            }
        }
    }

    /// Encodes a value without inserting; `None` if a needed term is absent.
    pub fn lookup_value(&mut self, exec: &mut dyn Executor, v: &Value) -> Result<Option<ObjectId>> {
        match codec::encode(v) {
            Encoded::Inline(id) => id.check_origin().map(|_| Some(id)),
            Encoded::Term(spec) => {
                let Some((k, _)) = self.spec_key(exec, &spec, false)? else {
                    return Ok(None);
                };
                Ok(self
                    .lookup(exec, &k)?
                    .map(|id| ObjectId::from_unsigned(spec.tag, id as u64)))
            }
        }
    }

    /// The IRI text of an `IRI` id visible to the running transaction.
    pub fn iri(&mut self, exec: &mut dyn Executor, id: ObjectId) -> Result<Option<String>> {
        if id.tag_bits() != Tag::Iri as u8 {
            return Ok(None);
        }
        Ok(self
            .term(exec, id.unsigned_payload() as i64)?
            .filter(|t| t.tag == Tag::Iri)
            .map(|t| t.lex))
    }

    /// Decodes an id visible to the running transaction.
    pub fn decode(&mut self, exec: &mut dyn Executor, id: ObjectId) -> Result<Value> {
        if let Some(v) = codec::decode_inline(id)? {
            return Ok(v);
        }
        let tag = id.tag()?;
        let t = self
            .term(exec, id.unsigned_payload() as i64)?
            .filter(|t| t.tag == tag)
            .ok_or_else(|| unknown(id))?;
        let dt = match t.dt {
            Some(dt) => self.iri(exec, dt)?,
            None => None,
        };
        codec::value_from_term(t.tag, t.lex, dt, t.lang)
    }
}

fn unknown(id: ObjectId) -> Error {
    Error::InvalidTerm {
        position: Position::Value,
        reason: format!("unknown term id {id:?}"),
    }
}

/// Read-side dictionary access: lookups only, with a shared `id -> term` LRU
/// filled from committed rows.
#[derive(Debug)]
pub struct TermReader {
    cache: Mutex<LruCache<i64, Term>>,
}

impl TermReader {
    /// A reader whose LRU holds `capacity` terms.
    pub fn new(capacity: usize) -> TermReader {
        TermReader {
            cache: Mutex::new(LruCache::new(
                NonZeroUsize::new(capacity.max(1)).expect("non-zero"),
            )),
        }
    }

    /// Encodes a value for a read: lookup only, never inserts.
    pub fn encode(exec: &mut dyn Executor, v: &Value) -> Result<Option<ObjectId>> {
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
                        match sql_lookup(exec, &k)? {
                            Some(id) => Some(ObjectId::from_unsigned(Tag::Iri, id as u64).raw()),
                            None => return Ok(None),
                        }
                    }
                };
                let k = TermKey {
                    tag: spec.tag as u8,
                    lex: spec.lex,
                    dt,
                    lang: spec.lang,
                };
                Ok(sql_lookup(exec, &k)?.map(|id| ObjectId::from_unsigned(spec.tag, id as u64)))
            }
        }
    }

    fn term(&self, exec: &mut dyn Executor, id: i64, use_cache: bool) -> Result<Option<Term>> {
        if use_cache {
            if let Some(t) = self.cache.lock().expect("term cache").get(&id) {
                return Ok(Some(t.clone()));
            }
        }
        let t = load_term(exec, id)?;
        if let (true, Some(t)) = (use_cache, &t) {
            self.cache.lock().expect("term cache").put(id, t.clone());
        }
        Ok(t)
    }

    /// Decodes an id. `use_cache` must be false when reading uncommitted state.
    pub fn decode(&self, exec: &mut dyn Executor, id: ObjectId, use_cache: bool) -> Result<Value> {
        if let Some(v) = codec::decode_inline(id)? {
            return Ok(v);
        }
        let tag = id.tag()?;
        let t = self
            .term(exec, id.unsigned_payload() as i64, use_cache)?
            .filter(|t| t.tag == tag)
            .ok_or_else(|| unknown(id))?;
        let dt = match t.dt {
            Some(dt) => self
                .term(exec, dt.unsigned_payload() as i64, use_cache)?
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
