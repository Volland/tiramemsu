//! Values as they cross the API, and literal canonicalisation.
//!
//! Every value has exactly one canonical form, so that it has exactly one
//! ObjectId (see `lat.md/data-model#ObjectId#Canonical Encoding`).

use std::fmt;

use crate::id::{Eid, TxId};
use crate::vocab::{
    RDF_LANGSTRING, XSD_BOOLEAN, XSD_DATE, XSD_DATETIME, XSD_DECIMAL, XSD_DOUBLE, XSD_INTEGER,
    XSD_STRING,
};

/// Smallest `INT` value: `-2^59`.
pub const INT_MIN: i64 = -(1i64 << 59);
/// Largest `INT` value: `2^59 - 1`.
pub const INT_MAX: i64 = (1i64 << 59) - 1;
/// Date-times must satisfy `DATETIME_MIN_MS <= ms < DATETIME_LIMIT_MS`.
pub const DATETIME_LIMIT_MS: i64 = 1i64 << 48;
/// Smallest inline date-time instant.
pub const DATETIME_MIN_MS: i64 = -(1i64 << 48);
/// Largest timezone offset in minutes (14:00).
pub const MAX_OFFSET_MIN: i16 = 14 * 60;

const MS_PER_DAY: i128 = 86_400_000;

/// A value in any statement position.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// An IRI. Skolem IRIs (`urn:tiramemsu:node:<n>`, ...) encode to their inline id.
    Iri(String),
    /// Anonymous node with its counter.
    Node(u64),
    /// Blank node with its counter.
    BNode(u64),
    /// A statement eid.
    Stmt(Eid),
    /// A transaction.
    Tx(TxId),
    /// An integer (`INT` when inside 60 bits, else `TYPED xsd:integer`).
    Int(i64),
    /// A boolean.
    Bool(bool),
    /// An `xsd:dateTime`: epoch milliseconds and optional offset in minutes.
    DateTime {
        /// The instant in epoch milliseconds (UTC).
        ms: i64,
        /// The timezone offset in minutes, `None` when the lexical form had none.
        tz: Option<i16>,
    },
    /// An `xsd:date`: days since 1970-01-01.
    Date(i64),
    /// A plain string (`SHORT_STR` or `STR`, chosen by the codec).
    Str(String),
    /// A language-tagged string.
    LangStr {
        /// Lexical form.
        lex: String,
        /// Language tag (lower-cased when canonical).
        lang: String,
    },
    /// A literal of any other datatype, or an ill-typed / out-of-range literal.
    Typed {
        /// Lexical form, kept verbatim.
        lex: String,
        /// Datatype IRI.
        datatype: String,
    },
    /// An `xsd:double`.
    Double(f64),
    /// An `xsd:decimal`, as its canonical lexical form.
    Decimal(String),
}

impl Value {
    /// An IRI value.
    pub fn iri(s: impl Into<String>) -> Value {
        Value::Iri(s.into())
    }

    /// A plain string value.
    pub fn str(s: impl Into<String>) -> Value {
        Value::Str(s.into())
    }

    /// Builds the canonical value of an RDF literal.
    ///
    /// Only `xsd:integer` maps to `INT`; derived integer types stay `TYPED`.
    /// Ill-typed literals of the special datatypes are kept verbatim as `TYPED`.
    pub fn literal(lex: &str, datatype: Option<&str>, lang: Option<&str>) -> Value {
        if let Some(lang) = lang {
            return Value::LangStr {
                lex: lex.to_string(),
                lang: lang.to_ascii_lowercase(),
            };
        }
        let typed = |dt: &str| Value::Typed {
            lex: lex.to_string(),
            datatype: dt.to_string(),
        };
        match datatype {
            None | Some(XSD_STRING) => Value::Str(lex.to_string()),
            Some(XSD_INTEGER) => Value::big_integer(lex),
            Some(XSD_BOOLEAN) => match lex {
                "true" | "1" => Value::Bool(true),
                "false" | "0" => Value::Bool(false),
                _ => typed(XSD_BOOLEAN),
            },
            Some(XSD_DATETIME) => match parse_datetime(lex) {
                Some((ms, tz)) if datetime_in_range(ms) => Value::DateTime { ms, tz },
                _ => typed(XSD_DATETIME),
            },
            Some(XSD_DATE) => match parse_date(lex) {
                Some(days) => Value::Date(days),
                None => typed(XSD_DATE),
            },
            Some(XSD_DOUBLE) => match parse_double(lex) {
                Some(x) => Value::Double(x),
                None => typed(XSD_DOUBLE),
            },
            Some(XSD_DECIMAL) => match canonical_decimal(lex) {
                Some(c) => Value::Decimal(c),
                None => typed(XSD_DECIMAL),
            },
            Some(dt) => typed(dt),
        }
    }

    /// An integer given as decimal text: `INT` if it fits in 60 bits, else
    /// `TYPED xsd:integer` with the canonical decimal; ill-typed text stays verbatim.
    pub fn big_integer(decimal: &str) -> Value {
        match canonical_integer(decimal) {
            Some(canon) => match canon.parse::<i64>() {
                Ok(i) if (INT_MIN..=INT_MAX).contains(&i) => Value::Int(i),
                _ => Value::Typed {
                    lex: canon,
                    datatype: XSD_INTEGER.to_string(),
                },
            },
            None => Value::Typed {
                lex: decimal.to_string(),
                datatype: XSD_INTEGER.to_string(),
            },
        }
    }

    /// The canonical form of this value (the value its ObjectId decodes to).
    pub fn canonical(&self) -> Value {
        match self {
            Value::Iri(s) => crate::codec::parse_skolem(s).unwrap_or_else(|| self.clone()),
            Value::Int(i) if !(INT_MIN..=INT_MAX).contains(i) => Value::Typed {
                lex: i.to_string(),
                datatype: XSD_INTEGER.to_string(),
            },
            Value::DateTime { ms, tz } => {
                let tz_ok = tz.is_none_or(|m| m.abs() <= MAX_OFFSET_MIN);
                if datetime_in_range(*ms) && tz_ok {
                    self.clone()
                } else {
                    Value::Typed {
                        lex: format_datetime(*ms, *tz),
                        datatype: XSD_DATETIME.to_string(),
                    }
                }
            }
            Value::Date(d) if !(INT_MIN..=INT_MAX).contains(d) => Value::Typed {
                lex: format_date(*d),
                datatype: XSD_DATE.to_string(),
            },
            Value::LangStr { lex, lang } => Value::LangStr {
                lex: lex.clone(),
                lang: lang.to_ascii_lowercase(),
            },
            Value::Typed { lex, datatype } => Value::literal(lex, Some(datatype), None),
            Value::Decimal(s) => match canonical_decimal(s) {
                Some(c) => Value::Decimal(c),
                None => Value::Typed {
                    lex: s.clone(),
                    datatype: XSD_DECIMAL.to_string(),
                },
            },
            _ => self.clone(),
        }
    }

    /// The lexical form of a literal value (or the IRI text).
    pub fn lexical(&self) -> String {
        match self {
            Value::Iri(s) => s.clone(),
            Value::Node(n) => format!("{}{n}", crate::vocab::SKOLEM_NODE),
            Value::BNode(n) => format!("{}{n}", crate::vocab::SKOLEM_BNODE),
            Value::Stmt(e) => format!("{}{}", crate::vocab::SKOLEM_STMT, e.n()),
            Value::Tx(t) => format!("{}{}", crate::vocab::SKOLEM_TX, t.0),
            Value::Int(i) => i.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::DateTime { ms, tz } => format_datetime(*ms, *tz),
            Value::Date(d) => format_date(*d),
            Value::Str(s) => s.clone(),
            Value::LangStr { lex, .. } => lex.clone(),
            Value::Typed { lex, .. } => lex.clone(),
            Value::Double(x) => canonical_double(*x),
            Value::Decimal(s) => s.clone(),
        }
    }

    /// The datatype IRI of a literal (`None` for IRIs, nodes, statements, transactions).
    pub fn datatype(&self) -> Option<&str> {
        match self {
            Value::Int(_) => Some(XSD_INTEGER),
            Value::Bool(_) => Some(XSD_BOOLEAN),
            Value::DateTime { .. } => Some(XSD_DATETIME),
            Value::Date(_) => Some(XSD_DATE),
            Value::Str(_) => Some(XSD_STRING),
            Value::LangStr { .. } => Some(RDF_LANGSTRING),
            Value::Typed { datatype, .. } => Some(datatype),
            Value::Double(_) => Some(XSD_DOUBLE),
            Value::Decimal(_) => Some(XSD_DECIMAL),
            _ => None,
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Iri(_) | Value::Node(_) | Value::BNode(_) | Value::Stmt(_) | Value::Tx(_) => {
                write!(f, "<{}>", self.lexical())
            }
            Value::LangStr { lex, lang } => write!(f, "{lex:?}@{lang}"),
            Value::Str(s) => write!(f, "{s:?}"),
            other => write!(
                f,
                "{:?}^^<{}>",
                other.lexical(),
                other.datatype().unwrap_or("")
            ),
        }
    }
}

/// True when a date-time instant fits the inline `DATETIME` payload.
pub fn datetime_in_range(ms: i64) -> bool {
    (DATETIME_MIN_MS..DATETIME_LIMIT_MS).contains(&ms)
}

/// Canonical decimal text of an `xsd:integer` lexical form, or `None` if ill-typed.
pub fn canonical_integer(lex: &str) -> Option<String> {
    let (neg, digits) = split_sign(lex);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let trimmed = digits.trim_start_matches('0');
    if trimmed.is_empty() {
        return Some("0".to_string());
    }
    Some(if neg {
        format!("-{trimmed}")
    } else {
        trimmed.to_string()
    })
}

/// Canonical `xsd:decimal` lexical form, or `None` if ill-typed.
///
/// No `+`, no leading zeros in the integer part, no trailing zeros in the fraction,
/// at least `.0`; negative zero is `0.0`.
pub fn canonical_decimal(lex: &str) -> Option<String> {
    let (neg, body) = split_sign(lex);
    let (int, frac) = match body.split_once('.') {
        Some((i, f)) => (i, f),
        None => (body, ""),
    };
    if int.is_empty() && frac.is_empty() {
        return None;
    }
    if !int.bytes().all(|b| b.is_ascii_digit()) || !frac.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let int = match int.trim_start_matches('0') {
        "" => "0",
        s => s,
    };
    let frac = match frac.trim_end_matches('0') {
        "" => "0",
        s => s,
    };
    let zero = int == "0" && frac == "0";
    Some(format!(
        "{}{int}.{frac}",
        if neg && !zero { "-" } else { "" }
    ))
}

/// Canonical `xsd:double` lexical form (`1.0E0`, `INF`, `-INF`, `NaN`).
pub fn canonical_double(x: f64) -> String {
    if x.is_nan() {
        return "NaN".to_string();
    }
    if x.is_infinite() {
        return if x > 0.0 { "INF" } else { "-INF" }.to_string();
    }
    let s = format!("{x:e}");
    let (mant, exp) = s.split_once('e').unwrap_or((&s, "0"));
    if mant.contains('.') {
        format!("{mant}E{exp}")
    } else {
        format!("{mant}.0E{exp}")
    }
}

/// Parses an `xsd:double` lexical form.
pub fn parse_double(lex: &str) -> Option<f64> {
    match lex {
        "INF" | "+INF" => return Some(f64::INFINITY),
        "-INF" => return Some(f64::NEG_INFINITY),
        "NaN" => return Some(f64::NAN),
        _ => {}
    }
    let (_, body) = split_sign(lex);
    let (mant, exp) = match body.find(['e', 'E']) {
        Some(i) => (&body[..i], Some(&body[i + 1..])),
        None => (body, None),
    };
    let (int, frac) = mant.split_once('.').unwrap_or((mant, ""));
    if int.is_empty() && frac.is_empty() {
        return None;
    }
    if !int.bytes().all(|b| b.is_ascii_digit()) || !frac.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if let Some(exp) = exp {
        let (_, ed) = split_sign(exp);
        if ed.is_empty() || !ed.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
    }
    lex.parse::<f64>().ok()
}

fn split_sign(s: &str) -> (bool, &str) {
    if let Some(r) = s.strip_prefix('-') {
        (true, r)
    } else if let Some(r) = s.strip_prefix('+') {
        (false, r)
    } else {
        (false, s)
    }
}

/// Days from 1970-01-01 to the proleptic Gregorian date `y-m-d` (astronomical years).
fn days_from_civil(y: i128, m: u32, d: u32) -> i128 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = m as i128;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i128 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The proleptic Gregorian date of a day number.
fn civil_from_days(z: i128) -> (i128, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn is_leap(y: i128) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_month(y: i128, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(y) => 29,
        2 => 28,
        _ => 0,
    }
}

fn digits(s: &str, n: usize) -> Option<u32> {
    (s.len() == n && s.bytes().all(|b| b.is_ascii_digit())).then(|| s.parse().ok())?
}

/// Parses `[-]YYYY-MM-DD` into (year, month, day) and the rest of the string.
fn parse_ymd(s: &str) -> Option<((i128, u32, u32), &str)> {
    let (neg, body) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s),
    };
    let dash = body.find('-')?;
    let ys = &body[..dash];
    if ys.len() < 4 || ys.len() > 30 || !ys.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if ys.len() > 4 && ys.starts_with('0') {
        return None;
    }
    let y: i128 = ys.parse().ok()?;
    let y = if neg { -y } else { y };
    let rest = &body[dash + 1..];
    if rest.len() < 5 || rest.as_bytes()[2] != b'-' {
        return None;
    }
    let m = digits(&rest[..2], 2)?;
    let d = digits(&rest[3..5], 2)?;
    if !(1..=12).contains(&m) || d < 1 || d > days_in_month(y, m) {
        return None;
    }
    Some(((y, m, d), &rest[5..]))
}

/// Parses an optional timezone suffix into an offset in minutes.
/// `Ok(None)` = no timezone; `Err(())` = ill-formed or beyond ±14:00.
fn parse_tz(s: &str) -> Result<Option<i16>, ()> {
    if s.is_empty() {
        return Ok(None);
    }
    if s == "Z" {
        return Ok(Some(0));
    }
    let neg = match s.as_bytes()[0] {
        b'+' => false,
        b'-' => true,
        _ => return Err(()),
    };
    let rest = &s[1..];
    if rest.len() != 5 || rest.as_bytes()[2] != b':' {
        return Err(());
    }
    let h = digits(&rest[..2], 2).ok_or(())?;
    let m = digits(&rest[3..], 2).ok_or(())?;
    if m > 59 {
        return Err(());
    }
    let total = (h * 60 + m) as i16;
    if total > MAX_OFFSET_MIN {
        return Err(());
    }
    Ok(Some(if neg { -total } else { total }))
}

/// Parses an `xsd:date` into days since 1970-01-01 (timezone ignored).
/// Returns `None` if ill-typed or outside the 60-bit payload.
pub fn parse_date(lex: &str) -> Option<i64> {
    let ((y, m, d), rest) = parse_ymd(lex)?;
    parse_tz(rest).ok()?;
    let days = days_from_civil(y, m, d);
    let days = i64::try_from(days).ok()?;
    (INT_MIN..=INT_MAX).contains(&days).then_some(days)
}

/// Parses an `xsd:dateTime` into (epoch ms, offset minutes). Sub-millisecond digits
/// are truncated. Returns `None` if ill-typed (including offsets beyond ±14:00).
/// The instant may lie outside the inline range; check [`datetime_in_range`].
pub fn parse_datetime(lex: &str) -> Option<(i64, Option<i16>)> {
    let ((y, mo, d), rest) = parse_ymd(lex)?;
    let rest = rest.strip_prefix('T')?;
    if rest.len() < 8 || rest.as_bytes()[2] != b':' || rest.as_bytes()[5] != b':' {
        return None;
    }
    let h = digits(&rest[..2], 2)?;
    let mi = digits(&rest[3..5], 2)?;
    let s = digits(&rest[6..8], 2)?;
    let mut rest = &rest[8..];
    let mut frac_ms: u32 = 0;
    let mut frac_zero = true;
    if let Some(r) = rest.strip_prefix('.') {
        let n = r.bytes().take_while(|b| b.is_ascii_digit()).count();
        if n == 0 {
            return None;
        }
        let f = &r[..n];
        frac_zero = f.bytes().all(|b| b == b'0');
        let mut ms_digits: String = f.chars().take(3).collect();
        while ms_digits.len() < 3 {
            ms_digits.push('0');
        }
        frac_ms = ms_digits.parse().ok()?;
        rest = &r[n..];
    }
    let tz = parse_tz(rest).ok()?;
    if mi > 59 || s > 59 {
        return None;
    }
    if h > 24 || (h == 24 && (mi != 0 || s != 0 || !frac_zero)) {
        return None;
    }
    let days = days_from_civil(y, mo, d);
    let local = days * MS_PER_DAY
        + (h as i128) * 3_600_000
        + (mi as i128) * 60_000
        + (s as i128) * 1000
        + frac_ms as i128;
    let utc = local - (tz.unwrap_or(0) as i128) * 60_000;
    let ms = i64::try_from(utc).ok()?;
    Some((ms, tz))
}

fn format_year(y: i128) -> String {
    if y < 0 {
        format!("-{:04}", -y)
    } else {
        format!("{y:04}")
    }
}

/// Formats days since 1970-01-01 as `YYYY-MM-DD`.
pub fn format_date(days: i64) -> String {
    let (y, m, d) = civil_from_days(days as i128);
    format!("{}-{m:02}-{d:02}", format_year(y))
}

/// Formats a date-time in its own offset: `YYYY-MM-DDThh:mm:ss.mmm[Z|±hh:mm]`.
pub fn format_datetime(ms: i64, tz: Option<i16>) -> String {
    let local = ms as i128 + tz.unwrap_or(0) as i128 * 60_000;
    let days = local.div_euclid(MS_PER_DAY);
    let tod = local.rem_euclid(MS_PER_DAY);
    let (y, m, d) = civil_from_days(days);
    let h = tod / 3_600_000;
    let mi = (tod / 60_000) % 60;
    let s = (tod / 1000) % 60;
    let milli = tod % 1000;
    let zone = match tz {
        None => String::new(),
        Some(0) => "Z".to_string(),
        Some(off) => {
            let sign = if off < 0 { '-' } else { '+' };
            let a = off.unsigned_abs();
            format!("{sign}{:02}:{:02}", a / 60, a % 60)
        }
    };
    format!(
        "{}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{milli:03}{zone}",
        format_year(y)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_round_trip() {
        for d in [-800_000i128, -1, 0, 1, 20_000, 2_932_896] {
            let (y, m, dd) = civil_from_days(d);
            assert_eq!(days_from_civil(y, m, dd), d);
        }
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(1969, 12, 31), -1);
    }

    #[test]
    fn datetime_parse_and_format() {
        let (ms, tz) = parse_datetime("2026-03-01T12:00:00+02:00").unwrap();
        assert_eq!(tz, Some(120));
        assert_eq!(format_datetime(ms, tz), "2026-03-01T12:00:00.000+02:00");
        let (ms2, tz2) = parse_datetime("2026-03-01T10:00:00Z").unwrap();
        assert_eq!(ms, ms2);
        assert_eq!(format_datetime(ms2, tz2), "2026-03-01T10:00:00.000Z");
        assert_eq!(
            parse_datetime("2026-03-01T10:00:00.123456Z").unwrap().0,
            ms2 + 123
        );
        assert!(parse_datetime("2026-03-01T10:00:00+14:01").is_none());
        assert!(parse_datetime("2026-02-30T10:00:00").is_none());
        assert!(parse_datetime("1969-12-31T23:59:59.999Z").unwrap().0 == -1);
    }

    #[test]
    fn numbers() {
        assert_eq!(canonical_integer("+0001").unwrap(), "1");
        assert_eq!(canonical_integer("-000").unwrap(), "0");
        assert!(canonical_integer("1.0").is_none());
        assert_eq!(canonical_decimal("01.50").unwrap(), "1.5");
        assert_eq!(canonical_decimal("-0.0").unwrap(), "0.0");
        assert_eq!(canonical_decimal("3").unwrap(), "3.0");
        assert_eq!(canonical_decimal(".5").unwrap(), "0.5");
        assert!(canonical_decimal(".").is_none());
        assert_eq!(canonical_double(1.0), "1.0E0");
        assert_eq!(canonical_double(150.0), "1.5E2");
        assert_eq!(canonical_double(-0.00003), "-3.0E-5");
        assert_eq!(parse_double("1E0"), Some(1.0));
        assert_eq!(parse_double("1.0"), Some(1.0));
        assert!(parse_double("inf").is_none());
        assert!(parse_double("1e").is_none());
    }
}
