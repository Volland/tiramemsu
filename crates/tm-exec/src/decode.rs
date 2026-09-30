//! Decoding result cells into typed values through a bounded LRU term cache
//! (design D13, `lat.md/storage#Term Dictionary`).

use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::Mutex;

use lru::LruCache;
use tm_core::term::load_term;
use tm_core::{codec, Executor, ObjectId, Result, SqlValue, Value};

use crate::error::invalid;
use crate::path::row::{virtual_pred_iri, Path};
use crate::plan::analyze::{Dom, VClass};
use crate::result::{ExecStats, ResultValue};

/// The shared `id → term` cache of one database, used by every reader. Terms are
/// immutable and never deleted, so entries never go stale.
#[derive(Debug)]
pub struct TermCache {
    inner: Mutex<LruCache<i64, Value>>,
}

impl TermCache {
    /// A cache holding at most `capacity` terms (at least 1).
    pub fn new(capacity: usize) -> TermCache {
        TermCache {
            inner: Mutex::new(LruCache::new(
                NonZeroUsize::new(capacity.max(1)).expect("non-zero"),
            )),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, LruCache<i64, Value>> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The cached term of `id` (marks it recently used).
    pub fn get(&self, id: ObjectId) -> Option<Value> {
        self.lock().get(&id.raw()).cloned()
    }

    /// The cached term of `id` without touching the LRU order.
    pub fn peek(&self, id: ObjectId) -> Option<Value> {
        self.lock().peek(&id.raw()).cloned()
    }

    /// Caches a committed term.
    pub fn put(&self, id: ObjectId, v: Value) {
        self.lock().put(id.raw(), v);
    }

    /// Number of cached terms.
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    /// True when empty.
    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    /// True when `id` is cached.
    pub fn contains(&self, id: ObjectId) -> bool {
        self.lock().contains(&id.raw())
    }

    /// The capacity.
    pub fn capacity(&self) -> usize {
        self.lock().cap().get()
    }
}

/// Where decoded terms are cached.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CacheMode {
    /// A reader on committed state: read and fill the shared cache.
    Shared,
    /// The writer inside a speculation: a private map first, the shared cache
    /// read-only, so speculative terms never reach the shared cache.
    Scoped,
}

/// Decodes the cells of one query on the connection that ran it.
pub struct Decoder<'a> {
    cache: &'a TermCache,
    mode: CacheMode,
    scoped: HashMap<i64, Value>,
    synthetic: &'a HashMap<i64, Value>,
    /// Counters reported in the result.
    pub stats: ExecStats,
}

impl<'a> Decoder<'a> {
    /// A decoder over `cache` in `mode`; `synthetic` maps plan-local ids of
    /// constants that are not in the dictionary back to their values.
    pub fn new(
        cache: &'a TermCache,
        mode: CacheMode,
        synthetic: &'a HashMap<i64, Value>,
    ) -> Decoder<'a> {
        Decoder {
            cache,
            mode,
            scoped: HashMap::new(),
            synthetic,
            stats: ExecStats::default(),
        }
    }

    /// Decodes one ObjectId.
    pub fn term(&mut self, exec: &mut dyn Executor, id: ObjectId) -> Result<Value> {
        if let Some(v) = codec::decode_inline(id)? {
            return Ok(v);
        }
        if let Some(v) = self.synthetic.get(&id.raw()) {
            return Ok(v.clone());
        }
        if self.mode == CacheMode::Scoped {
            if let Some(v) = self.scoped.get(&id.raw()) {
                self.stats.cache_hits += 1;
                return Ok(v.clone());
            }
        }
        if let Some(v) = self.cache.get(id) {
            self.stats.cache_hits += 1;
            return Ok(v);
        }
        let tag = id.tag()?;
        self.stats.dictionary_reads += 1;
        let t = load_term(exec, id.unsigned_payload() as i64)?
            .filter(|t| t.tag == tag)
            .ok_or_else(|| invalid(format!("unknown term id {id:?}")))?;
        let dt = match t.dt {
            Some(dt) => match self.term(exec, dt)? {
                Value::Iri(s) => Some(s),
                _ => None,
            },
            None => None,
        };
        let v = codec::value_from_term(t.tag, t.lex, dt, t.lang)?;
        match self.mode {
            CacheMode::Shared => self.cache.put(id, v.clone()),
            CacheMode::Scoped => {
                self.scoped.insert(id.raw(), v.clone());
            }
        }
        Ok(v)
    }

    /// Decodes one cell of static domain `dom`.
    pub fn cell(
        &mut self,
        exec: &mut dyn Executor,
        v: &SqlValue,
        dom: &Dom,
    ) -> Result<Option<ResultValue>> {
        if v.is_null() {
            return Ok(None);
        }
        Ok(Some(match dom {
            Dom::Term => match v {
                SqlValue::Integer(i) => ResultValue::Term(self.term(exec, ObjectId::from_raw(*i))?),
                other => return Err(invalid(format!("expected a term id, got {other:?}"))),
            },
            Dom::TermOrList => match v {
                SqlValue::Integer(i) => ResultValue::Term(self.term(exec, ObjectId::from_raw(*i))?),
                SqlValue::Text(t) => self.list(exec, t, &Dom::Term)?,
                other => return Err(invalid(format!("expected a term or list, got {other:?}"))),
            },
            Dom::List(elem) => match v {
                SqlValue::Text(t) => self.list(exec, t, elem)?,
                other => return Err(invalid(format!("expected a list, got {other:?}"))),
            },
            Dom::PathJson { reversed } => match v {
                SqlValue::Text(t) => ResultValue::Term(Value::Str(self.path(exec, t, *reversed)?)),
                other => return Err(invalid(format!("expected path_json, got {other:?}"))),
            },
            Dom::Computed(c) => return Ok(computed(v, *c).map(ResultValue::Term)),
        }))
    }

    /// Decodes a `path_json` cell into a self-describing text:
    /// `{"nodes":[<lexical>,…],"edges":[{"eid":<lexical>,"p":<lexical>,"dir":"out"|"in"},…]}`
    /// with nodes and statements in their lexical (skolem IRI) forms, virtual hops
    /// named by their `sys:` IRI, and the path read start to end of the pattern
    /// (`reversed` calls are flipped back).
    fn path(&mut self, exec: &mut dyn Executor, text: &str, reversed: bool) -> Result<String> {
        let mut p =
            Path::from_json(text).ok_or_else(|| invalid(format!("bad path_json {text:?}")))?;
        if reversed {
            p = p.reversed();
        }
        let mut out = String::from("{\"nodes\":[");
        for (i, n) in p.nodes.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(&json_string(&self.term(exec, *n)?.lexical()));
        }
        out.push_str("],\"edges\":[");
        for (i, h) in p.hops.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            let pred = match virtual_pred_iri(h.pred) {
                Some(iri) => iri.to_string(),
                None => self.term(exec, h.pred)?.lexical(),
            };
            out.push_str(&format!(
                "{{\"eid\":{},\"p\":{},\"dir\":\"{}\"}}",
                json_string(&self.term(exec, h.eid)?.lexical()),
                json_string(&pred),
                if h.dir == crate::path::row::Dir::Out {
                    "out"
                } else {
                    "in"
                }
            ));
        }
        out.push_str("]}");
        Ok(out)
    }

    fn list(&mut self, exec: &mut dyn Executor, text: &str, elem: &Dom) -> Result<ResultValue> {
        let json = parse_json(text).ok_or_else(|| invalid(format!("bad list {text:?}")))?;
        let Json::Arr(items) = json else {
            return Err(invalid(format!("not a list: {text:?}")));
        };
        let mut out = Vec::with_capacity(items.len());
        for it in items {
            let sv = match it {
                Json::Null => SqlValue::Null,
                Json::Int(i) => SqlValue::Integer(i),
                Json::Real(r) => SqlValue::Real(r),
                Json::Str(s) => SqlValue::Text(s),
                Json::Arr(_) => SqlValue::Null,
            };
            out.push(self.cell(exec, &sv, elem)?);
        }
        Ok(ResultValue::List(out))
    }
}

/// Decodes a computed native value of class `c`.
pub fn computed(v: &SqlValue, c: VClass) -> Option<Value> {
    Some(match (c, v) {
        (_, SqlValue::Null) => return None,
        (VClass::Bool, SqlValue::Integer(i)) => Value::Bool(*i != 0),
        (VClass::Bool, SqlValue::Real(x)) => Value::Bool(*x != 0.0),
        (VClass::Iri, SqlValue::Text(s)) => Value::Iri(s.clone()),
        (VClass::Lit, SqlValue::Text(s)) => crate::udf_fn::unboxed_value(s),
        (VClass::Double, SqlValue::Integer(i)) => Value::Double(*i as f64),
        (_, SqlValue::Integer(i)) => Value::Int(*i),
        (_, SqlValue::Real(x)) => Value::Double(*x),
        (_, SqlValue::Text(s)) => Value::Str(s.clone()),
        (_, SqlValue::Blob(_) | SqlValue::IntArray(_)) => return None,
    })
}

/// A minimal JSON value (lists produced by `json_group_array` / `json_array`).
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    /// `null`.
    Null,
    /// An integer.
    Int(i64),
    /// A real.
    Real(f64),
    /// A string.
    Str(String),
    /// An array.
    Arr(Vec<Json>),
}

/// Parses the JSON that SQLite's JSON functions produce for lists.
pub fn parse_json(text: &str) -> Option<Json> {
    let mut p = JsonParser {
        s: text.as_bytes(),
        i: 0,
    };
    let v = p.value()?;
    p.ws();
    (p.i == p.s.len()).then_some(v)
}

struct JsonParser<'a> {
    s: &'a [u8],
    i: usize,
}

impl JsonParser<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn value(&mut self) -> Option<Json> {
        self.ws();
        match *self.s.get(self.i)? {
            b'[' => {
                self.i += 1;
                let mut out = Vec::new();
                self.ws();
                if self.s.get(self.i) == Some(&b']') {
                    self.i += 1;
                    return Some(Json::Arr(out));
                }
                loop {
                    out.push(self.value()?);
                    self.ws();
                    match *self.s.get(self.i)? {
                        b',' => self.i += 1,
                        b']' => {
                            self.i += 1;
                            return Some(Json::Arr(out));
                        }
                        _ => return None,
                    }
                }
            }
            b'"' => {
                self.i += 1;
                let mut buf = Vec::new();
                loop {
                    let c = *self.s.get(self.i)?;
                    self.i += 1;
                    match c {
                        b'"' => break,
                        b'\\' => {
                            let e = *self.s.get(self.i)?;
                            self.i += 1;
                            match e {
                                b'n' => buf.push(b'\n'),
                                b't' => buf.push(b'\t'),
                                b'r' => buf.push(b'\r'),
                                b'b' => buf.push(8),
                                b'f' => buf.push(12),
                                b'u' => {
                                    let h = std::str::from_utf8(self.s.get(self.i..self.i + 4)?)
                                        .ok()?;
                                    self.i += 4;
                                    let mut cp = u32::from_str_radix(h, 16).ok()?;
                                    if (0xD800..0xDC00).contains(&cp)
                                        && self.s.get(self.i..self.i + 2) == Some(b"\\u")
                                    {
                                        let h2 = std::str::from_utf8(
                                            self.s.get(self.i + 2..self.i + 6)?,
                                        )
                                        .ok()?;
                                        let lo = u32::from_str_radix(h2, 16).ok()?;
                                        self.i += 6;
                                        cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                                    }
                                    let ch = char::from_u32(cp)?;
                                    let mut b = [0; 4];
                                    buf.extend_from_slice(ch.encode_utf8(&mut b).as_bytes());
                                }
                                other => buf.push(other),
                            }
                        }
                        other => buf.push(other),
                    }
                }
                String::from_utf8(buf).ok().map(Json::Str)
            }
            b'n' => {
                if self.s.get(self.i..self.i + 4) == Some(b"null") {
                    self.i += 4;
                    Some(Json::Null)
                } else {
                    None
                }
            }
            _ => {
                let st = self.i;
                while self.i < self.s.len()
                    && matches!(
                        self.s[self.i],
                        b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9'
                    )
                {
                    self.i += 1;
                }
                let t = std::str::from_utf8(&self.s[st..self.i]).ok()?;
                if let Ok(i) = t.parse::<i64>() {
                    Some(Json::Int(i))
                } else {
                    t.parse::<f64>().ok().map(Json::Real)
                }
            }
        }
    }
}

/// A JSON string literal.
pub fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_lists() {
        assert_eq!(
            parse_json("[1,-2,3.5,\"a\\\"b\",null]"),
            Some(Json::Arr(vec![
                Json::Int(1),
                Json::Int(-2),
                Json::Real(3.5),
                Json::Str("a\"b".into()),
                Json::Null
            ]))
        );
        assert_eq!(parse_json("[]"), Some(Json::Arr(vec![])));
        assert_eq!(parse_json("\"\\u00e9\""), Some(Json::Str("é".into())));
        assert_eq!(parse_json("[1,"), None);
    }

    #[test]
    fn capacity_is_respected() {
        let c = TermCache::new(10);
        for i in 0..50 {
            c.put(ObjectId::from_raw(i << 4), Value::Int(i));
        }
        assert_eq!(c.len(), 10);
        assert_eq!(c.capacity(), 10);
    }
}
