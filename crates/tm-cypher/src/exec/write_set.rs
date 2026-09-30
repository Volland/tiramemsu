//! `SET`, `REMOVE` and `DELETE` (design Decision 11, the rules of C8 and C18).

use tm_core::{Eid, Error, ObjectId, Patch, Valid, Value};
use tm_ir::{Expr as IrExpr, Func, Key, Op, TermOrVar, TriplePattern, View};

use super::access::{is_sys, tv, RDF_TYPE};
use super::write::{oid, term_of};
use super::{core, Exec, Row};
use crate::ast::*;
use crate::error::{CResult, CypherError};
use crate::value::Val;
use crate::vocab::VocabExt;

fn eval_err(msg: impl Into<String>) -> CypherError {
    CypherError::eval(msg.into())
}

impl Exec<'_> {
    /// Live property statements `(subject, key, object)` at `Now`, valid time ignored.
    pub(crate) fn live_props(&mut self, subject: &Value, key: &str) -> CResult<Vec<(Eid, Value)>> {
        let f = self.flags(View::NOW)?;
        if f.edge_true.contains(key) {
            return Ok(Vec::new());
        }
        let mut op = Op::Triple(
            TriplePattern::new(
                tv(subject),
                TermOrVar::iri(key),
                TermOrVar::var("o"),
                View::NOW,
            )
            .with_eid("e"),
        );
        if !f.edge_false.contains(key) {
            op = op.filter(IrExpr::Func(Func::IsLiteral, vec![IrExpr::var("o")]));
        }
        let op = op
            .project(&["e", "o"])
            .order_limit(vec![Key::asc(IrExpr::var("e"))], None, None);
        let rows = self.run_op(op)?;
        let mut out = Vec::new();
        for r in &rows.rows {
            if let (Some(Value::Stmt(e)), Some(o)) = (&r[0], &r[1]) {
                out.push((*e, o.clone()));
            }
        }
        Ok(out)
    }

    /// `SET x.key = value` by the rule list of C8.
    pub(crate) fn set_property(&mut self, subject: &Value, iri: &str, v: &Val) -> CResult<()> {
        let newterms = self.write_terms(v)?;
        let is_list = matches!(v, Val::List(_));
        let existing = self.live_props(subject, iri)?;
        let (subject, iri) = (subject.clone(), iri.to_string());
        self.tx_do(move |tx| {
            let s = oid(tx, &subject)?;
            let p = tx.encode(Value::iri(iri.clone()))?;
            if newterms.is_empty() {
                for (e, _) in &existing {
                    tx.retract(*e)?;
                }
                return Ok(());
            }
            if is_list {
                for (e, o) in &existing {
                    if !newterms.contains(o) {
                        tx.retract(*e)?;
                    }
                }
                for t in &newterms {
                    let o = oid(tx, t)?;
                    tx.assert(s, p, o, Valid::ALWAYS)?;
                }
                return Ok(());
            }
            let nv = &newterms[0];
            let o = oid(tx, nv)?;
            if existing.is_empty() {
                tx.assert(s, p, o, Valid::ALWAYS)?;
                return Ok(());
            }
            if existing.iter().any(|(_, x)| x == nv) {
                tx.assert(s, p, o, Valid::ALWAYS)?;
                for (e, x) in &existing {
                    if x != nv {
                        tx.retract(*e)?;
                    }
                }
                return Ok(());
            }
            if tx.schema(p)?.one {
                tx.assert(s, p, o, Valid::ALWAYS)?;
                return Ok(());
            }
            if existing.len() == 1 {
                tx.supersede(existing[0].0, Patch::object(nv.clone()))?;
                return Ok(());
            }
            for (e, _) in &existing {
                tx.retract(*e)?;
            }
            tx.assert(s, p, o, Valid::ALWAYS)?;
            Ok(())
        })
    }

    fn set_target(&mut self, row: &Row, e: &Expr) -> CResult<Option<(Val, Name)>> {
        let ExprKind::Prop(base, key) = &e.kind else {
            return Err(CypherError::unsupported(
                "this assignment target",
                Some(e.span),
            ));
        };
        let b = self.eval(base, row)?;
        if b.is_null() {
            return Ok(None);
        }
        Ok(Some((b, key.clone())))
    }

    fn valid_bound(v: &Val, what: &str) -> CResult<Option<i64>> {
        match v {
            Val::Null => Ok(None),
            Val::Date(d) => Ok(Some(d * 86_400_000)),
            Val::DateTime { ms, .. } => Ok(Some(*ms)),
            other => Err(eval_err(format!(
                "{what} must be a date or a datetime, got {}",
                other.type_name()
            ))),
        }
    }

    /// Supersedes statement `e` with a patched valid-time bound; returns the new eid.
    fn patch_valid(&mut self, e: Eid, key: &str, v: &Val) -> CResult<Eid> {
        let bound = Self::valid_bound(v, key)?;
        let patch = if key == "validFrom" {
            Patch {
                v_from: Some(bound),
                ..Patch::default()
            }
        } else {
            Patch {
                v_to: Some(bound),
                ..Patch::default()
            }
        };
        self.tx_do(move |tx| tx.supersede(e, patch))
    }

    pub(crate) fn apply_set(&mut self, row: &mut Row, items: &[SetItem]) -> CResult<()> {
        for it in items {
            match it {
                SetItem::Prop { target, value, .. } => {
                    let Some((b, key)) = self.set_target(row, target)? else {
                        continue;
                    };
                    let val = self.eval(value, row)?;
                    let Some(term) = term_of(&b) else {
                        return Err(eval_err(format!(
                            "cannot set a property on a {}",
                            b.type_name()
                        )));
                    };
                    if let (Value::Stmt(e), true) = (&term, Self::is_temporal_name(&key)) {
                        match key.text.as_str() {
                            "txAdded" | "txRetracted" => {
                                return Err(CypherError::unsupported(
                                    format!("SET of {} (transaction time is read-only)", key.text),
                                    Some(key.span),
                                ))
                            }
                            k => {
                                let new = self.patch_valid(*e, k, &val)?;
                                self.rebind_stmt(row, target, new);
                                continue;
                            }
                        }
                    }
                    let iri = self.vocab.resolve(&key)?;
                    self.set_property(&term, &iri, &val)?;
                }
                SetItem::Replace { target, value, .. } | SetItem::Merge { target, value, .. } => {
                    let replace = matches!(it, SetItem::Replace { .. });
                    let b = row.get(&target.text).cloned().unwrap_or(Val::Null);
                    if b.is_null() {
                        continue;
                    }
                    let val = self.eval(value, row)?;
                    let map = match val {
                        Val::Map(m) => m,
                        Val::Node(_) | Val::Rel(_) => {
                            return Err(CypherError::unsupported(
                                "SET x = <node or relationship>",
                                Some(target.span),
                            ))
                        }
                        Val::Null if replace => Default::default(),
                        Val::Null => continue,
                        other => {
                            return Err(eval_err(format!(
                                "SET needs a map, got {}",
                                other.type_name()
                            )))
                        }
                    };
                    let Some(term) = term_of(&b) else {
                        return Err(eval_err(format!(
                            "cannot set properties on a {}",
                            b.type_name()
                        )));
                    };
                    let mut keep: Vec<String> = Vec::new();
                    for (k, v) in &map {
                        let name = Name {
                            text: k.clone(),
                            escaped: true,
                            span: target.span,
                        };
                        let iri = self.vocab.resolve(&name)?;
                        keep.push(iri.clone());
                        self.set_property(&term, &iri, v)?;
                    }
                    if replace {
                        self.retract_other_props(&term, &keep)?;
                    }
                }
                SetItem::Labels { target, labels, .. } => {
                    let b = row.get(&target.text).cloned().unwrap_or(Val::Null);
                    if b.is_null() {
                        continue;
                    }
                    let Some(term) = term_of(&b) else {
                        return Err(eval_err("cannot set a label on this value"));
                    };
                    let mut iris = Vec::new();
                    for l in labels {
                        iris.push(self.vocab.resolve(l)?);
                    }
                    self.tx_do(move |tx| {
                        let s = oid(tx, &term)?;
                        let p = tx.encode(Value::iri(RDF_TYPE))?;
                        for l in &iris {
                            let o = tx.encode(Value::iri(l.clone()))?;
                            tx.assert(s, p, o, Valid::ALWAYS)?;
                        }
                        Ok(())
                    })?;
                }
            }
        }
        Ok(())
    }

    fn rebind_stmt(&mut self, row: &mut Row, target: &Expr, new: Eid) {
        if let ExprKind::Prop(base, _) = &target.kind {
            if let ExprKind::Var(name) = &base.kind {
                let v = match row.get(name) {
                    Some(Val::Node(_)) => Val::Node(Value::Stmt(new)),
                    _ => Val::Rel(new),
                };
                row.insert(name.clone(), v);
            }
        }
    }

    /// Retracts every live non-`sys:` property statement whose key is not in `keep`.
    fn retract_other_props(&mut self, subject: &Value, keep: &[String]) -> CResult<()> {
        let f = self.flags(View::NOW)?;
        let op = Op::Triple(
            TriplePattern::new(
                tv(subject),
                TermOrVar::var("p"),
                TermOrVar::var("o"),
                View::NOW,
            )
            .with_eid("e"),
        )
        .project(&["e", "p", "o"]);
        let rows = self.run_op(op)?;
        let mut victims: Vec<Eid> = Vec::new();
        for r in &rows.rows {
            let (Some(Value::Stmt(e)), Some(Value::Iri(p)), Some(o)) = (&r[0], &r[1], &r[2]) else {
                continue;
            };
            if p == RDF_TYPE || is_sys(p) || keep.contains(p) || !self.is_property(&f, p, o) {
                continue;
            }
            victims.push(*e);
        }
        self.tx_do(move |tx| {
            for e in victims {
                tx.retract(e)?;
            }
            Ok(())
        })
    }

    pub(crate) fn apply_remove(&mut self, row: &mut Row, items: &[RemoveItem]) -> CResult<()> {
        for it in items {
            match it {
                RemoveItem::Prop(e) => {
                    let Some((b, key)) = self.set_target(row, e)? else {
                        continue;
                    };
                    let Some(term) = term_of(&b) else {
                        return Err(eval_err("cannot remove a property from this value"));
                    };
                    if let (Value::Stmt(id), true) = (&term, Self::is_temporal_name(&key)) {
                        match key.text.as_str() {
                            "txAdded" | "txRetracted" => {
                                return Err(CypherError::unsupported(
                                    format!(
                                        "REMOVE of {} (transaction time is read-only)",
                                        key.text
                                    ),
                                    Some(key.span),
                                ))
                            }
                            k => {
                                let new = self.patch_valid(*id, k, &Val::Null)?;
                                self.rebind_stmt(row, e, new);
                                continue;
                            }
                        }
                    }
                    let iri = self.vocab.resolve(&key)?;
                    self.set_property(&term, &iri, &Val::Null)?;
                }
                RemoveItem::Labels { target, labels } => {
                    let b = row.get(&target.text).cloned().unwrap_or(Val::Null);
                    if b.is_null() {
                        continue;
                    }
                    let Some(term) = term_of(&b) else { continue };
                    let mut iris = Vec::new();
                    for l in labels {
                        iris.push(self.vocab.resolve(l)?);
                    }
                    self.tx_do(move |tx| {
                        let (Some(s), Some(p)) = (
                            match &term {
                                Value::Stmt(e) => Some(e.oid()),
                                other => tx.lookup(other)?,
                            },
                            tx.lookup(&Value::iri(RDF_TYPE))?,
                        ) else {
                            return Ok(());
                        };
                        for l in &iris {
                            if let Some(o) = tx.lookup(&Value::iri(l.clone()))? {
                                tx.retract_matching(Some(s), Some(p), Some(o))?;
                            }
                        }
                        Ok(())
                    })?;
                }
            }
        }
        Ok(())
    }

    /// `DELETE` / `DETACH DELETE` of one value.
    pub(crate) fn delete_value(&mut self, v: &Val, detach: bool) -> CResult<()> {
        match v {
            Val::Null => Ok(()),
            Val::Rel(e) | Val::Node(Value::Stmt(e)) => {
                let e = *e;
                self.writer.deleted.insert(Value::Stmt(e).lexical());
                self.tx_do(move |tx| {
                    tx.retract(e)?;
                    if detach {
                        tx.retract_matching(None, None, Some(e.oid()))?;
                    }
                    Ok(())
                })
            }
            Val::Node(t) => {
                let t = t.clone();
                self.writer.deleted.insert(t.lexical());
                if detach {
                    self.tx_do(move |tx| {
                        if let Some(id) = tx.lookup(&t)? {
                            tx.retract_matching(Some(id), None, None)?;
                            tx.retract_matching(None, None, Some(id))?;
                        }
                        Ok(())
                    })
                } else {
                    let f = self.flags(View::NOW)?;
                    let op = Op::Triple(
                        TriplePattern::new(
                            tv(&t),
                            TermOrVar::var("p"),
                            TermOrVar::var("o"),
                            View::NOW,
                        )
                        .with_eid("e"),
                    )
                    .project(&["e", "p", "o"]);
                    let rows = self.run_op(op)?;
                    let mut victims: Vec<Eid> = Vec::new();
                    for r in &rows.rows {
                        let (Some(Value::Stmt(e)), Some(Value::Iri(p)), Some(o)) =
                            (&r[0], &r[1], &r[2])
                        else {
                            continue;
                        };
                        if !is_sys(p) && (p == RDF_TYPE || self.is_property(&f, p, o)) {
                            victims.push(*e);
                        }
                    }
                    self.tx_do(move |tx| {
                        for e in victims {
                            tx.retract(e)?;
                        }
                        Ok(())
                    })?;
                    self.note_deleted(t);
                    Ok(())
                }
            }
            Val::Path(items) => {
                for it in items {
                    self.delete_value(it, detach)?;
                }
                Ok(())
            }
            other => Err(eval_err(format!("cannot delete a {}", other.type_name()))),
        }
    }

    /// End of query: a node deleted without `DETACH` must have no live relationship.
    pub(crate) fn finish_writes(&mut self) -> CResult<()> {
        let nodes = std::mem::take(&mut self.writer.deleted_nodes);
        for n in nodes {
            let f = self.flags(View::NOW)?;
            let out = Op::Triple(
                TriplePattern::new(tv(&n), TermOrVar::var("p"), TermOrVar::var("o"), View::NOW)
                    .with_eid("e"),
            )
            .project(&["e", "p", "o"]);
            let inc = Op::Triple(
                TriplePattern::new(TermOrVar::var("s"), TermOrVar::var("p"), tv(&n), View::NOW)
                    .with_eid("e"),
            )
            .project(&["e", "p"]);
            let mut rels: Vec<Eid> = Vec::new();
            for r in &self.run_op(out)?.rows {
                let (Some(Value::Stmt(e)), Some(Value::Iri(p)), Some(o)) = (&r[0], &r[1], &r[2])
                else {
                    continue;
                };
                if p != RDF_TYPE && !is_sys(p) && !self.is_property(&f, p, o) {
                    rels.push(*e);
                }
            }
            for r in &self.run_op(inc)?.rows {
                let (Some(Value::Stmt(e)), Some(Value::Iri(p))) = (&r[0], &r[1]) else {
                    continue;
                };
                if p != RDF_TYPE && !is_sys(p) && !f.edge_false.contains(p) && !rels.contains(e) {
                    rels.push(*e);
                }
            }
            if !rels.is_empty() {
                rels.sort();
                let node = self
                    .runner
                    .object_id(&n)
                    .map_err(core)?
                    .map_or(ObjectId::from_raw(0), ObjectId::from_raw);
                return Err(CypherError::Core(Error::DeleteConnectedNode {
                    node,
                    relationships: rels,
                }));
            }
        }
        Ok(())
    }
}
