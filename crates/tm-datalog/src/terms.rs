//! Terms inside a run: every value is an `i64`, the ObjectId raw the store
//! uses for it, so joins compare integers and match stored statements directly.
//!
//! A value the store does not hold (a computed string, an IRI only a head
//! names, an invented node) gets a run-local id: a dictionary tag with a
//! payload at or above [`LOCAL_BASE`], which no stored term id reaches. Those
//! ids never leave the run.

use std::cmp::Ordering;
use std::sync::Mutex;

use rustc_hash::FxHashMap;
use tm_core::codec::{self, Encoded};
use tm_core::store::ReadStore;
use tm_core::vocab::{DL_INF, TM_INTERVAL};
use tm_core::{Allen, ObjectId, Result, Tag, Valid, Value};
use tm_store::dict::TermCache;

use crate::interval::{IvSet, NEG_INF, POS_INF};
use crate::syntax::{BinOp, CmpOp};

/// A term id: an ObjectId raw, or a run-local id.
pub type Id = i64;

/// The first payload of run-local ids (stored term ids are counters far below).
pub const LOCAL_BASE: u64 = 1 << 58;

/// A value of a result: a stored or computed value, or a node a rule invented.
#[derive(Clone, Debug, PartialEq)]
pub enum Term {
    /// A value.
    Value(Value),
    /// A node invented by `exists`, numbered within the run.
    Invented(u64),
}

impl std::fmt::Display for Term {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Term::Value(v) => write!(f, "{v}"),
            Term::Invented(n) => write!(f, "_:invented{n}"),
        }
    }
}

#[derive(Clone, Debug)]
enum LocalVal {
    Value(Value),
    Invented(u64),
}

#[derive(Default)]
struct Local {
    by_value: FxHashMap<String, Id>,
    by_key: FxHashMap<(u32, Vec<Id>), Id>,
    keys: Vec<(u32, Vec<Id>)>,
    vals: Vec<LocalVal>,
}

/// The term table of one run.
pub struct Terms {
    cache: TermCache,
    local: Mutex<Local>,
    decoded: Mutex<FxHashMap<Id, Value>>,
    invented: std::sync::atomic::AtomicU64,
}

impl Default for Terms {
    fn default() -> Self {
        Terms::new()
    }
}

fn value_key(v: &Value) -> String {
    format!("{v:?}")
}

impl Terms {
    /// An empty table.
    pub fn new() -> Terms {
        Terms {
            cache: TermCache::new(4096),
            local: Mutex::new(Local::default()),
            decoded: Mutex::new(FxHashMap::default()),
            invented: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// True for a run-local id.
    pub fn is_local(id: Id) -> bool {
        let o = ObjectId::from_raw(id);
        matches!(
            o.tag(),
            Ok(Tag::Iri | Tag::Str | Tag::LangStr | Tag::Typed | Tag::Double | Tag::Decimal)
        ) && o.unsigned_payload() >= LOCAL_BASE
    }

    /// The id of `v`: the store's if it has it, the inline id, or a run-local one.
    pub fn intern(&self, store: &dyn ReadStore, v: &Value) -> Result<Id> {
        let canon = v.canonical();
        if let Encoded::Inline(id) = codec::encode(&canon) {
            return Ok(id.raw());
        }
        if let Some(id) = tm_store::dict::encode(store, &canon)? {
            return Ok(id.raw());
        }
        let key = value_key(&canon);
        let mut l = self.local.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(&id) = l.by_value.get(&key) {
            return Ok(id);
        }
        let tag = match &canon {
            Value::Iri(_) => Tag::Iri,
            Value::Str(_) => Tag::Str,
            Value::LangStr { .. } => Tag::LangStr,
            Value::Double(_) => Tag::Double,
            Value::Decimal(_) => Tag::Decimal,
            _ => Tag::Typed,
        };
        let id = ObjectId::from_unsigned(tag, LOCAL_BASE + l.vals.len() as u64).raw();
        l.vals.push(LocalVal::Value(canon));
        l.by_value.insert(key, id);
        Ok(id)
    }

    /// The node invented for `(rule, frontier)`, the same id for the same key.
    pub fn invent(&self, rule: u32, frontier: &[Id]) -> Id {
        let mut l = self.local.lock().unwrap_or_else(|p| p.into_inner());
        let key = (rule, frontier.to_vec());
        if let Some(&id) = l.by_key.get(&key) {
            return id;
        }
        let n = self
            .invented
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let id = ObjectId::from_unsigned(Tag::Iri, LOCAL_BASE + l.vals.len() as u64).raw();
        l.vals.push(LocalVal::Invented(n));
        l.keys.push(key.clone());
        l.by_key.insert(key, id);
        id
    }

    /// The `(rule, frontier values)` key node `n` was invented for.
    pub fn invent_key(&self, n: u64) -> Option<(u32, Vec<Id>)> {
        self.local
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .keys
            .get(n as usize)
            .cloned()
    }

    /// The number of nodes invented so far.
    pub fn invented_count(&self) -> u64 {
        self.invented.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// The result term of `id`.
    pub fn term(&self, store: &dyn ReadStore, id: Id) -> Result<Term> {
        if Terms::is_local(id) {
            let payload = ObjectId::from_raw(id).unsigned_payload() - LOCAL_BASE;
            let l = self.local.lock().unwrap_or_else(|p| p.into_inner());
            return Ok(match &l.vals[payload as usize] {
                LocalVal::Value(v) => Term::Value(v.clone()),
                LocalVal::Invented(n) => Term::Invented(*n),
            });
        }
        Ok(Term::Value(self.value(store, id)?))
    }

    /// The value of `id`; an invented node is its placeholder IRI.
    pub fn value(&self, store: &dyn ReadStore, id: Id) -> Result<Value> {
        if let Some(v) = codec::decode_inline(ObjectId::from_raw(id))? {
            return Ok(v);
        }
        if let Some(v) = self
            .decoded
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&id)
        {
            return Ok(v.clone());
        }
        let v = if Terms::is_local(id) {
            match self.term(store, id)? {
                Term::Value(v) => v,
                Term::Invented(n) => Value::Iri(format!("urn:tiramemsu:dl:invented:{n}")),
            }
        } else {
            self.cache.decode(store, ObjectId::from_raw(id), true)?
        };
        self.decoded
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id, v.clone());
        Ok(v)
    }
}

// ---------------------------------------------------------------- values

/// A number view of a value.
fn num(v: &Value) -> Option<f64> {
    match v {
        Value::Int(i) => Some(*i as f64),
        Value::Double(d) => Some(*d),
        Value::Decimal(d) => d.parse().ok(),
        _ => None,
    }
}

/// The lexical form of a string value.
pub fn string(v: &Value) -> Option<&str> {
    match v {
        Value::Str(s) => Some(s),
        Value::LangStr { lex, .. } => Some(lex),
        _ => None,
    }
}

/// An instant view of a value: a date-time's milliseconds, a date's midnight.
fn instant(v: &Value) -> Option<i64> {
    match v {
        Value::DateTime { ms, .. } => Some(*ms),
        Value::Date(d) => Some(d * 86_400_000),
        _ => None,
    }
}

fn kind_rank(v: &Value) -> u8 {
    match v {
        Value::Bool(_) => 0,
        Value::Int(_) | Value::Double(_) | Value::Decimal(_) => 1,
        Value::DateTime { .. } | Value::Date(_) => 2,
        Value::Str(_) | Value::LangStr { .. } => 3,
        Value::Typed { .. } => 4,
        Value::Iri(_) => 5,
        Value::Node(_) | Value::BNode(_) => 6,
        Value::Stmt(_) => 7,
        Value::Tx(_) => 8,
    }
}

/// The total order of results: by kind, then within a kind.
pub fn total_cmp(a: &Value, b: &Value) -> Ordering {
    let (ra, rb) = (kind_rank(a), kind_rank(b));
    if ra != rb {
        return ra.cmp(&rb);
    }
    if let (Some(x), Some(y)) = (num(a), num(b)) {
        return x
            .total_cmp(&y)
            .then_with(|| format!("{a:?}").cmp(&format!("{b:?}")));
    }
    if let (Some(x), Some(y)) = (instant(a), instant(b)) {
        return x.cmp(&y);
    }
    match (a, b) {
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        (Value::Iri(x), Value::Iri(y)) => x.cmp(y),
        (Value::Node(x), Value::Node(y)) | (Value::BNode(x), Value::BNode(y)) => x.cmp(y),
        (Value::Stmt(x), Value::Stmt(y)) => x.cmp(y),
        (Value::Tx(x), Value::Tx(y)) => x.cmp(y),
        _ => match (string(a), string(b)) {
            (Some(x), Some(y)) => x
                .cmp(y)
                .then_with(|| format!("{a:?}").cmp(&format!("{b:?}"))),
            _ => format!("{a:?}").cmp(&format!("{b:?}")),
        },
    }
}

/// The order a comparison builtin uses: numbers, instants and strings among
/// themselves; `None` when the kinds are not comparable.
pub fn compare(a: &Value, b: &Value) -> Option<Ordering> {
    if let (Some(x), Some(y)) = (num(a), num(b)) {
        return x.partial_cmp(&y);
    }
    if let (Some(x), Some(y)) = (instant(a), instant(b)) {
        return Some(x.cmp(&y));
    }
    if let (Some(x), Some(y)) = (string(a), string(b)) {
        return Some(x.cmp(y));
    }
    match (a, b) {
        (Value::Bool(x), Value::Bool(y)) => Some(x.cmp(y)),
        _ => None,
    }
}

/// `a op b` on values (`=` and `!=` are decided on ids by the caller).
pub fn cmp_values(a: &Value, op: CmpOp, b: &Value) -> bool {
    let Some(o) = compare(a, b) else {
        return false;
    };
    match op {
        CmpOp::Eq => o == Ordering::Equal,
        CmpOp::Ne => o != Ordering::Equal,
        CmpOp::Lt => o == Ordering::Less,
        CmpOp::Le => o != Ordering::Greater,
        CmpOp::Gt => o == Ordering::Greater,
        CmpOp::Ge => o != Ordering::Less,
    }
}

fn datetime(ms: i64) -> Value {
    Value::DateTime { ms, tz: Some(0) }
}

/// `a op b`; `None` for values of the wrong kind or an overflow.
// @tg: implements:: [[openspec:datalog-language#Builtins]] {since: "2026-10-28"}
pub fn arith(a: &Value, op: BinOp, b: &Value) -> Option<Value> {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => match op {
            BinOp::Add => x.checked_add(*y).map(Value::Int),
            BinOp::Sub => x.checked_sub(*y).map(Value::Int),
            BinOp::Mul => x.checked_mul(*y).map(Value::Int),
            BinOp::Div if *y == 0 => None,
            BinOp::Div if x % y == 0 => Some(Value::Int(x / y)),
            BinOp::Div => Some(Value::Double(*x as f64 / *y as f64)),
            BinOp::Rem if *y == 0 => None,
            BinOp::Rem => Some(Value::Int(x % y)),
        },
        _ => {
            // instants: instant ± duration (ms), instant − instant
            if let (Some(t), Value::Int(d)) = (instant(a), b) {
                return match op {
                    BinOp::Add => t.checked_add(*d).map(datetime),
                    BinOp::Sub => t.checked_sub(*d).map(datetime),
                    _ => None,
                };
            }
            if let (Value::Int(d), Some(t), BinOp::Add) = (a, instant(b), op) {
                return t.checked_add(*d).map(datetime);
            }
            if let (Some(x), Some(y), BinOp::Sub) = (instant(a), instant(b), op) {
                return x.checked_sub(y).map(Value::Int);
            }
            let (x, y) = (num(a)?, num(b)?);
            let r = match op {
                BinOp::Add => x + y,
                BinOp::Sub => x - y,
                BinOp::Mul => x * y,
                BinOp::Div if y == 0.0 => return None,
                BinOp::Div => x / y,
                BinOp::Rem if y == 0.0 => return None,
                BinOp::Rem => x % y,
            };
            r.is_finite().then_some(Value::Double(r))
        }
    }
}

/// A function builtin; `None` when the arguments are of the wrong kind.
pub fn call(name: &str, args: &[Value], now: i64) -> Option<Value> {
    match (name, args) {
        ("now", []) => Some(datetime(now)),
        ("concat", parts) => {
            let mut s = String::new();
            for p in parts {
                s.push_str(string(p)?);
            }
            Some(Value::Str(s))
        }
        ("strlen", [a]) => Some(Value::Int(string(a)?.chars().count() as i64)),
        ("lower", [a]) => Some(Value::Str(string(a)?.to_lowercase())),
        ("upper", [a]) => Some(Value::Str(string(a)?.to_uppercase())),
        _ => None,
    }
}

/// The names `call` knows.
pub const FUNCTIONS: [&str; 5] = ["now", "concat", "strlen", "lower", "upper"];

// ---------------------------------------------------------------- intervals

/// An interval value `"<from>/<to>"^^tm:Interval` (`..` for an open end).
pub fn interval_value(v: Valid) -> Value {
    let end = |x: Option<i64>| x.map_or_else(|| "..".to_string(), |x| x.to_string());
    Value::Typed {
        lex: format!("{}/{}", end(v.from), end(v.to)),
        datatype: TM_INTERVAL.to_string(),
    }
}

/// The interval an operand stands for: an interval value or an instant
/// (`[t, t+1)`); statements are resolved by the caller.
pub fn as_interval(v: &Value) -> Option<Valid> {
    match v {
        Value::Typed { lex, datatype } if datatype == TM_INTERVAL => {
            let (a, b) = lex.split_once('/')?;
            let end = |s: &str| -> Option<Option<i64>> {
                if s == ".." {
                    Some(None)
                } else {
                    s.parse().ok().map(Some)
                }
            };
            Some(Valid {
                from: end(a)?,
                to: end(b)?,
            })
        }
        _ => instant(v).map(|t| Valid::between(t, t + 1)),
    }
}

/// The value of an interval end: a date-time, or `dl:inf` when open.
pub fn end_value(x: Option<i64>) -> Value {
    x.map_or_else(|| Value::Iri(DL_INF.to_string()), datetime)
}

/// `rel(a, b)` for an Allen relation name.
// @tg: implements:: [[openspec:datalog-temporal#Allen relations]] {since: "2026-10-28"}
pub fn allen(name: &str, a: Valid, b: Valid) -> Option<bool> {
    let r = Allen::from_name(name).or_else(|| {
        // snake_case aliases: `finished_by`, `overlapped_by`, ...
        let camel: String = name
            .split('_')
            .enumerate()
            .map(|(i, p)| {
                if i == 0 {
                    p.to_string()
                } else {
                    let mut c = p.chars();
                    c.next()
                        .map(|f| f.to_ascii_uppercase().to_string() + c.as_str())
                        .unwrap_or_default()
                }
            })
            .collect();
        Allen::from_name(&camel)
    });
    match (name, r) {
        ("intersects", _) => Some(a.intersects(&b)),
        (_, Some(r)) => Some(a.relate(&b) == r),
        _ => None,
    }
}

/// True when `name` is an Allen relation or `intersects`.
pub fn is_allen(name: &str) -> bool {
    allen(name, Valid::ALWAYS, Valid::ALWAYS).is_some()
}

/// An interval set as one interval value per maximal interval.
pub fn ivset_bounds(s: &IvSet) -> Vec<(Option<i64>, Option<i64>)> {
    s.intervals()
        .iter()
        .map(|&(a, b)| ((a != NEG_INF).then_some(a), (b != POS_INF).then_some(b)))
        .collect()
}
