//! JSON forms of terms and Cypher parameters, shared by both bindings.
//!
//! A term is a JSON string (a plain string), a boolean, a number (an integer is an
//! `xsd:integer`, a fractional number an `xsd:double`), or an object with one of
//! the keys `iri`, `node`, `bnode`, `stmt`, `tx`, `$int`, or `lex` with `datatype`
//! or `lang`. Values that JSON cannot carry exactly (integers beyond 2^53, whole
//! doubles, dates, decimals) leave as `{"lex", "datatype"}`.

use serde_json::{json, Map, Value as J};
use tiramemsu::value::{parse_date, parse_datetime};
use tiramemsu::{CypherParams, CypherValue, Eid, Error, TxId, Value};

use crate::{arg, Res};

const MAX_SAFE: u64 = 1 << 53;

/// A term from its JSON form.
pub fn value_from_json(j: &J) -> Res<Value> {
    Ok(match j {
        J::String(s) => Value::Str(s.clone()),
        J::Bool(b) => Value::Bool(*b),
        J::Number(n) => match n.as_i64() {
            Some(i) => Value::Int(i),
            None => match n.as_f64() {
                Some(f) if f.is_finite() => Value::Double(f),
                _ => return Err(arg(format!("number {n} is not a term"))),
            },
        },
        J::Object(o) => object_value(o)?,
        J::Null | J::Array(_) => return Err(arg(format!("{j} is not a term"))),
    })
}

fn object_value(o: &Map<String, J>) -> Res<Value> {
    let text = |k: &str| o.get(k).and_then(J::as_str);
    let num = |k: &str| o.get(k).and_then(J::as_u64);
    if let Some(iri) = text("iri") {
        return Ok(Value::iri(iri));
    }
    if let Some(n) = num("node") {
        return Ok(Value::Node(n));
    }
    if let Some(n) = num("bnode") {
        return Ok(Value::BNode(n));
    }
    if let Some(n) = num("stmt") {
        return Ok(Value::Stmt(Eid::new(n)));
    }
    if let Some(n) = num("tx") {
        return Ok(Value::Tx(TxId(n)));
    }
    if let Some(d) = text("$int") {
        return Ok(Value::big_integer(d));
    }
    if let Some(lex) = text("lex") {
        return Ok(Value::literal(lex, text("datatype"), text("lang")));
    }
    Err(arg(format!("{} is not a term", J::Object(o.clone()))))
}

/// The JSON form of a term.
pub fn value_to_json(v: &Value) -> J {
    match v {
        Value::Iri(s) => json!({ "iri": s }),
        Value::Node(n) => json!({ "node": n }),
        Value::BNode(n) => json!({ "bnode": n }),
        Value::Stmt(e) => json!({ "stmt": e.n() }),
        Value::Tx(t) => json!({ "tx": t.0 }),
        Value::Int(i) if i.unsigned_abs() <= MAX_SAFE => json!(i),
        Value::Int(i) => json!({ "$int": i.to_string() }),
        Value::Bool(b) => json!(b),
        Value::Str(s) => json!(s),
        Value::LangStr { lex, lang } => json!({ "lex": lex, "lang": lang }),
        Value::Double(x) if x.is_finite() && x.fract() != 0.0 => json!(x),
        Value::Typed { .. }
        | Value::DateTime { .. }
        | Value::Date(_)
        | Value::Decimal(_)
        | Value::Double(_) => {
            json!({ "lex": v.lexical(), "datatype": v.datatype() })
        }
    }
}

/// An optional epoch-millisecond bound: a number, or an RFC 3339 date or date-time.
pub fn time_from_json(j: &J) -> Res<Option<i64>> {
    match j {
        J::Null => Ok(None),
        J::Number(n) => n
            .as_i64()
            .map(Some)
            .ok_or_else(|| arg(format!("time {n} is not an integer of milliseconds"))),
        J::String(s) => parse_datetime(s)
            .map(|(ms, _)| ms)
            .or_else(|| parse_date(s).map(|d| d * 86_400_000))
            .map(Some)
            .ok_or_else(|| arg(format!("time {s:?} is not a date or date-time"))),
        _ => Err(arg(format!("{j} is not a time"))),
    }
}

/// A statement id: a number, `{"stmt": n}`, or `{"ref": name}` resolved by `refs`.
pub fn eid_from_json(j: &J, refs: &std::collections::HashMap<String, Eid>) -> Res<Eid> {
    match j {
        J::Number(n) => n
            .as_u64()
            .map(Eid::new)
            .ok_or_else(|| arg(format!("{n} is not a statement id"))),
        J::Object(o) => {
            if let Some(n) = o.get("stmt").and_then(J::as_u64) {
                Ok(Eid::new(n))
            } else if let Some(name) = o.get("ref").and_then(J::as_str) {
                refs.get(name)
                    .copied()
                    .ok_or_else(|| arg(format!("no earlier operation is named {name:?}")))
            } else {
                Err(arg(format!("{j} is not a statement id")))
            }
        }
        _ => Err(arg(format!("{j} is not a statement id"))),
    }
}

/// Cypher parameters from a JSON object.
pub fn params_from_json(j: &J) -> Res<CypherParams> {
    let mut out = CypherParams::new();
    match j {
        J::Null => {}
        J::Object(o) => {
            for (k, v) in o {
                out.insert(k.clone(), cypher_from_json(v)?);
            }
        }
        _ => return Err(arg("params must be an object")),
    }
    Ok(out)
}

fn cypher_from_json(j: &J) -> Res<CypherValue> {
    Ok(match j {
        J::Null => CypherValue::Null,
        J::Bool(b) => CypherValue::Boolean(*b),
        J::Number(n) => match (n.as_i64(), n.as_f64()) {
            (Some(i), _) => CypherValue::Integer(i),
            (None, Some(f)) => CypherValue::Float(f),
            _ => return Err(arg(format!("number {n} is not a parameter"))),
        },
        J::String(s) => CypherValue::String(s.clone()),
        J::Array(a) => CypherValue::List(a.iter().map(cypher_from_json).collect::<Res<_>>()?),
        J::Object(o) => {
            if let Some(d) = o.get("$int").and_then(J::as_str) {
                let i = d
                    .parse::<i64>()
                    .map_err(|_| arg(format!("{d:?} does not fit a 64-bit integer")))?;
                return Ok(CypherValue::Integer(i));
            }
            CypherValue::Map(
                o.iter()
                    .map(|(k, v)| Ok((k.clone(), cypher_from_json(v)?)))
                    .collect::<Res<_>>()?,
            )
        }
    })
}

/// The bridge's own error for a rejected argument, kept apart from database errors.
#[derive(Debug)]
pub struct ArgError(pub String);

impl std::fmt::Display for ArgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ArgError {}

/// Wraps an argument error so it can travel through a transaction body.
pub fn into_core(e: crate::BindError) -> Error {
    match e {
        crate::BindError::Arg(m) => Error::custom(ArgError(m)),
        crate::BindError::Db(e) => e,
    }
}
