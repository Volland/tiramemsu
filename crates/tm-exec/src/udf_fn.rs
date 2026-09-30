//! SQL helper functions for the SPARQL built-ins: string, numeric, date-time and
//! cast functions, and the boxed literal (`STRDT` / `STRLANG` / `TIMEZONE`)
//! representation. Registered next to the core helpers by [`crate::udf`].

use tm_core::{codec, value, vocab, ObjectId, ScalarFunction, SqlValue, Tag, Value};

use crate::udf::{arg_f64, arg_i64, arg_str, opt, scalar};

/// Separator between the lexical form and its datatype (`dt`) or language (`@tag`)
/// in a boxed literal, a TEXT value of class `Lit`.
pub const LIT_SEP: char = '\u{1}';

/// Cast target codes of `tm_cast`.
pub mod cast {
    /// `xsd:integer`.
    pub const INTEGER: i64 = 1;
    /// `xsd:double`.
    pub const DOUBLE: i64 = 2;
    /// `xsd:boolean`.
    pub const BOOLEAN: i64 = 3;
    /// `xsd:date`.
    pub const DATE: i64 = 4;
    /// `xsd:dateTime`.
    pub const DATETIME: i64 = 5;
    /// `xsd:decimal` (stored as a double).
    pub const DECIMAL: i64 = 6;
}

/// `LANGMATCHES(tag, range)` (RFC 4647 basic filtering).
pub fn lang_matches(tag: &str, range: &str) -> bool {
    if range == "*" {
        return !tag.is_empty();
    }
    let (tag, range) = (tag.to_ascii_lowercase(), range.to_ascii_lowercase());
    tag == range || (tag.starts_with(&range) && tag[range.len()..].starts_with('-'))
}

/// `ENCODE_FOR_URI`.
pub fn encode_for_uri(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `SUBSTR(s, start [, len])` with XPath rounding and 1-based positions.
pub fn substr(s: &str, start: f64, len: Option<f64>) -> Option<String> {
    if start.is_nan() || len.is_some_and(f64::is_nan) {
        return Some(String::new());
    }
    let first = (start + 0.5).floor();
    let last = match len {
        Some(l) => first + (l + 0.5).floor(),
        None => f64::INFINITY,
    };
    Some(
        s.chars()
            .enumerate()
            .filter(|(i, _)| {
                let pos = (*i + 1) as f64;
                pos >= first && pos < last
            })
            .map(|(_, c)| c)
            .collect(),
    )
}

/// The local date-time fields of a `DATETIME` id: `(year, month, day, hour,
/// minute, milliseconds of the minute)`.
pub fn dt_fields(id: i64) -> Option<(i64, i64, i64, i64, i64, i64)> {
    let oid = ObjectId::from_raw(id);
    let Some(Value::DateTime { ms, tz }) = codec::decode_inline(oid)
        .ok()?
        .filter(|_| oid.tag().ok() == Some(Tag::DateTime))
    else {
        return None;
    };
    let lex = value::format_datetime(ms, tz);
    // YYYY-MM-DDThh:mm:ss.mmm[zone], the year possibly negative
    let (neg, body) = match lex.strip_prefix('-') {
        Some(b) => (true, b),
        None => (false, lex.as_str()),
    };
    let year: i64 = body.get(0..4)?.parse().ok()?;
    let year = if neg { -year } else { year };
    let n = |a: usize, b: usize| body.get(a..b)?.parse::<i64>().ok();
    Some((
        year,
        n(5, 7)?,
        n(8, 10)?,
        n(11, 13)?,
        n(14, 16)?,
        n(17, 19)? * 1000 + n(20, 23)?,
    ))
}

/// The stored offset in minutes of a `DATETIME` id.
fn dt_tz(id: i64) -> Option<Option<i16>> {
    let oid = ObjectId::from_raw(id);
    match codec::decode_inline(oid).ok()?? {
        Value::DateTime { tz, .. } if oid.tag().ok() == Some(Tag::DateTime) => Some(tz),
        _ => None,
    }
}

/// `TZ`: `"Z"`, `"+02:00"`, `""` without a timezone.
pub fn tz_text(id: i64) -> Option<String> {
    Some(match dt_tz(id)? {
        None => String::new(),
        Some(0) => "Z".to_string(),
        Some(m) => format!(
            "{}{:02}:{:02}",
            if m < 0 { '-' } else { '+' },
            m.abs() / 60,
            m.abs() % 60
        ),
    })
}

/// `TIMEZONE`: the offset as an `xsd:dayTimeDuration` lexical form; `None` without
/// a timezone.
pub fn timezone_text(id: i64) -> Option<String> {
    let m = dt_tz(id)??;
    let (sign, a) = (if m < 0 { "-" } else { "" }, m.abs());
    let (h, mi) = (a / 60, a % 60);
    let mut out = format!("{sign}PT");
    if h != 0 {
        out.push_str(&format!("{h}H"));
    }
    if mi != 0 {
        out.push_str(&format!("{mi}M"));
    }
    if h == 0 && mi == 0 {
        out.push_str("0S");
    }
    Some(out)
}

fn inline(v: &Value) -> SqlValue {
    match codec::encode(v) {
        codec::Encoded::Inline(id) => SqlValue::Integer(id.raw()),
        codec::Encoded::Term(_) => SqlValue::Null,
    }
}

/// The XSD cast of a lexical form: a native INTEGER / REAL, or an inline
/// ObjectId for `date` and `dateTime`. `NULL` when the cast is an error.
pub fn cast_lexical(lex: &str, target: i64) -> SqlValue {
    let lex = lex.trim();
    let number = || -> Option<f64> {
        match lex {
            "true" => Some(1.0),
            "false" => Some(0.0),
            _ => value::parse_double(lex)
                .or_else(|| value::canonical_decimal(lex).and_then(|d| d.parse().ok())),
        }
    };
    match target {
        cast::INTEGER => match value::canonical_integer(lex).and_then(|c| c.parse::<i64>().ok()) {
            Some(i) if (value::INT_MIN..=value::INT_MAX).contains(&i) => SqlValue::Integer(i),
            Some(_) => SqlValue::Null,
            None => match number() {
                Some(x) if x.is_finite() && x.abs() < (1u64 << 59) as f64 => {
                    SqlValue::Integer(x.trunc() as i64)
                }
                _ => SqlValue::Null,
            },
        },
        cast::DOUBLE | cast::DECIMAL => match number() {
            Some(x) if target == cast::DECIMAL && !x.is_finite() => SqlValue::Null,
            Some(x) => SqlValue::Real(x),
            None => SqlValue::Null,
        },
        cast::BOOLEAN => match lex {
            "true" | "1" => SqlValue::Integer(1),
            "false" | "0" => SqlValue::Integer(0),
            _ => match number() {
                Some(x) => SqlValue::Integer((x != 0.0 && !x.is_nan()) as i64),
                None => SqlValue::Null,
            },
        },
        cast::DATE => {
            if lex.contains('T') {
                match value::parse_datetime(lex) {
                    Some((ms, tz)) => {
                        let local = ms + tz.unwrap_or(0) as i64 * 60_000;
                        inline(&Value::Date(local.div_euclid(86_400_000)))
                    }
                    None => SqlValue::Null,
                }
            } else {
                value::parse_date(lex).map_or(SqlValue::Null, |d| inline(&Value::Date(d)))
            }
        }
        cast::DATETIME => {
            let v = if lex.contains('T') {
                value::parse_datetime(lex)
            } else {
                value::parse_date(lex).map(|d| (d * 86_400_000, None))
            };
            match v {
                Some((ms, tz)) if value::datetime_in_range(ms) => {
                    inline(&Value::DateTime { ms, tz })
                }
                _ => SqlValue::Null,
            }
        }
        _ => SqlValue::Null,
    }
}

/// The parts of a boxed literal: `(lexical form, datatype, language)`.
pub fn unbox(s: &str) -> (String, Option<String>, Option<String>) {
    match s.split_once(LIT_SEP) {
        Some((lex, meta)) => match meta.strip_prefix('@') {
            Some(lang) => (lex.to_string(), None, Some(lang.to_string())),
            None => (lex.to_string(), Some(meta.to_string()), None),
        },
        None => (s.to_string(), None, None),
    }
}

/// The value a boxed literal denotes.
pub fn unboxed_value(s: &str) -> Value {
    let (lex, dt, lang) = unbox(s);
    Value::literal(&lex, dt.as_deref(), lang.as_deref())
}

fn num_fn(name: &str, f: fn(f64) -> f64) -> ScalarFunction {
    scalar(name, 1, move |a| {
        Ok(match a.first() {
            Some(SqlValue::Integer(i)) => SqlValue::Integer(*i),
            Some(SqlValue::Real(x)) => SqlValue::Real(f(*x)),
            _ => SqlValue::Null,
        })
    })
}

/// The SPARQL helper functions.
pub fn functions() -> Vec<ScalarFunction> {
    vec![
        scalar("tm_langmatches", 2, |a| {
            Ok(match (arg_str(a, 0), arg_str(a, 1)) {
                (Some(t), Some(r)) => SqlValue::Integer(lang_matches(t, r) as i64),
                _ => SqlValue::Null,
            })
        }),
        scalar("tm_encode_uri", 1, |a| {
            Ok(opt(arg_str(a, 0), |s| SqlValue::Text(encode_for_uri(s))))
        }),
        scalar("tm_replace", 4, |a| {
            let (Some(s), Some(p), Some(r)) = (arg_str(a, 0), arg_str(a, 1), arg_str(a, 2)) else {
                return Ok(SqlValue::Null);
            };
            let mut b = regex::RegexBuilder::new(p);
            for f in arg_str(a, 3).unwrap_or("").chars() {
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
            Ok(match b.build() {
                Ok(re) => SqlValue::Text(re.replace_all(s, r).into_owned()),
                Err(_) => SqlValue::Null,
            })
        }),
        scalar("tm_substr", 3, |a| {
            let (Some(s), Some(start)) = (arg_str(a, 0), arg_f64(a, 1)) else {
                return Ok(SqlValue::Null);
            };
            Ok(opt(substr(s, start, arg_f64(a, 2)), SqlValue::Text))
        }),
        num_fn("tm_ceil", f64::ceil),
        num_fn("tm_floor", f64::floor),
        num_fn("tm_round", |x| (x + 0.5).floor()),
        scalar("tm_dt", 2, |a| {
            let (Some(id), Some(part)) = (arg_i64(a, 0), arg_i64(a, 1)) else {
                return Ok(SqlValue::Null);
            };
            Ok(match dt_fields(id) {
                None => SqlValue::Null,
                Some((y, mo, d, h, mi, msec)) => match part {
                    0 => SqlValue::Integer(y),
                    1 => SqlValue::Integer(mo),
                    2 => SqlValue::Integer(d),
                    3 => SqlValue::Integer(h),
                    4 => SqlValue::Integer(mi),
                    _ if msec % 1000 == 0 => SqlValue::Integer(msec / 1000),
                    _ => SqlValue::Real(msec as f64 / 1000.0),
                },
            })
        }),
        scalar("tm_tz", 1, |a| {
            Ok(opt(arg_i64(a, 0).and_then(tz_text), SqlValue::Text))
        }),
        scalar("tm_timezone", 1, |a| {
            Ok(opt(arg_i64(a, 0).and_then(timezone_text), |t| {
                SqlValue::Text(format!("{t}{LIT_SEP}{}", vocab::XSD_DAYTIME_DURATION))
            }))
        }),
        scalar("tm_cast", 2, |a| {
            Ok(match (arg_str(a, 0), arg_i64(a, 1)) {
                (Some(l), Some(t)) => cast_lexical(l, t),
                _ => SqlValue::Null,
            })
        }),
        scalar("tm_lit", 3, |a| {
            let (Some(lex), Some(meta)) = (arg_str(a, 0), arg_str(a, 1)) else {
                return Ok(SqlValue::Null);
            };
            let at = if arg_i64(a, 2) == Some(1) { "@" } else { "" };
            Ok(SqlValue::Text(format!(
                "{lex}{LIT_SEP}{at}{}",
                if at.is_empty() {
                    meta.to_string()
                } else {
                    meta.to_ascii_lowercase()
                }
            )))
        }),
        scalar("tm_lit_lex", 1, |a| {
            Ok(opt(arg_str(a, 0), |s| SqlValue::Text(unbox(s).0)))
        }),
        scalar("tm_lit_lang", 1, |a| {
            Ok(opt(arg_str(a, 0), |s| {
                SqlValue::Text(unbox(s).2.unwrap_or_default())
            }))
        }),
        scalar("tm_lit_dt", 1, |a| {
            Ok(opt(arg_str(a, 0), |s| {
                let (_, dt, lang) = unbox(s);
                match (dt, lang) {
                    (Some(d), _) => SqlValue::Text(d),
                    (None, Some(_)) => SqlValue::Text(vocab::RDF_LANGSTRING.to_string()),
                    (None, None) => SqlValue::Text(vocab::XSD_STRING.to_string()),
                }
            }))
        }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substring_follows_xpath() {
        assert_eq!(substr("motor car", 6.0, None).as_deref(), Some(" car"));
        assert_eq!(substr("metadata", 4.0, Some(3.0)).as_deref(), Some("ada"));
        assert_eq!(substr("abc", 0.0, Some(2.0)).as_deref(), Some("a"));
        assert_eq!(substr("abc", -1.0, Some(3.0)).as_deref(), Some("a"));
        assert_eq!(substr("abc", 2.0, Some(10.0)).as_deref(), Some("bc"));
    }

    #[test]
    fn lang_matches_basic_filtering() {
        assert!(lang_matches("en-GB", "en"));
        assert!(lang_matches("EN", "en"));
        assert!(lang_matches("de", "*"));
        assert!(!lang_matches("", "*"));
        assert!(!lang_matches("english", "en"));
    }

    #[test]
    fn uri_encoding() {
        assert_eq!(encode_for_uri("Los Angeles/ä"), "Los%20Angeles%2F%C3%A4");
    }

    #[test]
    fn timezone_forms() {
        let id = |lex: &str| match inline(&Value::literal(lex, Some(vocab::XSD_DATETIME), None)) {
            SqlValue::Integer(i) => i,
            _ => panic!(),
        };
        assert_eq!(
            tz_text(id("2026-09-01T12:00:00+02:00")).as_deref(),
            Some("+02:00")
        );
        assert_eq!(tz_text(id("2026-09-01T12:00:00Z")).as_deref(), Some("Z"));
        assert_eq!(tz_text(id("2026-09-01T12:00:00")).as_deref(), Some(""));
        assert_eq!(
            timezone_text(id("2026-09-01T12:00:00+02:00")).as_deref(),
            Some("PT2H")
        );
        assert_eq!(
            timezone_text(id("2026-09-01T12:00:00-03:30")).as_deref(),
            Some("-PT3H30M")
        );
        assert_eq!(
            timezone_text(id("2026-09-01T12:00:00Z")).as_deref(),
            Some("PT0S")
        );
        assert_eq!(timezone_text(id("2026-09-01T12:00:00")), None);
        let f = dt_fields(id("2026-09-01T14:05:06.250+02:00")).unwrap();
        assert_eq!(f, (2026, 9, 1, 14, 5, 6250));
    }

    #[test]
    fn casts() {
        assert_eq!(cast_lexical("41", cast::INTEGER), SqlValue::Integer(41));
        assert_eq!(cast_lexical("3.9", cast::INTEGER), SqlValue::Integer(3));
        assert_eq!(cast_lexical("abc", cast::INTEGER), SqlValue::Null);
        assert_eq!(cast_lexical("true", cast::BOOLEAN), SqlValue::Integer(1));
        assert_eq!(cast_lexical("0", cast::BOOLEAN), SqlValue::Integer(0));
        assert_eq!(cast_lexical("1.5", cast::DOUBLE), SqlValue::Real(1.5));
        assert!(matches!(
            cast_lexical("2026-09-01", cast::DATE),
            SqlValue::Integer(_)
        ));
        assert!(matches!(
            cast_lexical("2026-09-01T10:00:00Z", cast::DATETIME),
            SqlValue::Integer(_)
        ));
        assert_eq!(cast_lexical("nope", cast::DATETIME), SqlValue::Null);
    }

    #[test]
    fn boxed_literals_round_trip() {
        let (l, d, g) = unbox(&format!("x{LIT_SEP}@de"));
        assert_eq!((l.as_str(), d, g.as_deref()), ("x", None, Some("de")));
        assert_eq!(
            unboxed_value(&format!("P{LIT_SEP}{}", vocab::XSD_DAYTIME_DURATION)),
            Value::Typed {
                lex: "P".into(),
                datatype: vocab::XSD_DAYTIME_DURATION.into()
            }
        );
    }
}
