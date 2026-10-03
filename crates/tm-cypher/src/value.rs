//! Cypher values: the internal [`Val`] the interpreter computes with, Cypher
//! equality and ordering, and the public [`CypherValue`] results
//! (`lat.md/query#Front Ends#Cypher`, design Decision 10).

use std::cmp::Ordering;
use std::collections::BTreeMap;

use tm_core::value::{format_date, format_datetime};
use tm_core::{Eid, Value};

/// A value during evaluation. Graph entities are references; they are turned into
/// [`CypherValue`]s with their labels and properties when a result is built.
#[derive(Clone, Debug)]
pub enum Val {
    /// `null`.
    Null,
    /// Boolean.
    Bool(bool),
    /// Integer.
    Int(i64),
    /// Float.
    Float(f64),
    /// String.
    Str(String),
    /// Date: days since 1970-01-01.
    Date(i64),
    /// DateTime: the instant and the offset in minutes it keeps.
    DateTime {
        /// Epoch milliseconds (UTC).
        ms: i64,
        /// Offset in minutes.
        tz: i16,
    },
    /// LocalDateTime: the local reading as milliseconds since 1970-01-01T00:00.
    LocalDateTime(i64),
    /// List.
    List(Vec<Val>),
    /// Map, ordered by key.
    Map(BTreeMap<String, Val>),
    /// A node: an IRI, anonymous node, blank node, or a statement in node form.
    Node(Value),
    /// A relationship (statement eid).
    Rel(Eid),
    /// A path: nodes and relationships alternating, starting with a node.
    Path(Vec<Val>),
}

impl Val {
    /// The Cypher type name used in messages.
    pub fn type_name(&self) -> &'static str {
        match self {
            Val::Null => "Null",
            Val::Bool(_) => "Boolean",
            Val::Int(_) => "Integer",
            Val::Float(_) => "Float",
            Val::Str(_) => "String",
            Val::Date(_) => "Date",
            Val::DateTime { .. } => "DateTime",
            Val::LocalDateTime(_) => "LocalDateTime",
            Val::List(_) => "List",
            Val::Map(_) => "Map",
            Val::Node(_) => "Node",
            Val::Rel(_) => "Relationship",
            Val::Path(_) => "Path",
        }
    }

    /// True for `null`.
    pub fn is_null(&self) -> bool {
        matches!(self, Val::Null)
    }

    /// Decodes a stored literal term (never an entity) into a Cypher value.
    /// Entities read in property position become their IRI strings (`isEdge false`).
    pub fn from_literal(v: &Value) -> Val {
        match v {
            Value::Int(i) => Val::Int(*i),
            Value::Bool(b) => Val::Bool(*b),
            Value::Double(x) => Val::Float(*x),
            Value::Decimal(s) => Val::Float(s.parse().unwrap_or(f64::NAN)),
            Value::Str(s) => Val::Str(s.clone()),
            Value::LangStr { lex, .. } => Val::Str(lex.clone()),
            Value::Typed { lex, datatype } => {
                if datatype == tm_core::vocab::XSD_INTEGER {
                    if let Ok(i) = lex.parse::<i64>() {
                        return Val::Int(i);
                    }
                }
                Val::Str(lex.clone())
            }
            Value::Date(d) => Val::Date(*d),
            Value::DateTime { ms, tz: Some(tz) } => Val::DateTime { ms: *ms, tz: *tz },
            Value::DateTime { ms, tz: None } => Val::LocalDateTime(*ms),
            Value::Tx(t) => Val::Int(t.0 as i64),
            other => Val::Str(other.lexical()),
        }
    }

    /// The entity for a node-position term.
    pub fn from_entity(v: &Value) -> Val {
        match v {
            Value::Stmt(_) | Value::Iri(_) | Value::Node(_) | Value::BNode(_) => {
                Val::Node(v.clone())
            }
            other => Val::from_literal(other),
        }
    }

    /// The stored term of a scalar value, or `None` for lists, maps and entities.
    pub fn to_term(&self) -> Option<Value> {
        Some(match self {
            Val::Bool(b) => Value::Bool(*b),
            Val::Int(i) => Value::Int(*i).canonical(),
            Val::Float(x) => Value::Double(*x),
            Val::Str(s) => Value::Str(s.clone()),
            Val::Date(d) => Value::Date(*d),
            Val::DateTime { ms, tz } => Value::DateTime {
                ms: *ms,
                tz: Some(*tz),
            },
            Val::LocalDateTime(ms) => Value::DateTime { ms: *ms, tz: None },
            _ => return None,
        })
    }

    /// Text of a temporal or scalar value as Cypher's `toString` prints it.
    pub fn to_display(&self) -> String {
        match self {
            Val::Null => "null".into(),
            Val::Bool(b) => b.to_string(),
            Val::Int(i) => i.to_string(),
            Val::Float(x) => fmt_float(*x),
            Val::Str(s) => s.clone(),
            Val::Date(d) => format_date(*d),
            Val::DateTime { ms, tz } => format_datetime(*ms, Some(*tz)),
            Val::LocalDateTime(ms) => format_datetime(*ms, None),
            Val::List(l) => format!(
                "[{}]",
                l.iter().map(Val::to_repr).collect::<Vec<_>>().join(", ")
            ),
            Val::Map(m) => format!(
                "{{{}}}",
                m.iter()
                    .map(|(k, v)| format!("{k}: {}", v.to_repr()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Val::Node(v) => v.lexical(),
            Val::Rel(e) => format!("{}{}", tm_core::vocab::SKOLEM_STMT, e.n()),
            Val::Path(_) => "<path>".into(),
        }
    }

    /// Text with strings quoted (inside collections).
    pub fn to_repr(&self) -> String {
        match self {
            Val::Str(s) => format!("'{s}'"),
            other => other.to_display(),
        }
    }
}

/// Cypher's float printing: integral values keep `.0`.
pub fn fmt_float(x: f64) -> String {
    if x.is_nan() {
        "NaN".into()
    } else if x.is_infinite() {
        if x > 0.0 {
            "Infinity".into()
        } else {
            "-Infinity".into()
        }
    } else if x == x.trunc() && x.abs() < 1e15 {
        format!("{x:.1}")
    } else {
        format!("{x}")
    }
}

fn num(v: &Val) -> Option<f64> {
    match v {
        Val::Int(i) => Some(*i as f64),
        Val::Float(x) => Some(*x),
        _ => None,
    }
}

/// Cypher `=`: `None` is the unknown (`null`) result.
pub fn eq3(a: &Val, b: &Val) -> Option<bool> {
    match (a, b) {
        (Val::Null, _) | (_, Val::Null) => None,
        (Val::Int(x), Val::Int(y)) => Some(x == y),
        (Val::Int(_) | Val::Float(_), Val::Int(_) | Val::Float(_)) => {
            let (x, y) = (num(a)?, num(b)?);
            if x.is_nan() || y.is_nan() {
                Some(false)
            } else if let (Val::Int(i), Val::Float(f)) | (Val::Float(f), Val::Int(i)) = (a, b) {
                Some((*i as f64) == *f && (*f).abs() < 9.0e18 && (*f as i64) == *i)
            } else {
                Some(x == y)
            }
        }
        (Val::Bool(x), Val::Bool(y)) => Some(x == y),
        (Val::Str(x), Val::Str(y)) => Some(x == y),
        (Val::Date(x), Val::Date(y)) => Some(x == y),
        (Val::DateTime { ms: x, .. }, Val::DateTime { ms: y, .. }) => Some(x == y),
        (Val::LocalDateTime(x), Val::LocalDateTime(y)) => Some(x == y),
        (Val::List(x), Val::List(y)) => {
            if x.len() != y.len() {
                return Some(false);
            }
            let mut unknown = false;
            for (p, q) in x.iter().zip(y) {
                match eq3(p, q) {
                    Some(false) => return Some(false),
                    None => unknown = true,
                    Some(true) => {}
                }
            }
            if unknown {
                None
            } else {
                Some(true)
            }
        }
        (Val::Map(x), Val::Map(y)) => {
            if x.len() != y.len() || !x.keys().eq(y.keys()) {
                return Some(false);
            }
            let mut unknown = false;
            for (k, p) in x {
                match eq3(p, &y[k]) {
                    Some(false) => return Some(false),
                    None => unknown = true,
                    Some(true) => {}
                }
            }
            if unknown {
                None
            } else {
                Some(true)
            }
        }
        (Val::Node(x), Val::Node(y)) => Some(x == y),
        (Val::Rel(x), Val::Rel(y)) => Some(x == y),
        // the dual view: a statement in node form equals the relationship
        (Val::Node(Value::Stmt(x)), Val::Rel(y)) | (Val::Rel(y), Val::Node(Value::Stmt(x))) => {
            Some(x == y)
        }
        (Val::Path(x), Val::Path(y)) => {
            if x.len() != y.len() {
                return Some(false);
            }
            let mut unknown = false;
            for (p, q) in x.iter().zip(y) {
                match eq3(p, q) {
                    Some(false) => return Some(false),
                    None => unknown = true,
                    Some(true) => {}
                }
            }
            if unknown {
                None
            } else {
                Some(true)
            }
        }
        _ => Some(false),
    }
}

/// Cypher `<`-style comparison: `None` when the values are not comparable or a
/// comparison involves `null` or `NaN`.
pub fn cmp3(a: &Val, b: &Val) -> Option<Ordering> {
    match (a, b) {
        (Val::Null, _) | (_, Val::Null) => None,
        (Val::Int(x), Val::Int(y)) => Some(x.cmp(y)),
        (Val::Int(_) | Val::Float(_), Val::Int(_) | Val::Float(_)) => num(a)?.partial_cmp(&num(b)?),
        (Val::Bool(x), Val::Bool(y)) => Some(x.cmp(y)),
        (Val::Str(x), Val::Str(y)) => Some(x.cmp(y)),
        (Val::Date(x), Val::Date(y)) => Some(x.cmp(y)),
        (Val::DateTime { ms: x, .. }, Val::DateTime { ms: y, .. }) => Some(x.cmp(y)),
        (Val::LocalDateTime(x), Val::LocalDateTime(y)) => Some(x.cmp(y)),
        (Val::List(x), Val::List(y)) => {
            for (p, q) in x.iter().zip(y) {
                match cmp3(p, q) {
                    Some(Ordering::Equal) => {}
                    other => return other,
                }
            }
            Some(x.len().cmp(&y.len()))
        }
        _ => None,
    }
}

fn rank(v: &Val) -> u8 {
    match v {
        Val::Map(_) => 0,
        Val::Node(_) => 1,
        Val::Rel(_) => 2,
        Val::List(_) => 3,
        Val::Path(_) => 4,
        Val::DateTime { .. } => 5,
        Val::LocalDateTime(_) => 6,
        Val::Date(_) => 7,
        Val::Str(_) => 8,
        Val::Bool(_) => 9,
        Val::Int(_) | Val::Float(_) => 10,
        Val::Null => 12,
    }
}

fn entity_key(v: &Val) -> (u8, String) {
    match v {
        Val::Node(Value::Stmt(e)) | Val::Rel(e) => (0, format!("{:020}", e.n())),
        Val::Node(x) => (1, x.lexical()),
        _ => (2, String::new()),
    }
}

/// The total order of `ORDER BY` (ascending; `null` last).
pub fn order(a: &Val, b: &Val) -> Ordering {
    let (ra, rb) = (rank(a), rank(b));
    if ra != rb {
        return ra.cmp(&rb);
    }
    match (a, b) {
        (Val::Int(_) | Val::Float(_), Val::Int(_) | Val::Float(_)) => {
            let (x, y) = (num(a).unwrap_or(f64::NAN), num(b).unwrap_or(f64::NAN));
            match (x.is_nan(), y.is_nan()) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                _ => {
                    if let (Val::Int(i), Val::Int(j)) = (a, b) {
                        i.cmp(j)
                    } else {
                        x.partial_cmp(&y).unwrap_or(Ordering::Equal)
                    }
                }
            }
        }
        (Val::List(x), Val::List(y)) | (Val::Path(x), Val::Path(y)) => {
            for (p, q) in x.iter().zip(y) {
                let o = order(p, q);
                if o != Ordering::Equal {
                    return o;
                }
            }
            x.len().cmp(&y.len())
        }
        (Val::Map(x), Val::Map(y)) => {
            let ka: Vec<_> = x.iter().collect();
            let kb: Vec<_> = y.iter().collect();
            for (p, q) in ka.iter().zip(&kb) {
                let o = p.0.cmp(q.0).then_with(|| order(p.1, q.1));
                if o != Ordering::Equal {
                    return o;
                }
            }
            ka.len().cmp(&kb.len())
        }
        (Val::Node(_), Val::Node(_)) | (Val::Rel(_), Val::Rel(_)) => {
            entity_key(a).cmp(&entity_key(b))
        }
        _ => cmp3(a, b).unwrap_or(Ordering::Equal),
    }
}

/// A canonical key under which equivalent values (Cypher `DISTINCT`/grouping:
/// two nulls are equivalent, `1` and `1.0` are equivalent) collide.
pub fn group_key(v: &Val) -> String {
    let mut s = String::new();
    write_key(v, &mut s);
    s
}

fn write_key(v: &Val, out: &mut String) {
    use std::fmt::Write;
    match v {
        Val::Null => out.push('N'),
        Val::Bool(b) => {
            let _ = write!(out, "B{b}");
        }
        Val::Int(i) => {
            let _ = write!(out, "n{i}");
        }
        Val::Float(x) => {
            if x.is_nan() {
                out.push_str("nNaN");
            } else if *x == x.trunc() && x.abs() < 9.0e18 {
                let _ = write!(out, "n{}", *x as i64);
            } else {
                let _ = write!(out, "n{x:?}");
            }
        }
        Val::Str(s) => {
            let _ = write!(out, "S{}:{s}", s.len());
        }
        Val::Date(d) => {
            let _ = write!(out, "D{d}");
        }
        Val::DateTime { ms, .. } => {
            let _ = write!(out, "T{ms}");
        }
        Val::LocalDateTime(ms) => {
            let _ = write!(out, "L{ms}");
        }
        Val::List(l) => {
            out.push('[');
            for x in l {
                write_key(x, out);
                out.push(',');
            }
            out.push(']');
        }
        Val::Path(l) => {
            out.push('P');
            for x in l {
                write_key(x, out);
                out.push(',');
            }
            out.push(';');
        }
        Val::Map(m) => {
            out.push('{');
            for (k, x) in m {
                let _ = write!(out, "{}:{k}=", k.len());
                write_key(x, out);
                out.push(',');
            }
            out.push('}');
        }
        Val::Node(Value::Stmt(e)) | Val::Rel(e) => {
            let _ = write!(out, "E{}", e.n());
        }
        Val::Node(x) => {
            let _ = write!(out, "V{}", x.lexical());
        }
    }
}

/// A node in a result. A statement used as a node has the label `Statement`
/// first and the time properties `txAdded`, `txRetracted`, `validFrom` and
/// `validTo`.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeValue {
    /// The stored term.
    pub term: Value,
    /// The element id (IRI or skolem IRI).
    pub element_id: String,
    /// Rendered labels (`Statement` first for statements).
    pub labels: Vec<String>,
    /// Properties, without `sys:` names and temporal names.
    pub properties: BTreeMap<String, CypherValue>,
}

/// A relationship in a result. `eid` is the statement id, and the same id is
/// also a [`NodeValue`] wherever the query used it in node position.
#[derive(Clone, Debug, PartialEq)]
pub struct RelValue {
    /// The statement eid.
    pub eid: Eid,
    /// Element id (`urn:tiramemsu:stmt:<n>`).
    pub element_id: String,
    /// Rendered predicate name.
    pub rel_type: String,
    /// Start element id.
    pub start_element_id: String,
    /// End element id.
    pub end_element_id: String,
    /// Properties.
    pub properties: BTreeMap<String, CypherValue>,
}

/// A path in a result.
#[derive(Clone, Debug, PartialEq)]
pub struct PathValue {
    /// The nodes.
    pub nodes: Vec<NodeValue>,
    /// The relationships.
    pub rels: Vec<RelValue>,
}

/// A value in a result table.
///
/// Datetimes keep the offset they were stored with; a `LocalDateTime` has none.
/// Integers are 64-bit. [`to_json`](CypherValue::to_json) gives the JSON form.
#[derive(Clone, Debug, PartialEq)]
pub enum CypherValue {
    /// `null`.
    Null,
    /// Boolean.
    Boolean(bool),
    /// Integer.
    Integer(i64),
    /// Float.
    Float(f64),
    /// String.
    String(String),
    /// Date (days since 1970-01-01).
    Date(i64),
    /// DateTime with its offset.
    DateTime {
        /// Epoch ms (UTC).
        ms: i64,
        /// Offset minutes.
        tz: i16,
    },
    /// LocalDateTime.
    LocalDateTime(i64),
    /// List.
    List(Vec<CypherValue>),
    /// Map.
    Map(BTreeMap<String, CypherValue>),
    /// Node.
    Node(Box<NodeValue>),
    /// Relationship.
    Relationship(Box<RelValue>),
    /// Path.
    Path(Box<PathValue>),
}

/// A query result: columns and rows, plus the transaction report of a write.
///
/// `report` is `Some` only for queries run through `Db::cypher_write`.
///
/// # Example
///
/// ```
/// use tm_cypher::{CypherResult, CypherValue};
///
/// let r = CypherResult {
///     columns: vec!["n".into()],
///     rows: vec![vec![CypherValue::Integer(1)]],
///     report: None,
///     path_completeness: None,
/// };
/// assert_eq!(r.to_json().to_string(), r#"{"columns":["n"],"rows":[[1]]}"#);
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct CypherResult {
    /// Column names.
    pub columns: Vec<String>,
    /// Rows.
    pub rows: Vec<Vec<CypherValue>>,
    /// The report of the transaction, for queries run through `cypher_write`.
    pub report: Option<tm_core::TxReport>,
    /// How completely the query's variable-length and shortest-path searches were
    /// evaluated, merged over all of them ([`tm_ir::PathCompleteness`]): `None`
    /// when the query ran none. An unbounded `*` stopped by the configured hop cap
    /// is `StoppedAtCap`. Filled by the host (the `tiramemsu` facade).
    pub path_completeness: Option<tm_ir::PathCompleteness>,
}

impl CypherValue {
    /// The JSON encoding (design Decision 10). Integers beyond ±2^53 become
    /// `{"$int": "<digits>"}`.
    pub fn to_json(&self) -> serde_json::Value {
        use serde_json::{json, Map, Value as J};
        match self {
            CypherValue::Null => J::Null,
            CypherValue::Boolean(b) => J::Bool(*b),
            CypherValue::Integer(i) => {
                if i.unsigned_abs() > (1u64 << 53) {
                    json!({ "$int": i.to_string() })
                } else {
                    json!(i)
                }
            }
            CypherValue::Float(x) => {
                if x.is_finite() {
                    json!(x)
                } else {
                    J::String(fmt_float(*x))
                }
            }
            CypherValue::String(s) => J::String(s.clone()),
            CypherValue::Date(d) => J::String(format_date(*d)),
            CypherValue::DateTime { ms, tz } => J::String(format_datetime(*ms, Some(*tz))),
            CypherValue::LocalDateTime(ms) => J::String(format_datetime(*ms, None)),
            CypherValue::List(l) => J::Array(l.iter().map(CypherValue::to_json).collect()),
            CypherValue::Map(m) => J::Object(props_json(m)),
            CypherValue::Node(n) => {
                let mut o = Map::new();
                o.insert("elementId".into(), J::String(n.element_id.clone()));
                o.insert("labels".into(), json!(n.labels));
                o.insert("properties".into(), J::Object(props_json(&n.properties)));
                J::Object(o)
            }
            CypherValue::Relationship(r) => rel_json(r),
            CypherValue::Path(p) => json!({
                "nodes": p.nodes.iter().map(|n| CypherValue::Node(Box::new(n.clone())).to_json()).collect::<Vec<_>>(),
                "relationships": p.rels.iter().map(rel_json).collect::<Vec<_>>(),
            }),
        }
    }
}

fn props_json(m: &BTreeMap<String, CypherValue>) -> serde_json::Map<String, serde_json::Value> {
    m.iter().map(|(k, v)| (k.clone(), v.to_json())).collect()
}

fn rel_json(r: &RelValue) -> serde_json::Value {
    serde_json::json!({
        "elementId": r.element_id,
        "type": r.rel_type,
        "startNodeElementId": r.start_element_id,
        "endNodeElementId": r.end_element_id,
        "properties": serde_json::Value::Object(props_json(&r.properties)),
    })
}

impl CypherResult {
    /// The JSON encoding: `{"columns": […], "rows": [[…]]}`.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "columns": self.columns,
            "rows": self.rows.iter().map(|r| r.iter().map(CypherValue::to_json).collect::<Vec<_>>()).collect::<Vec<_>>(),
        })
    }
}
