//! Write clauses: `CREATE` and `MERGE` here, `SET`/`REMOVE`/`DELETE` in
//! `write_set.rs` (design Decision 11). Every operation goes through the
//! transaction handle, so schema checks and the reserved-namespace rule apply.

use tm_core::{Eid, ObjectId, Tx, Valid, Value};

use super::access::{RDF_TYPE, TEMPORAL};
use super::{core, Exec, Row};
use crate::ast::*;
use crate::error::{CResult, CypherError};
use crate::value::Val;
use crate::vocab::VocabExt;

/// State kept across the write clauses of one query.
#[derive(Default)]
pub struct WriteState {
    /// Entities deleted by this query; reading them is an error.
    pub deleted: std::collections::HashSet<String>,
    /// Nodes deleted without `DETACH`; checked for live relationships at the end.
    pub deleted_nodes: Vec<Value>,
}

/// Evaluated property map entries (key IRI, key name, value) and the `@id`.
pub(crate) type PropMap = (Vec<(String, String, Val)>, Option<Value>);

/// The `ObjectId` of a term, interning it.
pub(crate) fn oid(tx: &mut Tx<'_>, v: &Value) -> tm_core::Result<ObjectId> {
    match v {
        Value::Stmt(e) => Ok(e.oid()),
        other => tx.encode(other),
    }
}

/// A subject or object term of a value.
pub(crate) fn term_of(v: &Val) -> Option<Value> {
    match v {
        Val::Node(t) => Some(t.clone()),
        Val::Rel(e) => Some(Value::Stmt(*e)),
        other => other.to_term(),
    }
}

fn write_err(msg: impl Into<String>) -> CypherError {
    CypherError::eval(msg.into())
}

impl Exec<'_> {
    /// Runs `f` on the transaction; caches are dropped afterwards.
    pub(crate) fn tx_do<R>(
        &mut self,
        f: impl FnOnce(&mut Tx<'_>) -> tm_core::Result<R>,
    ) -> CResult<R> {
        let mut f = Some(f);
        let mut out: Option<R> = None;
        let r = self.runner.with_tx(&mut |tx| {
            let f = f.take().expect("runs once");
            out = Some(f(tx)?);
            Ok(())
        });
        self.props.clear();
        self.flags.clear();
        r.map_err(core)?;
        Ok(out.expect("ran"))
    }

    /// The stored terms a property value writes: none for `null` or `[]`, one per
    /// distinct element for a list, one for a scalar.
    pub(crate) fn write_terms(&self, v: &Val) -> CResult<Vec<Value>> {
        match v {
            Val::Null => Ok(Vec::new()),
            Val::List(l) => {
                let mut out: Vec<Value> = Vec::new();
                for x in l {
                    match x {
                        Val::Null | Val::List(_) | Val::Map(_) => {
                            return Err(write_err(
                                "a property list must not contain null, lists or maps",
                            ))
                        }
                        other => {
                            let t = other
                                .to_term()
                                .ok_or_else(|| write_err("unstorable list element"))?;
                            if !out.contains(&t) {
                                out.push(t);
                            }
                        }
                    }
                }
                Ok(out)
            }
            Val::Map(_) => Err(write_err("a map cannot be stored as a property value")),
            Val::Node(_) | Val::Rel(_) | Val::Path(_) => Err(write_err(
                "a node, relationship or path cannot be stored as a property value",
            )),
            other => Ok(vec![other
                .to_term()
                .ok_or_else(|| write_err("unstorable value"))?]),
        }
    }

    /// Evaluates a property map into `(key iri, key name, value)`, plus the `@id`.
    pub(crate) fn eval_prop_map(&mut self, e: &Option<Expr>, row: &Row) -> CResult<PropMap> {
        let mut props = Vec::new();
        let mut id = None;
        let Some(e) = e else { return Ok((props, id)) };
        let entries: Vec<(String, bool, Val, Span2)> = match &e.kind {
            ExprKind::Map(es) => {
                let mut v = Vec::new();
                for (k, x) in es {
                    v.push((k.text.clone(), k.escaped, self.eval(x, row)?, k.span));
                }
                v
            }
            _ => match self.eval(e, row)? {
                Val::Map(m) => m.into_iter().map(|(k, x)| (k, true, x, e.span)).collect(),
                _ => return Err(write_err("a property map is expected")),
            },
        };
        for (k, esc, v, span) in entries {
            if k == "@id" && esc {
                match &v {
                    Val::Str(s) => id = Some(self.vocab.resolve_id(s).map_err(write_err)?),
                    Val::Null => return Err(write_err("@id must not be null")),
                    _ => return Err(write_err("@id must be a string")),
                }
                continue;
            }
            let name = Name {
                text: k.clone(),
                escaped: esc,
                span,
            };
            let iri = self.vocab.resolve(&name)?;
            props.push((iri, k, v));
        }
        Ok((props, id))
    }

    fn label_iris(&self, groups: &[Vec<Name>]) -> CResult<Vec<String>> {
        let mut out = Vec::new();
        for g in groups {
            if g.len() != 1 {
                return Err(CypherError::unsupported(
                    "a label disjunction in a write clause",
                    Some(g[0].span),
                ));
            }
            if (g[0].text == "Statement" || g[0].text == "Predicate") && !g[0].text.contains(':') {
                return Err(CypherError::unsupported(
                    format!("the implicit label {} in a write clause", g[0].text),
                    Some(g[0].span),
                ));
            }
            out.push(self.vocab.resolve(&g[0])?);
        }
        Ok(out)
    }

    pub(crate) fn exec_write(&mut self, rows: Vec<Row>, c: &Clause) -> CResult<Vec<Row>> {
        match c {
            Clause::Create { pattern, .. } => {
                let mut out = Vec::with_capacity(rows.len());
                for mut r in rows {
                    self.create_pattern(&mut r, pattern)?;
                    out.push(r);
                }
                Ok(out)
            }
            Clause::Merge {
                part,
                on_create,
                on_match,
                ..
            } => self.exec_merge(rows, part, on_create, on_match),
            Clause::Set(items) => {
                let mut out = Vec::with_capacity(rows.len());
                for mut r in rows {
                    self.apply_set(&mut r, items)?;
                    out.push(r);
                }
                Ok(out)
            }
            Clause::Remove(items) => {
                let mut out = Vec::with_capacity(rows.len());
                for mut r in rows {
                    self.apply_remove(&mut r, items)?;
                    out.push(r);
                }
                Ok(out)
            }
            Clause::Delete { detach, exprs, .. } => {
                for r in &rows {
                    for e in exprs {
                        let v = self.eval(e, r)?;
                        self.delete_value(&v, *detach)?;
                    }
                }
                Ok(rows)
            }
            _ => Ok(rows),
        }
    }

    /// Creates the parts of `pattern` for one row, binding new variables.
    pub(crate) fn create_pattern(&mut self, row: &mut Row, pattern: &Pattern) -> CResult<()> {
        for part in pattern {
            let mut nodes: Vec<Val> = Vec::new();
            for n in &part.nodes {
                let v = self.create_node(row, n, !part.rels.is_empty())?;
                nodes.push(v);
            }
            let mut items: Vec<Val> = Vec::new();
            for (i, r) in part.rels.iter().enumerate() {
                let (a, b) = match r.dir {
                    Dir::Left => (&nodes[i + 1], &nodes[i]),
                    _ => (&nodes[i], &nodes[i + 1]),
                };
                let rel = self.create_rel(row, r, a, b)?;
                items.push(nodes[i].clone());
                items.push(rel);
            }
            if let Some(last) = nodes.last() {
                items.push(last.clone());
            }
            if let Some(b) = &part.binding {
                row.insert(b.text.clone(), Val::Path(items));
            }
        }
        Ok(())
    }

    fn create_node(&mut self, row: &mut Row, n: &NodePat, in_rel: bool) -> CResult<Val> {
        let existing = n.var.as_ref().and_then(|v| row.get(&v.text)).cloned();
        let (props, id) = self.eval_prop_map(&n.props, row)?;
        let labels = self.label_iris(&n.labels)?;
        let term: Value = match &existing {
            Some(Val::Node(t)) => t.clone(),
            Some(Val::Rel(e)) => Value::Stmt(*e),
            Some(Val::Null) => {
                if in_rel {
                    return Err(write_err("cannot create a relationship on a null node"));
                }
                return Ok(Val::Null);
            }
            Some(other) => {
                return Err(write_err(format!(
                    "variable is a {}, not a node",
                    other.type_name()
                )))
            }
            None => match id.clone() {
                Some(v) => v,
                None => {
                    let has_content = !labels.is_empty() || !props.is_empty() || in_rel;
                    let node = self.tx_do(|tx| tx.new_node().and_then(|o| tx.decode(o)))?;
                    if !has_content {
                        // C11: an empty node writes nothing and is not persisted
                        let val = Val::Node(node);
                        if let Some(v) = &n.var {
                            row.insert(v.text.clone(), val.clone());
                        }
                        return Ok(val);
                    }
                    node
                }
            },
        };
        // labels and properties, idempotently
        let mut writes: Vec<(String, Value)> = Vec::new();
        for l in &labels {
            writes.push((RDF_TYPE.to_string(), Value::iri(l.clone())));
        }
        for (iri, _, v) in &props {
            for t in self.write_terms(v)? {
                writes.push((iri.clone(), t));
            }
        }
        let t2 = term.clone();
        self.tx_do(move |tx| {
            let s = oid(tx, &t2)?;
            for (p, o) in &writes {
                let (p, o) = (tx.encode(Value::iri(p.clone()))?, oid(tx, o)?);
                tx.assert(s, p, o, Valid::ALWAYS)?;
            }
            Ok(())
        })?;
        let val = Val::from_entity(&term);
        if let Some(v) = &n.var {
            row.insert(v.text.clone(), val.clone());
        }
        Ok(val)
    }

    fn create_rel(&mut self, row: &mut Row, r: &RelPat, a: &Val, b: &Val) -> CResult<Val> {
        let (Some(s), Some(o)) = (term_of(a), term_of(b)) else {
            return Err(write_err(
                "cannot create a relationship on a null or unstorable end",
            ));
        };
        let iri = self.vocab.resolve(&r.types[0])?;
        let (props, _) = self.eval_prop_map(&r.props, row)?;
        let mut valid = Valid::ALWAYS;
        let mut plain: Vec<(String, Val)> = Vec::new();
        for (piri, key, v) in props {
            if key == "validFrom" || key == "validTo" {
                let ms = match &v {
                    Val::Date(d) => d * 86_400_000,
                    Val::DateTime { ms, .. } => *ms,
                    Val::Null => {
                        continue;
                    }
                    other => {
                        return Err(write_err(format!(
                            "{key} must be a date or a datetime, got {}",
                            other.type_name()
                        )))
                    }
                };
                if key == "validFrom" {
                    valid.from = Some(ms);
                } else {
                    valid.to = Some(ms);
                }
            } else {
                plain.push((piri, v));
            }
        }
        let mut writes: Vec<(String, Value)> = Vec::new();
        for (piri, v) in &plain {
            for t in self.write_terms(v)? {
                writes.push((piri.clone(), t));
            }
        }
        let eid: Eid = self.tx_do(move |tx| {
            let (s, p, o) = (
                oid(tx, &s)?,
                tx.encode(Value::iri(iri.clone()))?,
                oid(tx, &o)?,
            );
            let e = tx.create(s, p, o, valid)?;
            for (k, t) in &writes {
                let (k, t) = (tx.encode(Value::iri(k.clone()))?, oid(tx, t)?);
                tx.assert(e, k, t, Valid::ALWAYS)?;
            }
            Ok(e)
        })?;
        let val = Val::Rel(eid);
        if let Some(v) = &r.var {
            row.insert(v.text.clone(), val.clone());
        }
        Ok(val)
    }

    fn exec_merge(
        &mut self,
        rows: Vec<Row>,
        part: &PatternPart,
        on_create: &[SetItem],
        on_match: &[SetItem],
    ) -> CResult<Vec<Row>> {
        let mut out = Vec::new();
        for r in rows {
            // null property values are rejected up front
            for n in &part.nodes {
                self.reject_null_props(&n.props, &r)?;
            }
            for rel in &part.rels {
                self.reject_null_props(&rel.props, &r)?;
            }
            // node MERGE with a unique key: an upsert
            if part.rels.is_empty() {
                if let Some(res) = self.merge_unique(&r, &part.nodes[0], part)? {
                    let (mut row, created) = res;
                    self.apply_set(&mut row, if created { on_create } else { on_match })?;
                    out.push(row);
                    continue;
                }
            }
            let one = vec![r.clone()];
            let matched =
                self.exec_match(one, &vec![part.clone()], None, false, MatchModeExt::Default)?;
            if matched.is_empty() {
                let mut row = r;
                self.create_pattern(&mut row, &vec![part.clone()])?;
                self.apply_set(&mut row, on_create)?;
                out.push(row);
            } else {
                for mut m in matched {
                    self.apply_set(&mut m, on_match)?;
                    out.push(m);
                }
            }
        }
        Ok(out)
    }

    fn reject_null_props(&mut self, e: &Option<Expr>, row: &Row) -> CResult<()> {
        if let Some(Expr {
            kind: ExprKind::Map(es),
            ..
        }) = e
        {
            for (k, x) in es {
                if matches!(self.eval(x, row)?, Val::Null) {
                    return Err(write_err(format!(
                        "MERGE property `{}` must not be null",
                        k.text
                    )));
                }
            }
        }
        Ok(())
    }

    /// `MERGE (n {uk: v, …})` with a `sys:unique` key: look up or create the subject.
    /// Returns the row and whether the node was created.
    fn merge_unique(
        &mut self,
        row: &Row,
        n: &NodePat,
        part: &PatternPart,
    ) -> CResult<Option<(Row, bool)>> {
        if n.var.as_ref().is_some_and(|v| row.contains_key(&v.text)) {
            return Ok(None);
        }
        let (props, id) = self.eval_prop_map(&n.props, row)?;
        if id.is_some() {
            return Ok(None);
        }
        // the first key whose predicate is flagged unique, in map order
        let mut chosen: Option<(String, Val)> = None;
        for (iri, _, v) in &props {
            let iri2 = iri.clone();
            let unique = self.tx_do(move |tx| match tx.lookup(&Value::iri(iri2))? {
                Some(p) => Ok(tx.schema(p)?.unique),
                None => Ok(false),
            })?;
            if unique {
                chosen = Some((iri.clone(), v.clone()));
                break;
            }
        }
        let Some((key, kv)) = chosen else {
            return Ok(None);
        };
        let terms = self.write_terms(&kv)?;
        let Some(kterm) = terms.into_iter().next() else {
            return Err(write_err("a unique MERGE key must not be null"));
        };
        let labels = self.label_iris(&n.labels)?;
        let mut writes: Vec<(String, Value)> = Vec::new();
        for l in &labels {
            writes.push((RDF_TYPE.to_string(), Value::iri(l.clone())));
        }
        for (iri, _, v) in &props {
            for t in self.write_terms(v)? {
                writes.push((iri.clone(), t));
            }
        }
        let (key2, kterm2) = (key.clone(), kterm.clone());
        let (node, created) = self.tx_do(move |tx| {
            let p = tx.encode(Value::iri(key2.clone()))?;
            let o = oid(tx, &kterm2)?;
            let found = tx.read_with(|e| {
                e.query_i64(
                    "SELECT s FROM triple WHERE p = ?1 AND o = ?2 AND t_ret IS NULL ORDER BY eid LIMIT 1",
                    &[tm_core::SqlValue::Integer(p.raw()), tm_core::SqlValue::Integer(o.raw())],
                )
            })?;
            let created = found.is_none();
            let node = tx.upsert(p, o)?;
            // the key statement is asserted idempotently so that it is reported
            tx.assert(node, p, o, Valid::ALWAYS)?;
            for (pi, t) in &writes {
                let (pi, t) = (tx.encode(Value::iri(pi.clone()))?, oid(tx, t)?);
                tx.assert(node, pi, t, Valid::ALWAYS)?;
            }
            Ok((tx.decode(node)?, created))
        })?;
        let mut out = row.clone();
        if let Some(v) = &n.var {
            out.insert(v.text.clone(), Val::from_entity(&node));
        }
        let _ = part;
        Ok(Some((out, created)))
    }

    /// Nodes to check at the end of the query.
    pub(crate) fn note_deleted(&mut self, n: Value) {
        if !self.writer.deleted_nodes.contains(&n) {
            self.writer.deleted_nodes.push(n);
        }
    }

    /// The pattern-time rule for temporal names on nodes vs statements.
    pub(crate) fn is_temporal_name(k: &Name) -> bool {
        !k.text.contains(':') && TEMPORAL.contains(&k.text.as_str())
    }
}

type Span2 = crate::Span;
