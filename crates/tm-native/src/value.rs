//! SQLite value semantics, as the SQL path relies on them: comparison across
//! storage classes, arithmetic with overflow to REAL, text conversion, the string
//! built-ins (`length`, `instr`, `substr`, `||`), JSON lists and the aggregates.
//!
//! The native runner evaluates the same expressions the SQL generator writes, on
//! the same values, so results agree with the SQL route value for value.

use std::cmp::Ordering;

use tm_core::SqlValue;

/// A SQL value. Booleans are `Integer(0 | 1)` or `Null`, as in SQLite.
pub type V = SqlValue;

/// `0` / `1` / NULL from a three-valued truth.
pub fn truth(b: Option<bool>) -> V {
    match b {
        Some(true) => V::Integer(1),
        Some(false) => V::Integer(0),
        None => V::Null,
    }
}

/// The three-valued truth of a value used as a condition: NULL is unknown,
/// numbers are true when non-zero, text is converted to a number first.
pub fn cond(v: &V) -> Option<bool> {
    match v {
        V::Null => None,
        V::Integer(i) => Some(*i != 0),
        V::Real(x) => Some(*x != 0.0),
        V::Text(t) => Some(text_number(t) != 0.0),
        V::Blob(_) | V::IntArray(_) => Some(false),
    }
}

/// The leading numeric prefix of a text, as SQLite reads it (0 when none).
pub fn text_number(t: &str) -> f64 {
    let t = t.trim_start();
    let mut end = 0;
    let b = t.as_bytes();
    if end < b.len() && (b[end] == b'+' || b[end] == b'-') {
        end += 1;
    }
    let digits = |e: &mut usize| {
        while *e < b.len() && b[*e].is_ascii_digit() {
            *e += 1;
        }
    };
    digits(&mut end);
    if end < b.len() && b[end] == b'.' {
        end += 1;
        digits(&mut end);
    }
    if end < b.len() && (b[end] == b'e' || b[end] == b'E') {
        let save = end;
        end += 1;
        if end < b.len() && (b[end] == b'+' || b[end] == b'-') {
            end += 1;
        }
        let before = end;
        digits(&mut end);
        if end == before {
            end = save;
        }
    }
    t[..end].parse().unwrap_or(0.0)
}

fn class(v: &V) -> u8 {
    match v {
        V::Null => 0,
        V::Integer(_) | V::Real(_) => 1,
        V::Text(_) => 2,
        V::Blob(_) | V::IntArray(_) => 3,
    }
}

/// SQLite's ordering of two non-NULL values (no affinity): numbers before text
/// before blobs; integers and reals numerically; text and blobs bytewise.
pub fn order(a: &V, b: &V) -> Ordering {
    match (a, b) {
        (V::Integer(x), V::Integer(y)) => x.cmp(y),
        (V::Integer(x), V::Real(y)) => cmp_int_real(*x, *y),
        (V::Real(x), V::Integer(y)) => cmp_int_real(*y, *x).reverse(),
        (V::Real(x), V::Real(y)) => x.partial_cmp(y).unwrap_or(Ordering::Equal),
        (V::Text(x), V::Text(y)) => x.as_bytes().cmp(y.as_bytes()),
        (V::Blob(x), V::Blob(y)) => x.cmp(y),
        _ => class(a).cmp(&class(b)),
    }
}

fn cmp_int_real(i: i64, r: f64) -> Ordering {
    if r.is_nan() {
        return Ordering::Greater;
    }
    (i as f64).partial_cmp(&r).unwrap_or(Ordering::Equal)
}

/// The ordering used by `ORDER BY` and `DISTINCT`: NULL first, then [`order`].
pub fn order_nulls_first(a: &V, b: &V) -> Ordering {
    match (a.is_null(), b.is_null()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        _ => order(a, b),
    }
}

/// `a <op> b` with NULL propagation.
pub fn compare(a: &V, b: &V) -> Option<Ordering> {
    if a.is_null() || b.is_null() {
        return None;
    }
    Some(order(a, b))
}

/// `a = b`.
pub fn eq(a: &V, b: &V) -> V {
    truth(compare(a, b).map(|o| o == Ordering::Equal))
}

/// `a IS b` (NULL-safe equality).
pub fn is(a: &V, b: &V) -> bool {
    match (a.is_null(), b.is_null()) {
        (true, true) => true,
        (false, false) => order(a, b) == Ordering::Equal,
        _ => false,
    }
}

/// A hashable key with SQLite's equality for `DISTINCT` and `GROUP BY`
/// (integers and equal reals collide).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Key {
    /// NULL.
    Null,
    /// A number (reals that are integers become integers).
    Int(i64),
    /// A non-integral real, by its bits.
    Real(u64),
    /// Text.
    Text(String),
    /// Bytes.
    Blob(Vec<u8>),
}

/// The [`Key`] of a value.
pub fn key(v: &V) -> Key {
    match v {
        V::Null => Key::Null,
        V::Integer(i) => Key::Int(*i),
        V::Real(x) if x.fract() == 0.0 && x.abs() < 9.0e15 => Key::Int(*x as i64),
        V::Real(x) => Key::Real(x.to_bits()),
        V::Text(t) => Key::Text(t.clone()),
        V::Blob(b) => Key::Blob(b.clone()),
        V::IntArray(a) => Key::Blob(a.iter().flat_map(|x| x.to_be_bytes()).collect()),
    }
}

/// SQLite's text form of a REAL (`%!.15g`).
pub fn real_text(x: f64) -> String {
    if x.is_nan() {
        return String::new();
    }
    if x.is_infinite() {
        return if x > 0.0 { "Inf".into() } else { "-Inf".into() };
    }
    if x == 0.0 {
        return if x.is_sign_negative() {
            "-0.0".into()
        } else {
            "0.0".into()
        };
    }
    let exp = x.abs().log10().floor() as i32;
    let s = if !(-4..15).contains(&exp) {
        // scientific: 15 significant digits, trailing zeros removed
        let m = format!("{:.14e}", x);
        let (mant, e) = m.split_once('e').expect("exponent");
        let mant = trim_zeros(mant);
        let mant = if mant.contains('.') {
            mant
        } else {
            format!("{mant}.0")
        };
        let e: i32 = e.parse().unwrap_or(0);
        format!("{mant}e{}{:02}", if e < 0 { '-' } else { '+' }, e.abs())
    } else {
        let decimals = (14 - exp).max(0) as usize;
        let f = format!("{:.*}", decimals, x);
        let f = trim_zeros(&f);
        if f.contains('.') {
            f
        } else {
            format!("{f}.0")
        }
    };
    s
}

fn trim_zeros(s: &str) -> String {
    if !s.contains('.') {
        return s.to_string();
    }
    let t = s.trim_end_matches('0');
    let t = t
        .strip_suffix('.')
        .map_or(t.to_string(), |x| format!("{x}.0"));
    t
}

/// `CAST(v AS TEXT)`.
pub fn to_text(v: &V) -> V {
    match v {
        V::Null => V::Null,
        V::Integer(i) => V::Text(i.to_string()),
        V::Real(x) => V::Text(real_text(*x)),
        V::Text(t) => V::Text(t.clone()),
        V::Blob(b) => V::Text(String::from_utf8_lossy(b).into_owned()),
        V::IntArray(_) => V::Null,
    }
}

/// The text of a value for string functions (NULL stays NULL).
pub fn text_of(v: &V) -> Option<String> {
    match to_text(v) {
        V::Text(t) => Some(t),
        _ => None,
    }
}

/// `CAST(v AS REAL)`.
pub fn to_real(v: &V) -> V {
    match v {
        V::Null => V::Null,
        V::Integer(i) => V::Real(*i as f64),
        V::Real(x) => V::Real(*x),
        V::Text(t) => V::Real(text_number(t)),
        V::Blob(_) | V::IntArray(_) => V::Real(0.0),
    }
}

/// The numeric operand of an arithmetic operator.
fn num(v: &V) -> Option<V> {
    match v {
        V::Null => None,
        V::Integer(_) | V::Real(_) => Some(v.clone()),
        V::Text(t) => Some(V::Real(text_number(t))),
        _ => Some(V::Integer(0)),
    }
}

/// `a + b`, `a - b`, `a * b`: integers stay integers until they overflow.
pub fn arith(op: char, a: &V, b: &V) -> V {
    let (Some(a), Some(b)) = (num(a), num(b)) else {
        return V::Null;
    };
    match (&a, &b) {
        (V::Integer(x), V::Integer(y)) => {
            let r = match op {
                '+' => x.checked_add(*y),
                '-' => x.checked_sub(*y),
                _ => x.checked_mul(*y),
            };
            match r {
                Some(r) => V::Integer(r),
                None => real_arith(op, *x as f64, *y as f64),
            }
        }
        _ => real_arith(op, a.as_f64().unwrap_or(0.0), b.as_f64().unwrap_or(0.0)),
    }
}

fn real_arith(op: char, x: f64, y: f64) -> V {
    let r = match op {
        '+' => x + y,
        '-' => x - y,
        _ => x * y,
    };
    if r.is_nan() {
        V::Null
    } else {
        V::Real(r)
    }
}

/// `a / b` with a REAL dividend (division by zero is NULL).
pub fn div(a: &V, b: &V) -> V {
    let (Some(a), Some(b)) = (num(a), num(b)) else {
        return V::Null;
    };
    let y = b.as_f64().unwrap_or(0.0);
    if y == 0.0 {
        return V::Null;
    }
    let r = a.as_f64().unwrap_or(0.0) / y;
    if r.is_nan() {
        V::Null
    } else {
        V::Real(r)
    }
}

/// `-a`.
pub fn neg(a: &V) -> V {
    match num(a) {
        None => V::Null,
        Some(V::Integer(i)) => match i.checked_neg() {
            Some(n) => V::Integer(n),
            None => V::Real(-(i as f64)),
        },
        Some(v) => V::Real(-v.as_f64().unwrap_or(0.0)),
    }
}

/// `abs(a)`.
pub fn abs(a: &V) -> V {
    match num(a) {
        None => V::Null,
        Some(V::Integer(i)) => match i.checked_abs() {
            Some(n) => V::Integer(n),
            None => V::Real((i as f64).abs()),
        },
        Some(v) => V::Real(v.as_f64().unwrap_or(0.0).abs()),
    }
}

/// `length(a)`: characters of text, bytes of a blob, digits of a number.
pub fn length(a: &V) -> V {
    match a {
        V::Null => V::Null,
        V::Blob(b) => V::Integer(b.len() as i64),
        other => match text_of(other) {
            Some(t) => V::Integer(t.chars().count() as i64),
            None => V::Null,
        },
    }
}

/// `instr(a, b)`: the 1-based character position of `b` in `a`, 0 when absent.
pub fn instr(a: &V, b: &V) -> V {
    let (Some(a), Some(b)) = (text_of(a), text_of(b)) else {
        return V::Null;
    };
    match a.find(&b) {
        Some(byte) => V::Integer(a[..byte].chars().count() as i64 + 1),
        None => V::Integer(0),
    }
}

/// `substr(a, start [, len])` with SQLite's 1-based, negative-from-the-end rules.
pub fn substr(a: &V, start: i64, len: Option<i64>) -> V {
    let Some(s) = text_of(a) else {
        return V::Null;
    };
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len() as i64;
    let mut p1 = start;
    let mut p2 = len.unwrap_or(n.max(0) + 1);
    if p1 < 0 {
        p1 += n;
        if p1 < 0 {
            p2 += p1;
            p1 = 0;
        }
    } else if p1 > 0 {
        p1 -= 1;
    } else if p2 > 0 {
        p2 -= 1;
    }
    if p2 < 0 {
        p1 += p2;
        p2 = -p2;
        if p1 < 0 {
            p2 += p1;
            p1 = 0;
        }
    }
    let from = p1.clamp(0, n) as usize;
    let to = (p1 + p2).clamp(0, n) as usize;
    if from >= to {
        return V::Text(String::new());
    }
    V::Text(chars[from..to].iter().collect())
}

/// `a || b`.
pub fn concat(a: &V, b: &V) -> V {
    match (text_of(a), text_of(b)) {
        (Some(x), Some(y)) => V::Text(x + &y),
        _ => V::Null,
    }
}

/// The JSON text of one value.
pub fn json_value(v: &V) -> String {
    match v {
        V::Null => "null".into(),
        V::Integer(i) => i.to_string(),
        V::Real(x) => {
            let t = real_text(*x);
            if t == "Inf" || t == "-Inf" || t.is_empty() {
                "null".into()
            } else {
                t
            }
        }
        V::Text(t) => tm_exec::decode::json_string(t),
        V::Blob(_) | V::IntArray(_) => "null".into(),
    }
}

/// `json_array(vs…)`.
pub fn json_array(vs: &[V]) -> V {
    let parts: Vec<String> = vs.iter().map(json_value).collect();
    V::Text(format!("[{}]", parts.join(",")))
}

/// The elements of `json_each(v)`: an array's elements, or the value itself.
pub fn json_each(v: &V) -> Vec<V> {
    use tm_exec::decode::{parse_json, Json};
    match v {
        V::Null => Vec::new(),
        V::Text(t) => match parse_json(t) {
            Some(Json::Arr(items)) => items
                .into_iter()
                .map(|j| match j {
                    Json::Null => V::Null,
                    Json::Int(i) => V::Integer(i),
                    Json::Real(r) => V::Real(r),
                    Json::Str(s) => V::Text(s),
                    Json::Arr(_) => V::Null,
                })
                .collect(),
            Some(Json::Int(i)) => vec![V::Integer(i)],
            Some(Json::Real(r)) => vec![V::Real(r)],
            Some(Json::Str(s)) => vec![V::Text(s)],
            Some(Json::Null) => vec![V::Null],
            None => Vec::new(),
        },
        other => vec![other.clone()],
    }
}

/// The running state of the aggregates SQLite computes itself.
#[derive(Clone, Debug, Default)]
pub struct Sum {
    int: Option<i64>,
    real: Option<f64>,
    any: bool,
    overflow: bool,
    n: u64,
}

impl Sum {
    /// Adds one value (NULLs are skipped).
    pub fn add(&mut self, v: &V) {
        match v {
            V::Null => {}
            V::Integer(i) => {
                self.n += 1;
                self.any = true;
                match self.real.as_mut() {
                    Some(r) => *r += *i as f64,
                    None => match self.int.unwrap_or(0).checked_add(*i) {
                        Some(s) => self.int = Some(s),
                        None => self.overflow = true,
                    },
                }
            }
            other => {
                self.n += 1;
                self.any = true;
                let x = match other {
                    V::Real(x) => *x,
                    V::Text(t) => text_number(t),
                    _ => 0.0,
                };
                let base = self.real.unwrap_or(self.int.unwrap_or(0) as f64);
                self.real = Some(base + x);
            }
        }
    }

    /// `sum(…)`: NULL over no value, an integer while every input is one.
    pub fn sum(&self) -> Result<V, String> {
        if !self.any {
            return Ok(V::Null);
        }
        if let Some(r) = self.real {
            return Ok(V::Real(r));
        }
        if self.overflow {
            return Err("integer overflow".into());
        }
        Ok(V::Integer(self.int.unwrap_or(0)))
    }

    /// `avg(…)`.
    pub fn avg(&self) -> V {
        if self.n == 0 {
            return V::Null;
        }
        let total = self.real.unwrap_or(self.int.unwrap_or(0) as f64);
        V::Real(total / self.n as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reals_print_like_sqlite() {
        for (x, s) in [
            (1.0, "1.0"),
            (1.5, "1.5"),
            (0.1, "0.1"),
            (-2.25, "-2.25"),
            (1e20, "1.0e+20"),
            (1.5e-7, "1.5e-07"),
            (123456789012345.0, "123456789012345.0"),
            (1.0 / 3.0, "0.333333333333333"),
        ] {
            assert_eq!(real_text(x), s, "{x}");
        }
    }

    #[test]
    fn substr_follows_sqlite() {
        let t = V::Text("hello".into());
        assert_eq!(substr(&t, 1, Some(3)), V::Text("hel".into()));
        assert_eq!(substr(&t, -3, None), V::Text("llo".into()));
        assert_eq!(substr(&t, 0, Some(2)), V::Text("h".into()));
        assert_eq!(substr(&t, 2, None), V::Text("ello".into()));
    }

    #[test]
    fn ordering_by_storage_class() {
        assert_eq!(order(&V::Integer(2), &V::Real(1.5)), Ordering::Greater);
        assert_eq!(order(&V::Integer(9), &V::Text("1".into())), Ordering::Less);
        assert_eq!(key(&V::Real(2.0)), key(&V::Integer(2)));
    }
}
