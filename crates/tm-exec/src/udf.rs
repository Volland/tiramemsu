//! Pure, deterministic SQL helper functions, defined host-neutrally and registered
//! through the host's hooks on every connection (design D9).
//!
//! | Function | Purpose |
//! |---|---|
//! | `tm_kind(id)` | comparison class rank of a term |
//! | `tm_num(id, num)` | numeric value |
//! | `tm_str(id, lex)` | lexical form (STR) |
//! | `tm_lang(id, lang)` / `tm_datatype(id, dtlex)` | LANG / DATATYPE |
//! | `tm_sortkey(id, lex, num, lang)` | memcmp-ordered value key |
//! | `tm_vkind(v, class)` / `tm_vkey(v, class)` | the same for computed values |
//! | `tm_ucase(s)` / `tm_lcase(s)` | Unicode case mapping |
//! | `regexp(pattern, text)` / `tm_regex(text, pattern, flags)` | REGEX |
//! | `tm_min_by(v, key)` / `tm_max_by(v, key)` | aggregates: the value with the smallest / largest key |

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use tm_core::{
    codec, AggregateFunction, AggregateState, ObjectId, ScalarFunction, SqlValue, Tag, Value,
};

/// Class rank: blank and anonymous nodes.
pub const K_NODE: i64 = 1;
/// Class rank: IRIs.
pub const K_IRI: i64 = 2;
/// Class rank: statements.
pub const K_STMT: i64 = 3;
/// Class rank: transactions.
pub const K_TX: i64 = 4;
/// Class rank: numbers (integer, double, decimal).
pub const K_NUM: i64 = 5;
/// Class rank: booleans.
pub const K_BOOL: i64 = 6;
/// Class rank: date-times.
pub const K_DATETIME: i64 = 7;
/// Class rank: dates.
pub const K_DATE: i64 = 8;
/// Class rank: plain strings.
pub const K_STR: i64 = 9;
/// Class rank: language-tagged strings.
pub const K_LANG: i64 = 10;
/// Class rank: other typed literals.
pub const K_TYPED: i64 = 11;

/// The class rank of an ObjectId tag (the fixed cross-kind order of the spec).
pub fn kind_of_tag(tag: u8) -> Option<i64> {
    Some(match tag {
        1 | 2 => K_NODE,
        0 => K_IRI,
        3 => K_STMT,
        4 => K_TX,
        5 | 13 | 14 => K_NUM,
        6 => K_BOOL,
        7 => K_DATETIME,
        8 => K_DATE,
        9 | 10 => K_STR,
        11 => K_LANG,
        12 => K_TYPED,
        _ => return None,
    })
}

/// The class rank of a value.
pub fn kind_of_value(v: &Value) -> i64 {
    match v.canonical() {
        Value::Node(_) | Value::BNode(_) => K_NODE,
        Value::Iri(_) => K_IRI,
        Value::Stmt(_) => K_STMT,
        Value::Tx(_) => K_TX,
        Value::Int(_) | Value::Double(_) | Value::Decimal(_) => K_NUM,
        Value::Bool(_) => K_BOOL,
        Value::DateTime { .. } => K_DATETIME,
        Value::Date(_) => K_DATE,
        Value::Str(_) => K_STR,
        Value::LangStr { .. } => K_LANG,
        Value::Typed { .. } => K_TYPED,
    }
}

/// Order-preserving bytes of an `i64` (big-endian with the sign bit flipped).
pub fn i64_key(n: i64) -> [u8; 8] {
    ((n as u64) ^ (1 << 63)).to_be_bytes()
}

/// Order-preserving bytes of an `f64` (NaN sorts after +inf).
pub fn f64_key(x: f64) -> [u8; 8] {
    if x.is_nan() {
        return [0xff; 8];
    }
    let x = if x == 0.0 { 0.0 } else { x };
    let b = x.to_bits();
    let k = if b >> 63 == 1 { !b } else { b | (1 << 63) };
    k.to_be_bytes()
}

fn num_key(x: f64, tie: i64) -> Vec<u8> {
    let mut k = vec![K_NUM as u8];
    k.extend_from_slice(&f64_key(x));
    k.extend_from_slice(&i64_key(tie));
    k
}

fn floor_i64(x: f64) -> i64 {
    if x.is_finite() {
        x.floor().clamp(i64::MIN as f64, i64::MAX as f64) as i64
    } else if x > 0.0 {
        i64::MAX
    } else {
        i64::MIN
    }
}

fn str_key(rank: i64, s: &str) -> Vec<u8> {
    let mut k = vec![rank as u8];
    k.extend_from_slice(s.as_bytes());
    k
}

/// The sort key of a stored term: `[class rank][order-preserving payload]`.
/// Dictionary tags need their `lex` / `num` / `lang` columns; without them the key
/// is `None` (unknown).
pub fn sort_key(
    id: i64,
    lex: Option<&str>,
    num: Option<f64>,
    lang: Option<&str>,
) -> Option<Vec<u8>> {
    let oid = ObjectId::from_raw(id);
    let tag = oid.tag_bits();
    let rank = kind_of_tag(tag)?;
    Some(match tag {
        0 => str_key(K_IRI, lex?),
        1 | 2 => {
            let mut k = vec![K_NODE as u8, tag];
            k.extend_from_slice(&oid.unsigned_payload().to_be_bytes());
            k
        }
        3 | 4 => {
            let mut k = vec![rank as u8];
            k.extend_from_slice(&oid.unsigned_payload().to_be_bytes());
            k
        }
        5 => {
            let p = oid.signed_payload();
            num_key(p as f64, p)
        }
        6 => vec![K_BOOL as u8, oid.unsigned_payload() as u8],
        7 => {
            let mut k = vec![K_DATETIME as u8];
            k.extend_from_slice(&i64_key(oid.instant()));
            k
        }
        8 => {
            let mut k = vec![K_DATE as u8];
            k.extend_from_slice(&i64_key(oid.signed_payload()));
            k
        }
        9 => match codec::decode_inline(oid).ok()?? {
            Value::Str(s) => str_key(K_STR, &s),
            _ => return None,
        },
        10 => str_key(K_STR, lex?),
        11 => {
            let mut k = str_key(K_LANG, lex?);
            k.push(0);
            k.extend_from_slice(lang.unwrap_or("").as_bytes());
            k
        }
        12 => str_key(K_TYPED, lex?),
        13 | 14 => match num {
            Some(x) => num_key(x, floor_i64(x)),
            None => {
                let mut k = vec![K_NUM as u8];
                k.extend_from_slice(&[0xff; 16]);
                k
            }
        },
        _ => return None,
    })
}

/// The sort key of a value (equal to [`sort_key`] of its ObjectId).
pub fn value_key(v: &Value) -> Vec<u8> {
    match v.canonical() {
        Value::Iri(s) => str_key(K_IRI, &s),
        Value::Node(n) => {
            let mut k = vec![K_NODE as u8, Tag::Node as u8];
            k.extend_from_slice(&n.to_be_bytes());
            k
        }
        Value::BNode(n) => {
            let mut k = vec![K_NODE as u8, Tag::BNode as u8];
            k.extend_from_slice(&n.to_be_bytes());
            k
        }
        Value::Stmt(e) => {
            let mut k = vec![K_STMT as u8];
            k.extend_from_slice(&e.n().to_be_bytes());
            k
        }
        Value::Tx(t) => {
            let mut k = vec![K_TX as u8];
            k.extend_from_slice(&t.0.to_be_bytes());
            k
        }
        Value::Int(i) => num_key(i as f64, i),
        Value::Double(x) => num_key(x, floor_i64(x)),
        Value::Decimal(s) => {
            let x = s.parse::<f64>().unwrap_or(f64::NAN);
            if x.is_nan() {
                let mut k = vec![K_NUM as u8];
                k.extend_from_slice(&[0xff; 16]);
                k
            } else {
                num_key(x, floor_i64(x))
            }
        }
        Value::Bool(b) => vec![K_BOOL as u8, b as u8],
        Value::DateTime { ms, .. } => {
            let mut k = vec![K_DATETIME as u8];
            k.extend_from_slice(&i64_key(ms));
            k
        }
        Value::Date(d) => {
            let mut k = vec![K_DATE as u8];
            k.extend_from_slice(&i64_key(d));
            k
        }
        Value::Str(s) => str_key(K_STR, &s),
        Value::LangStr { lex, lang } => {
            let mut k = str_key(K_LANG, &lex);
            k.push(0);
            k.extend_from_slice(lang.as_bytes());
            k
        }
        Value::Typed { lex, .. } => str_key(K_TYPED, &lex),
    }
}

/// Class codes of computed values (the `class` argument of `tm_vkey`).
pub mod class {
    /// Numbers (INTEGER or REAL).
    pub const NUM: i64 = super::K_NUM;
    /// Booleans (0/1).
    pub const BOOL: i64 = super::K_BOOL;
    /// Plain strings (TEXT).
    pub const STR: i64 = super::K_STR;
    /// IRIs as text.
    pub const IRI: i64 = super::K_IRI;
    /// Any: decided by the SQLite type (numbers or text).
    pub const DYNAMIC: i64 = 0;
}

/// The class rank of a computed value of static class `class`.
pub fn vkind(v: &SqlValue, class: i64) -> Option<i64> {
    match v {
        SqlValue::Null => None,
        _ if class != class::DYNAMIC => Some(class),
        SqlValue::Integer(_) | SqlValue::Real(_) => Some(K_NUM),
        SqlValue::Text(_) => Some(K_STR),
        SqlValue::Blob(_) | SqlValue::IntArray(_) => None,
    }
}

/// The sort key of a computed value.
pub fn vkey(v: &SqlValue, class: i64) -> Option<Vec<u8>> {
    let k = vkind(v, class)?;
    Some(match (k, v) {
        (K_BOOL, SqlValue::Integer(i)) => vec![K_BOOL as u8, (*i != 0) as u8],
        (K_NUM, SqlValue::Integer(i)) => num_key(*i as f64, *i),
        (K_NUM, SqlValue::Real(x)) => num_key(*x, floor_i64(*x)),
        (K_STR | K_IRI, SqlValue::Text(s)) => str_key(k, s),
        (K_NUM, SqlValue::Text(s)) => {
            let x = s.parse::<f64>().ok()?;
            num_key(x, floor_i64(x))
        }
        _ => return None,
    })
}

pub(crate) fn arg_i64(a: &[SqlValue], i: usize) -> Option<i64> {
    a.get(i).and_then(SqlValue::as_i64)
}

pub(crate) fn arg_str(a: &[SqlValue], i: usize) -> Option<&str> {
    a.get(i).and_then(SqlValue::as_str)
}

pub(crate) fn arg_f64(a: &[SqlValue], i: usize) -> Option<f64> {
    match a.get(i) {
        Some(SqlValue::Real(x)) => Some(*x),
        Some(SqlValue::Integer(i)) => Some(*i as f64),
        _ => None,
    }
}

pub(crate) fn opt<T>(v: Option<T>, f: impl FnOnce(T) -> SqlValue) -> SqlValue {
    v.map_or(SqlValue::Null, f)
}

/// The lexical form of a term (`STR`): dictionary tags use `lex`.
pub fn lexical(id: i64, lex: Option<&str>) -> Option<String> {
    let oid = ObjectId::from_raw(id);
    let tag = oid.tag().ok()?;
    if tag.is_dictionary() {
        return lex.map(str::to_string);
    }
    codec::decode_inline(oid).ok()?.map(|v| v.lexical())
}

fn datatype_iri(id: i64, dtlex: Option<&str>) -> Option<String> {
    use tm_core::vocab::*;
    let oid = ObjectId::from_raw(id);
    Some(
        match oid.tag().ok()? {
            Tag::Int => XSD_INTEGER,
            Tag::Bool => XSD_BOOLEAN,
            Tag::DateTime => XSD_DATETIME,
            Tag::Date => XSD_DATE,
            Tag::ShortStr | Tag::Str => XSD_STRING,
            Tag::LangStr => RDF_LANGSTRING,
            Tag::Double => XSD_DOUBLE,
            Tag::Decimal => XSD_DECIMAL,
            Tag::Typed => return dtlex.map(str::to_string),
            _ => return None,
        }
        .to_string(),
    )
}

thread_local! {
    static REGEX_CACHE: RefCell<HashMap<(String, String), Option<regex::Regex>>> =
        RefCell::new(HashMap::new());
}

/// SPARQL `REGEX(text, pattern, flags)`; `None` for an invalid pattern.
pub fn regex_match(text: &str, pattern: &str, flags: &str) -> Option<bool> {
    REGEX_CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() > 256 {
            c.clear();
        }
        let re = c
            .entry((pattern.to_string(), flags.to_string()))
            .or_insert_with(|| {
                let mut b = regex::RegexBuilder::new(pattern);
                for f in flags.chars() {
                    match f {
                        'i' => {
                            b.case_insensitive(true);
                        }
                        's' => {
                            b.dot_matches_new_line(true);
                        }
                        'm' => {
                            b.multi_line(true);
                        }
                        'x' => {
                            b.ignore_whitespace(true);
                        }
                        _ => {}
                    }
                }
                b.build().ok()
            });
        re.as_ref().map(|r| r.is_match(text))
    })
}

pub(crate) fn scalar(
    name: &str,
    n_args: i32,
    f: impl Fn(&[SqlValue]) -> Result<SqlValue, String> + Send + Sync + 'static,
) -> ScalarFunction {
    ScalarFunction {
        name: name.to_string(),
        n_args,
        deterministic: true,
        func: Arc::new(f),
    }
}

/// Every scalar helper function.
pub fn scalar_functions() -> Vec<ScalarFunction> {
    let mut all = core_functions();
    all.extend(crate::udf_fn::functions());
    all
}

fn core_functions() -> Vec<ScalarFunction> {
    vec![
        scalar("tm_kind", 1, |a| {
            Ok(opt(arg_i64(a, 0), |id| {
                opt(kind_of_tag((id & 15) as u8), SqlValue::Integer)
            }))
        }),
        scalar("tm_num", 2, |a| {
            Ok(opt(arg_i64(a, 0), |id| match id & 15 {
                5 => SqlValue::Integer(id >> 4),
                13 | 14 => opt(arg_f64(a, 1), SqlValue::Real),
                _ => SqlValue::Null,
            }))
        }),
        scalar("tm_str", 2, |a| {
            Ok(opt(arg_i64(a, 0), |id| {
                opt(lexical(id, arg_str(a, 1)), SqlValue::Text)
            }))
        }),
        scalar("tm_lang", 2, |a| {
            Ok(opt(arg_i64(a, 0), |id| match id & 15 {
                11 => opt(arg_str(a, 1), |s| SqlValue::Text(s.to_string())),
                5..=14 => SqlValue::Text(String::new()),
                _ => SqlValue::Null,
            }))
        }),
        scalar("tm_datatype", 2, |a| {
            Ok(opt(arg_i64(a, 0), |id| {
                opt(datatype_iri(id, arg_str(a, 1)), SqlValue::Text)
            }))
        }),
        scalar("tm_sortkey", 4, |a| {
            Ok(opt(arg_i64(a, 0), |id| {
                opt(
                    sort_key(id, arg_str(a, 1), arg_f64(a, 2), arg_str(a, 3)),
                    SqlValue::Blob,
                )
            }))
        }),
        scalar("tm_vkind", 2, |a| {
            let class = arg_i64(a, 1).unwrap_or(0);
            Ok(opt(
                a.first().and_then(|v| vkind(v, class)),
                SqlValue::Integer,
            ))
        }),
        scalar("tm_vkey", 2, |a| {
            let class = arg_i64(a, 1).unwrap_or(0);
            Ok(opt(a.first().and_then(|v| vkey(v, class)), SqlValue::Blob))
        }),
        scalar("tm_ucase", 1, |a| {
            Ok(opt(arg_str(a, 0), |s| SqlValue::Text(s.to_uppercase())))
        }),
        scalar("tm_lcase", 1, |a| {
            Ok(opt(arg_str(a, 0), |s| SqlValue::Text(s.to_lowercase())))
        }),
        scalar("regexp", 2, |a| {
            Ok(match (arg_str(a, 0), arg_str(a, 1)) {
                (Some(p), Some(t)) => opt(regex_match(t, p, ""), |b| SqlValue::Integer(b as i64)),
                _ => SqlValue::Null,
            })
        }),
        scalar("tm_regex", 3, |a| {
            Ok(match (arg_str(a, 0), arg_str(a, 1)) {
                (Some(t), Some(p)) => opt(regex_match(t, p, arg_str(a, 2).unwrap_or("")), |b| {
                    SqlValue::Integer(b as i64)
                }),
                _ => SqlValue::Null,
            })
        }),
    ]
}

struct ByKey {
    max: bool,
    best: Option<(Vec<u8>, SqlValue)>,
}

impl AggregateState for ByKey {
    fn step(&mut self, args: &[SqlValue]) -> Result<(), String> {
        let (Some(v), Some(SqlValue::Blob(k))) = (args.first(), args.get(1)) else {
            return Ok(());
        };
        if v.is_null() {
            return Ok(());
        }
        let better = match &self.best {
            None => true,
            Some((b, _)) => {
                if self.max {
                    k > b
                } else {
                    k < b
                }
            }
        };
        if better {
            self.best = Some((k.clone(), v.clone()));
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<SqlValue, String> {
        Ok(self.best.take().map_or(SqlValue::Null, |(_, v)| v))
    }
}

/// Every aggregate helper function.
pub fn aggregate_functions() -> Vec<AggregateFunction> {
    vec![
        AggregateFunction {
            name: "tm_min_by".to_string(),
            n_args: 2,
            init: Arc::new(|| {
                Box::new(ByKey {
                    max: false,
                    best: None,
                })
            }),
        },
        AggregateFunction {
            name: "tm_max_by".to_string(),
            n_args: 2,
            init: Arc::new(|| {
                Box::new(ByKey {
                    max: true,
                    best: None,
                })
            }),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_of(v: &Value, lex: Option<&str>, num: Option<f64>) -> Vec<u8> {
        let id = match codec::encode(v) {
            codec::Encoded::Inline(id) => id.raw(),
            codec::Encoded::Term(t) => ObjectId::from_unsigned(t.tag, 1).raw(),
        };
        let lang = match v {
            Value::LangStr { lang, .. } => Some(lang.as_str()),
            _ => None,
        };
        let k = sort_key(id, lex, num, lang).unwrap();
        assert_eq!(k, value_key(v), "{v:?}");
        k
    }

    #[test]
    fn numbers_order_across_kinds() {
        let a = key_of(&Value::Int(-7), None, None);
        let b = key_of(&Value::Double(2.5), Some("2.5E0"), Some(2.5));
        let c = key_of(&Value::Int(3), None, None);
        let d = key_of(&Value::Decimal("3.5".into()), Some("3.5"), Some(3.5));
        let e = key_of(&Value::Double(-2.0), Some("-2.0E0"), Some(-2.0));
        assert!(a < e && e < b && b < c && c < d);
        // equal values of different kinds have equal keys
        assert_eq!(
            key_of(&Value::Int(1), None, None),
            key_of(&Value::Decimal("1.0".into()), Some("1.0"), Some(1.0))
        );
        // 60-bit integers stay exact
        let big = (1i64 << 55) + 1;
        assert!(key_of(&Value::Int(big - 1), None, None) < key_of(&Value::Int(big), None, None));
    }

    #[test]
    fn datetimes_order_by_instant() {
        let dt = |ms: i64, tz: Option<i16>| Value::DateTime { ms, tz };
        let neg = key_of(&dt(-86_400_000, Some(0)), None, None);
        let zero = key_of(&dt(0, None), None, None);
        let pos = key_of(&dt(1_000, Some(120)), None, None);
        assert!(neg < zero && zero < pos);
        // same instant, different offsets: equal keys
        let a = key_of(&dt(1_772_359_200_000, Some(120)), None, None);
        let b = key_of(&dt(1_772_359_200_000, Some(0)), None, None);
        assert_eq!(a, b);
    }

    #[test]
    fn strings_code_point_and_lang_after_plain() {
        let zoe = key_of(&Value::str("Zoe"), None, None);
        let alex = key_of(&Value::str("Alexander"), Some("Alexander"), None);
        let bo = key_of(&Value::str("Bo"), None, None);
        assert!(alex < bo && bo < zoe);
        let e_acute = key_of(&Value::str("é"), None, None);
        let z = key_of(&Value::str("z"), None, None);
        assert!(z < e_acute, "code point order");
        let plain = key_of(&Value::str("chat"), None, None);
        let lang = key_of(
            &Value::LangStr {
                lex: "chat".into(),
                lang: "fr".into(),
            },
            Some("chat"),
            None,
        );
        assert!(plain < lang);
    }

    #[test]
    fn cross_kind_rank() {
        let node = key_of(&Value::Node(5), None, None);
        let iri = key_of(&Value::iri("http://a"), Some("http://a"), None);
        let num = key_of(&Value::Int(0), None, None);
        let s = key_of(&Value::str("a"), None, None);
        assert!(node < iri && iri < num && num < s);
    }

    #[test]
    fn computed_keys_match_term_keys() {
        assert_eq!(
            vkey(&SqlValue::Integer(3), class::NUM).unwrap(),
            value_key(&Value::Int(3))
        );
        assert_eq!(
            vkey(&SqlValue::Real(2.5), class::DYNAMIC).unwrap(),
            value_key(&Value::Double(2.5))
        );
        assert_eq!(
            vkey(&SqlValue::Text("x".into()), class::STR).unwrap(),
            value_key(&Value::str("x"))
        );
        assert_eq!(vkind(&SqlValue::Null, class::NUM), None);
    }

    #[test]
    fn regex_flags() {
        assert_eq!(regex_match("Hello", "^h", "i"), Some(true));
        assert_eq!(regex_match("Hello", "^h", ""), Some(false));
        assert_eq!(regex_match("x", "(", ""), None);
    }

    #[test]
    fn min_by_aggregate() {
        let mut st = (aggregate_functions()[0].init)();
        for (v, k) in [
            (3, value_key(&Value::str("Zoe"))),
            (1, value_key(&Value::str("Alexander"))),
        ] {
            st.step(&[SqlValue::Integer(v), SqlValue::Blob(k)]).unwrap();
        }
        assert_eq!(st.finish().unwrap(), SqlValue::Integer(1));
    }
}
