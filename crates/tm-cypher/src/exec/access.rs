//! Reading the graph: property lookups, labels, keys, statement ends, and turning
//! entity references into result values. Everything here is an IR query through the
//! [`crate::runner::Runner`], evaluated under an explicit view.

use std::collections::BTreeMap;

use tm_core::vocab::{RDF, SYS, TM};
use tm_core::{Eid, Value};
use tm_ir::vocab as irv;
use tm_ir::{Expr, Func, IrQuery, Key, Op, Params, TermOrVar, TriplePattern, View};

use super::{core, Exec, Flags};
use crate::error::{CResult, CypherError};
use crate::runner::Rows;
use crate::value::{CypherValue, NodeValue, PathValue, RelValue, Val};

/// `rdf:type`.
pub const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

/// The temporal names that denote statement metadata.
pub const TEMPORAL: [&str; 6] = [
    "txAdded",
    "txRetracted",
    "addedAt",
    "retractedAt",
    "validFrom",
    "validTo",
];

/// The virtual predicate behind a temporal name on a statement, `None` for any
/// other name.
pub fn temporal_iri(name: &str) -> Option<&'static str> {
    Some(match name {
        "txAdded" => irv::TM_TX_ADDED,
        "txRetracted" => irv::TM_TX_RETRACTED,
        "addedAt" => irv::TM_ADDED_AT,
        "retractedAt" => irv::TM_RETRACTED_AT,
        "validFrom" => irv::TM_VALID_FROM,
        "validTo" => irv::TM_VALID_TO,
        _ => return None,
    })
}

/// True for a temporal name whose value is transaction time (read-only).
pub fn is_tx_time(name: &str) -> bool {
    matches!(name, "txAdded" | "txRetracted" | "addedAt" | "retractedAt")
}

/// Payload bit that marks the synthetic eid of a virtual layer hop.
const VIRTUAL_EID: u64 = 1 << 59;

/// The relationship value of a virtual layer hop (`sys:subject`, `sys:object` or
/// `sys:predicate`, kind 0, 1, 2) of statement `base`: a synthetic eid that
/// `Exec::stmt_parts` resolves to the statement's part, in the stored direction.
pub fn virtual_eid(base: Eid, kind: u8) -> Eid {
    Eid::new(VIRTUAL_EID | (u64::from(kind) << 50) | base.n())
}

/// The statement and kind of a synthetic virtual-hop eid.
pub fn virtual_parts(e: Eid) -> Option<(Eid, u8)> {
    let n = e.n();
    (n & VIRTUAL_EID != 0).then(|| (Eid::new(n & ((1 << 50) - 1)), ((n >> 50) & 3) as u8))
}

/// A term as an IR position.
pub fn tv(v: &Value) -> TermOrVar {
    match v {
        Value::Stmt(e) => TermOrVar::Id(e.oid()),
        other => TermOrVar::Const(other.clone()),
    }
}

/// True for a statement in either form.
pub fn stmt_of(v: &Val) -> Option<Eid> {
    match v {
        Val::Rel(e) | Val::Node(Value::Stmt(e)) => Some(*e),
        _ => None,
    }
}

/// True when the IRI is in the `sys:` namespace.
pub fn is_sys(iri: &str) -> bool {
    iri.starts_with(SYS)
}

/// The retract kind as Cypher renders it.
pub fn retract_kind_name(k: i64) -> Option<&'static str> {
    Some(match k {
        0 => "explicit",
        1 => "cascade",
        2 => "supersede",
        3 => "cardinality",
        _ => return None,
    })
}

impl Exec<'_> {
    /// Runs an IR operator as a Cypher-semantics query.
    pub(crate) fn run_op(&mut self, op: Op) -> CResult<Rows> {
        self.runner
            .run_ir(&IrQuery::cypher(op), &Params::new())
            .map_err(core)
    }

    /// The `sys:isEdge` flags visible in `view` (C6), cached per view.
    pub(crate) fn flags(&mut self, view: View) -> CResult<Flags> {
        if let Some(f) = self.flags.get(&view) {
            return Ok(f.clone());
        }
        let op = Op::Triple(
            TriplePattern::new(
                TermOrVar::var("p"),
                TermOrVar::iri(tm_core::vocab::SYS_IS_EDGE),
                TermOrVar::var("v"),
                view,
            )
            .with_eid("e"),
        )
        .order_limit(vec![Key::asc(Expr::var("e"))], None, None);
        let rows = self.run_op(op)?;
        let (pi, vi) = (rows.col("p"), rows.col("v"));
        let mut f = Flags::default();
        if let (Some(pi), Some(vi)) = (pi, vi) {
            for r in &rows.rows {
                if let (Some(Value::Iri(p)), Some(Value::Bool(b))) = (&r[pi], &r[vi]) {
                    f.edge_true.remove(p);
                    f.edge_false.remove(p);
                    if *b {
                        f.edge_true.insert(p.clone());
                    } else {
                        f.edge_false.insert(p.clone());
                    }
                }
            }
        }
        self.flags.insert(view, f.clone());
        Ok(f)
    }

    /// True when a statement `(p, o)` is presented as a property (not a relationship).
    pub(crate) fn is_property(&self, f: &Flags, p: &str, o: &Value) -> bool {
        if f.edge_true.contains(p) {
            return false;
        }
        if f.edge_false.contains(p) {
            return true;
        }
        !matches!(
            o,
            Value::Iri(_) | Value::Node(_) | Value::BNode(_) | Value::Stmt(_) | Value::Tx(_)
        )
    }

    /// The value of a property object.
    pub(crate) fn prop_val(o: &Value) -> Val {
        Val::from_literal(o)
    }

    /// Distinct values of property `key` (an IRI) of `subject`, in eid order. Volatile
    /// values count under `Now` when no stored statement exists.
    pub(crate) fn prop_values(
        &mut self,
        subject: &Value,
        key: &str,
        view: View,
    ) -> CResult<Vec<Val>> {
        let ck = (subject.lexical(), key.to_string(), view);
        if let Some(v) = self.props.get(&ck) {
            return Ok(v.clone());
        }
        let f = self.flags(view)?;
        if f.edge_true.contains(key) {
            self.props.insert(ck, Vec::new());
            return Ok(Vec::new());
        }
        let literal_only = !f.edge_false.contains(key);
        let stored = |ex: &mut Self, volatile: bool| -> CResult<Vec<Val>> {
            let mut pat =
                TriplePattern::new(tv(subject), TermOrVar::iri(key), TermOrVar::var("o"), view);
            if volatile {
                pat = pat.volatile();
            } else {
                pat = pat.with_eid("e");
            }
            let mut op = Op::Triple(pat);
            if literal_only {
                op = op.filter(Expr::Func(Func::IsLiteral, vec![Expr::var("o")]));
            }
            let op = if volatile {
                op.project_distinct(&["o"])
            } else {
                op.project(&["o", "e"])
                    .order_limit(vec![Key::asc(Expr::var("e"))], None, None)
            };
            let rows = ex.run_op(op)?;
            let oi = rows.col("o").unwrap_or(0);
            let mut out: Vec<Val> = Vec::new();
            let mut seen = std::collections::HashSet::new();
            for r in &rows.rows {
                if let Some(o) = &r[oi] {
                    let v = Self::prop_val(o);
                    if seen.insert(crate::value::group_key(&v)) {
                        out.push(v);
                    }
                }
            }
            Ok(out)
        };
        let mut out = stored(self, false)?;
        if out.is_empty() && view == View::NOW {
            // volatile values are virtual properties; a stored statement wins
            out = stored(self, true)?;
        }
        self.props.insert(ck, out.clone());
        Ok(out)
    }

    /// Reads a metadata field of statement `e` through the virtual predicates.
    pub(crate) fn virtual_prop(&mut self, e: Eid, virt: &str, view: View) -> CResult<Val> {
        let op = Op::Triple(TriplePattern::new(
            TermOrVar::Id(e.oid()),
            TermOrVar::iri(virt),
            TermOrVar::var("o"),
            view,
        ))
        .project(&["o"]);
        let rows = self.run_op(op)?;
        let Some(Some(o)) = rows.rows.first().map(|r| r[0].clone()) else {
            return Ok(Val::Null);
        };
        Ok(match (virt, &o) {
            (irv::TM_TX_ADDED | irv::TM_TX_RETRACTED, Value::Tx(t)) => Val::Int(t.0 as i64),
            (
                irv::TM_VALID_FROM | irv::TM_VALID_TO | irv::TM_ADDED_AT | irv::TM_RETRACTED_AT,
                Value::DateTime { ms, .. },
            ) => Val::DateTime { ms: *ms, tz: 0 },
            (irv::TM_RETRACT_KIND, Value::Int(k)) => match retract_kind_name(*k) {
                Some(n) => Val::Str(n.to_string()),
                None => Val::Null,
            },
            _ => Val::from_entity(&o),
        })
    }

    /// The subject, predicate IRI and object of a statement.
    pub(crate) fn stmt_parts(&mut self, e: Eid) -> CResult<Option<(Value, String, Value)>> {
        if let Some((base, kind)) = virtual_parts(e) {
            // a virtual hop reads as a relationship from the statement to its part
            let Some((s, p, o)) = self.stmt_parts(base)? else {
                return Ok(None);
            };
            let (iri, part) = match kind {
                0 => (irv::SYS_SUBJECT, s),
                1 => (irv::SYS_OBJECT, o),
                _ => (irv::SYS_PREDICATE, Value::iri(p)),
            };
            return Ok(Some((Value::Stmt(base), iri.to_string(), part)));
        }
        let id = TermOrVar::Id(e.oid());
        let t = |p: &str, v: &str| {
            Op::Triple(TriplePattern::new(
                id.clone(),
                TermOrVar::iri(p),
                TermOrVar::var(v),
                View::history(),
            ))
        };
        let op = Op::join(vec![
            t(irv::SYS_SUBJECT, "s"),
            t(irv::SYS_PREDICATE, "p"),
            t(irv::SYS_OBJECT, "o"),
        ])
        .project(&["s", "p", "o"]);
        let rows = self.run_op(op)?;
        let Some(r) = rows.rows.first() else {
            return Ok(None);
        };
        match (&r[0], &r[1], &r[2]) {
            (Some(s), Some(Value::Iri(p)), Some(o)) => Ok(Some((s.clone(), p.clone(), o.clone()))),
            _ => Ok(None),
        }
    }

    /// Rendered labels of an entity in `view` (`Statement` first for statements).
    pub(crate) fn labels_of(&mut self, subject: &Value, view: View) -> CResult<Vec<String>> {
        let op = Op::Triple(TriplePattern::new(
            tv(subject),
            TermOrVar::iri(RDF_TYPE),
            TermOrVar::var("o"),
            view,
        ))
        .project_distinct(&["o"]);
        let rows = self.run_op(op)?;
        let mut names: Vec<String> = Vec::new();
        for r in &rows.rows {
            if let Some(Value::Iri(i)) = &r[0] {
                if !is_sys(i) {
                    names.push(self.vocab.render(i));
                }
            }
        }
        names.sort();
        names.dedup();
        if matches!(subject, Value::Stmt(_)) {
            names.insert(0, "Statement".to_string());
        }
        Ok(names)
    }

    /// Properties of an entity: rendered key to value (list when multi-valued).
    pub(crate) fn node_props(
        &mut self,
        subject: &Value,
        view: View,
    ) -> CResult<BTreeMap<String, Val>> {
        let f = self.flags(view)?;
        let op = Op::Triple(
            TriplePattern::new(tv(subject), TermOrVar::var("p"), TermOrVar::var("o"), view)
                .with_eid("e"),
        )
        .project(&["p", "o", "e"])
        .order_limit(vec![Key::asc(Expr::var("e"))], None, None);
        let rows = self.run_op(op)?;
        let is_stmt = matches!(subject, Value::Stmt(_));
        let mut by_key: BTreeMap<String, Vec<Val>> = BTreeMap::new();
        for r in &rows.rows {
            let (Some(Value::Iri(p)), Some(o)) = (&r[0], &r[1]) else {
                continue;
            };
            if p == RDF_TYPE || is_sys(p) || p.starts_with(TM) && irv::is_virtual(p) {
                continue;
            }
            if !self.is_property(&f, p, o) {
                continue;
            }
            let key = self.vocab.render(p);
            if is_stmt && TEMPORAL.contains(&key.as_str()) {
                continue;
            }
            let v = Self::prop_val(o);
            let list = by_key.entry(key).or_default();
            if !list
                .iter()
                .any(|x| crate::value::group_key(x) == crate::value::group_key(&v))
            {
                list.push(v);
            }
        }
        if view == View::NOW && !is_stmt {
            for (k, v) in self.runner.volatile_of(subject).map_err(core)? {
                let key = self.vocab.render(&k);
                by_key
                    .entry(key)
                    .or_insert_with(|| vec![Val::from_literal(&v)]);
            }
        }
        Ok(by_key
            .into_iter()
            .map(|(k, mut v)| {
                let val = if v.len() == 1 {
                    v.remove(0)
                } else {
                    Val::List(v)
                };
                (k, val)
            })
            .collect())
    }

    /// Turns an evaluated value into a result value, loading labels and properties.
    pub(crate) fn materialize(&mut self, v: &Val) -> CResult<CypherValue> {
        Ok(match v {
            Val::Null => CypherValue::Null,
            Val::Bool(b) => CypherValue::Boolean(*b),
            Val::Int(i) => CypherValue::Integer(*i),
            Val::Float(x) => CypherValue::Float(*x),
            Val::Str(s) => CypherValue::String(s.clone()),
            Val::Date(d) => CypherValue::Date(*d),
            Val::DateTime { ms, tz } => CypherValue::DateTime { ms: *ms, tz: *tz },
            Val::LocalDateTime(ms) => CypherValue::LocalDateTime(*ms),
            Val::List(l) => CypherValue::List(
                l.iter()
                    .map(|x| self.materialize(x))
                    .collect::<CResult<_>>()?,
            ),
            Val::Map(m) => {
                let mut out = BTreeMap::new();
                for (k, x) in m {
                    out.insert(k.clone(), self.materialize(x)?);
                }
                CypherValue::Map(out)
            }
            Val::Node(_) => CypherValue::Node(Box::new(self.node_value(v)?)),
            Val::Rel(_) => CypherValue::Relationship(Box::new(self.rel_value(v)?)),
            Val::Path(items) => {
                let mut nodes = Vec::new();
                let mut rels = Vec::new();
                for it in items {
                    match it {
                        Val::Rel(_) => rels.push(self.rel_value(it)?),
                        other => nodes.push(self.node_value(other)?),
                    }
                }
                CypherValue::Path(Box::new(PathValue { nodes, rels }))
            }
        })
    }

    fn node_value(&mut self, v: &Val) -> CResult<NodeValue> {
        let Val::Node(term) = v else {
            return Err(CypherError::eval("not a node"));
        };
        let view = self.view();
        let labels = self.labels_of(term, view)?;
        let props = self.node_props(term, view)?;
        let mut properties = BTreeMap::new();
        for (k, x) in props {
            properties.insert(k, self.materialize(&x)?);
        }
        Ok(NodeValue {
            term: term.clone(),
            element_id: term.lexical(),
            labels,
            properties,
        })
    }

    fn rel_value(&mut self, v: &Val) -> CResult<RelValue> {
        let Val::Rel(e) = v else {
            return Err(CypherError::eval("not a relationship"));
        };
        let view = self.view();
        let subject = Value::Stmt(*e);
        let (s, p, o) = self
            .stmt_parts(*e)?
            .ok_or_else(|| CypherError::eval("relationship statement not found"))?;
        let props = self.node_props(&subject, view)?;
        let mut properties = BTreeMap::new();
        for (k, x) in props {
            properties.insert(k, self.materialize(&x)?);
        }
        Ok(RelValue {
            eid: *e,
            element_id: subject.lexical(),
            rel_type: self.vocab.render(&p),
            start_element_id: s.lexical(),
            end_element_id: o.lexical(),
            properties,
        })
    }
}

/// True for a namespace helper used by callers.
pub fn in_rdf(iri: &str) -> bool {
    iri.starts_with(RDF)
}
