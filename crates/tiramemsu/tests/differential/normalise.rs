//! Result normalisation of the differential suite: SPARQL terms and Cypher values
//! become rows of canonical values.

use std::collections::BTreeSet;

use tiramemsu::*;

/// A canonical cell.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Canon {
    /// A SPARQL unbound value or a Cypher `null`.
    Missing,
    /// A node, blank node, anonymous node, IRI or statement: its canonical lexical form.
    Entity(String),
    /// A number by value (canonical decimal text).
    Num(String),
    /// A string.
    Str(String),
    /// A boolean.
    Bool(bool),
    /// A date-time: instant and offset (`None` without a timezone).
    DateTime(i64, Option<i16>),
    /// A date.
    Date(i64),
    /// A transaction number.
    Tx(u64),
    /// A list, in order.
    List(Vec<Canon>),
}

fn num_text(x: f64) -> String {
    if x == x.trunc() && x.abs() < 1e15 {
        format!("{}", x as i64)
    } else {
        format!("{x:?}")
    }
}

/// Canonical entity text of an IRI string (skolem IRIs parse back to their id).
fn entity(iri: &str) -> Canon {
    Canon::Entity(Value::iri(iri).canonical().lexical())
}

/// A SPARQL term.
pub fn from_term(v: &Option<Value>) -> Canon {
    let Some(v) = v else { return Canon::Missing };
    match v.canonical() {
        Value::Iri(i) => entity(&i),
        e @ (Value::Node(_) | Value::BNode(_) | Value::Stmt(_)) => Canon::Entity(e.lexical()),
        Value::Tx(t) => Canon::Tx(t.0),
        Value::Int(i) => Canon::Num(i.to_string()),
        Value::Double(x) => Canon::Num(num_text(x)),
        Value::Decimal(s) => Canon::Num(s.parse::<f64>().map_or(s, num_text)),
        Value::Bool(b) => Canon::Bool(b),
        Value::Str(s) => Canon::Str(s),
        Value::LangStr { lex, .. } => Canon::Str(lex),
        Value::DateTime { ms, tz } => Canon::DateTime(ms, tz),
        Value::Date(d) => Canon::Date(d),
        Value::Typed { lex, datatype } if datatype == vocab::XSD_INTEGER => Canon::Num(lex),
        Value::Typed { lex, .. } => Canon::Str(lex),
    }
}

/// A Cypher value. `is_tx` reads an Integer as a transaction number.
pub fn from_cypher(v: &CypherValue, is_tx: bool) -> Canon {
    match v {
        CypherValue::Null => Canon::Missing,
        CypherValue::Boolean(b) => Canon::Bool(*b),
        CypherValue::Integer(i) if is_tx => Canon::Tx(*i as u64),
        CypherValue::Integer(i) => Canon::Num(i.to_string()),
        CypherValue::Float(x) => Canon::Num(num_text(*x)),
        CypherValue::String(s) if s.starts_with("urn:tiramemsu:") => entity(s),
        CypherValue::String(s) => Canon::Str(s.clone()),
        CypherValue::Date(d) => Canon::Date(*d),
        CypherValue::DateTime { ms, tz } => Canon::DateTime(*ms, Some(*tz)),
        CypherValue::LocalDateTime(ms) => Canon::DateTime(*ms, None),
        CypherValue::List(l) => Canon::List(l.iter().map(|x| from_cypher(x, false)).collect()),
        CypherValue::Node(n) => Canon::Entity(n.term.canonical().lexical()),
        CypherValue::Relationship(r) => Canon::Entity(Value::Stmt(r.eid).lexical()),
        other => Canon::Str(format!("{other:?}")),
    }
}

/// How rows are compared.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Mode {
    /// `set`: duplicates are removed on both sides.
    pub set: bool,
    /// `ordered`: row order is compared.
    pub ordered: bool,
}

/// Rows after applying the mode (`unordered` sorts, `set` deduplicates).
pub fn apply_mode(mut rows: Vec<Vec<Canon>>, mode: Mode) -> Vec<Vec<Canon>> {
    if mode.set {
        let mut seen = BTreeSet::new();
        rows.retain(|r| seen.insert(r.clone()));
    }
    if !mode.ordered {
        rows.sort();
    }
    rows
}

/// A human-readable difference report (rows only on one side), or `None` when equal.
pub fn diff(sparql: &[Vec<Canon>], cypher: &[Vec<Canon>]) -> Option<String> {
    if sparql == cypher {
        return None;
    }
    let mut only_s: Vec<&Vec<Canon>> = Vec::new();
    let mut rest: Vec<&Vec<Canon>> = cypher.iter().collect();
    for r in sparql {
        match rest.iter().position(|x| *x == r) {
            Some(i) => {
                rest.remove(i);
            }
            None => only_s.push(r),
        }
    }
    let mut out = String::new();
    for r in only_s {
        out.push_str(&format!("  only in SPARQL: {r:?}\n"));
    }
    for r in rest {
        out.push_str(&format!("  only in Cypher: {r:?}\n"));
    }
    if out.is_empty() {
        out.push_str("  same rows, different order\n");
    }
    Some(out)
}
