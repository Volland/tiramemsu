//! Expression evaluation over rows (Cypher three-valued logic, design Decision 5).

use std::cmp::Ordering;
use std::collections::BTreeMap;

use tm_core::Value;
use tm_ir::vocab as irv;

use super::access::{stmt_of, tv, TEMPORAL};
use super::{Exec, Row};
use crate::ast::*;
use crate::error::{CResult, CypherError};
use crate::value::{cmp3, eq3, order, Val};

fn type_err(op: &str, a: &Val, b: Option<&Val>) -> CypherError {
    match b {
        Some(b) => CypherError::eval(format!(
            "type mismatch: {op} is not defined for {} and {}",
            a.type_name(),
            b.type_name()
        )),
        None => CypherError::eval(format!(
            "type mismatch: {op} is not defined for {}",
            a.type_name()
        )),
    }
}

fn tri(v: &Val, op: &str) -> CResult<Option<bool>> {
    match v {
        Val::Null => Ok(None),
        Val::Bool(b) => Ok(Some(*b)),
        other => Err(type_err(op, other, None)),
    }
}

fn from_tri(t: Option<bool>) -> Val {
    t.map_or(Val::Null, Val::Bool)
}

fn ov() -> CypherError {
    CypherError::eval("integer overflow")
}

/// Arithmetic on two values (`+ - * / % ^`).
pub fn arith(op: BinOp, a: &Val, b: &Val) -> CResult<Val> {
    use Val::*;
    if a.is_null() || b.is_null() {
        // type errors win over null only for definitely-wrong operands; keep simple
        return Ok(Null);
    }
    Ok(match (op, a, b) {
        (BinOp::Add, Str(x), Str(y)) => Str(format!("{x}{y}")),
        (BinOp::Add, Str(x), y @ (Int(_) | Float(_) | Bool(_))) => {
            Str(format!("{x}{}", y.to_display()))
        }
        (BinOp::Add, x @ (Int(_) | Float(_) | Bool(_)), Str(y)) => {
            Str(format!("{}{y}", x.to_display()))
        }
        (BinOp::Add, List(x), List(y)) => List(x.iter().chain(y).cloned().collect()),
        (BinOp::Add, List(x), y) => {
            let mut v = x.clone();
            v.push(y.clone());
            List(v)
        }
        (BinOp::Add, x, List(y)) => {
            let mut v = vec![x.clone()];
            v.extend(y.iter().cloned());
            List(v)
        }
        (_, Int(x), Int(y)) => match op {
            BinOp::Add => Int(x.checked_add(*y).ok_or_else(ov)?),
            BinOp::Sub => Int(x.checked_sub(*y).ok_or_else(ov)?),
            BinOp::Mul => Int(x.checked_mul(*y).ok_or_else(ov)?),
            BinOp::Div => {
                if *y == 0 {
                    return Err(CypherError::eval("division by zero"));
                }
                Int(x.checked_div(*y).ok_or_else(ov)?)
            }
            BinOp::Mod => {
                if *y == 0 {
                    return Err(CypherError::eval("modulo by zero"));
                }
                Int(x.checked_rem(*y).ok_or_else(ov)?)
            }
            BinOp::Pow => Float((*x as f64).powf(*y as f64)),
            _ => return Err(type_err("arithmetic", a, Some(b))),
        },
        (_, Int(_) | Float(_), Int(_) | Float(_)) => {
            let x = num(a);
            let y = num(b);
            match op {
                BinOp::Add => Float(x + y),
                BinOp::Sub => Float(x - y),
                BinOp::Mul => Float(x * y),
                BinOp::Div => Float(x / y),
                BinOp::Mod => Float(x % y),
                BinOp::Pow => Float(x.powf(y)),
                _ => return Err(type_err("arithmetic", a, Some(b))),
            }
        }
        _ => return Err(type_err("arithmetic", a, Some(b))),
    })
}

fn num(v: &Val) -> f64 {
    match v {
        Val::Int(i) => *i as f64,
        Val::Float(x) => *x,
        _ => f64::NAN,
    }
}

impl Exec<'_> {
    /// Evaluates `e` on `row`.
    pub(crate) fn eval(&mut self, e: &Expr, row: &Row) -> CResult<Val> {
        self.eval_g(e, row, None)
    }

    /// Evaluates with an optional group of rows for aggregate calls.
    pub(crate) fn eval_g(&mut self, e: &Expr, row: &Row, group: Option<&[Row]>) -> CResult<Val> {
        Ok(match &e.kind {
            ExprKind::Lit(l) => match l {
                Lit::Null => Val::Null,
                Lit::Bool(b) => Val::Bool(*b),
                Lit::Int(i) => Val::Int(*i),
                Lit::Float(x) => Val::Float(*x),
                Lit::Str(s) => Val::Str(s.clone()),
            },
            ExprKind::Var(v) => row.get(v).cloned().ok_or_else(|| {
                CypherError::parse(e.span, format!("variable `{v}` is not defined"))
            })?,
            ExprKind::Param(p) => self
                .params
                .get(p)
                .cloned()
                .ok_or_else(|| CypherError::eval(format!("missing parameter ${p}")))?,
            ExprKind::CountStar => match group {
                Some(g) => Val::Int(g.len() as i64),
                None => return Err(CypherError::eval("aggregate outside a grouping projection")),
            },
            ExprKind::List(xs) => {
                let mut v = Vec::with_capacity(xs.len());
                for x in xs {
                    v.push(self.eval_g(x, row, group)?);
                }
                Val::List(v)
            }
            ExprKind::Map(es) => {
                let mut m = BTreeMap::new();
                for (k, x) in es {
                    m.insert(k.text.clone(), self.eval_g(x, row, group)?);
                }
                Val::Map(m)
            }
            ExprKind::MapProj(base, items) => self.map_projection(base, items, row, group)?,
            ExprKind::Unary(op, x) => {
                let v = self.eval_g(x, row, group)?;
                match op {
                    UnOp::Not => from_tri(tri(&v, "NOT")?.map(|b| !b)),
                    UnOp::Plus => match v {
                        Val::Null | Val::Int(_) | Val::Float(_) => v,
                        other => return Err(type_err("unary +", &other, None)),
                    },
                    UnOp::Neg => match v {
                        Val::Null => Val::Null,
                        Val::Int(i) => Val::Int(i.checked_neg().ok_or_else(ov)?),
                        Val::Float(x) => Val::Float(-x),
                        other => return Err(type_err("unary -", &other, None)),
                    },
                }
            }
            ExprKind::Binary(op, a, b) => self.binary(*op, a, b, row, group)?,
            ExprKind::IsNull(x, neg) => {
                let v = self.eval_g(x, row, group)?;
                Val::Bool(v.is_null() != *neg)
            }
            ExprKind::Prop(base, key) => {
                let b = self.eval_g(base, row, group)?;
                self.prop_access(&b, key)?
            }
            ExprKind::Index(base, idx) => {
                let b = self.eval_g(base, row, group)?;
                let i = self.eval_g(idx, row, group)?;
                self.index(&b, &i)?
            }
            ExprKind::Slice(base, lo, hi) => {
                let b = self.eval_g(base, row, group)?;
                let l = match lo {
                    Some(x) => Some(self.eval_g(x, row, group)?),
                    None => None,
                };
                let h = match hi {
                    Some(x) => Some(self.eval_g(x, row, group)?),
                    None => None,
                };
                slice(&b, l, h)?
            }
            ExprKind::Call {
                name,
                distinct,
                args,
            } => {
                if crate::funcs::is_aggregate(name) {
                    let Some(g) = group else {
                        return Err(CypherError::parse(
                            e.span,
                            "aggregate function used outside a projection",
                        ));
                    };
                    return self.aggregate(name, *distinct, args, g);
                }
                let mut vals = Vec::with_capacity(args.len());
                for a in args {
                    vals.push(self.eval_g(a, row, group)?);
                }
                self.call(name, vals, e)?
            }
            ExprKind::Case { operand, alts, els } => {
                let op = match operand {
                    Some(o) => Some(self.eval_g(o, row, group)?),
                    None => None,
                };
                for (w, t) in alts {
                    let wv = self.eval_g(w, row, group)?;
                    let hit = match &op {
                        Some(o) => eq3(o, &wv) == Some(true),
                        None => matches!(wv, Val::Bool(true)),
                    };
                    if hit {
                        return self.eval_g(t, row, group);
                    }
                }
                match els {
                    Some(x) => self.eval_g(x, row, group)?,
                    None => Val::Null,
                }
            }
            ExprKind::ListComp {
                var,
                list,
                pred,
                proj,
            } => {
                let l = self.eval_g(list, row, group)?;
                let items = match l {
                    Val::Null => return Ok(Val::Null),
                    Val::List(l) => l,
                    other => return Err(type_err("list comprehension", &other, None)),
                };
                let mut out = Vec::new();
                for it in items {
                    let mut r = row.clone();
                    r.insert(var.text.clone(), it.clone());
                    if let Some(p) = pred {
                        if !matches!(self.eval_g(p, &r, group)?, Val::Bool(true)) {
                            continue;
                        }
                    }
                    out.push(match proj {
                        Some(pr) => self.eval_g(pr, &r, group)?,
                        None => it,
                    });
                }
                Val::List(out)
            }
            ExprKind::Quantifier {
                kind,
                var,
                list,
                pred,
            } => {
                let l = self.eval_g(list, row, group)?;
                let items = match l {
                    Val::Null => return Ok(Val::Null),
                    Val::List(l) => l,
                    other => return Err(type_err("quantifier", &other, None)),
                };
                let (mut t, mut f, mut u) = (0usize, 0usize, 0usize);
                for it in items {
                    let mut r = row.clone();
                    r.insert(var.text.clone(), it);
                    match tri(&self.eval_g(pred, &r, group)?, "quantifier predicate")? {
                        Some(true) => t += 1,
                        Some(false) => f += 1,
                        None => u += 1,
                    }
                }
                let res = match kind.as_str() {
                    "all" => {
                        if f > 0 {
                            Some(false)
                        } else if u > 0 {
                            None
                        } else {
                            Some(true)
                        }
                    }
                    "any" => {
                        if t > 0 {
                            Some(true)
                        } else if u > 0 {
                            None
                        } else {
                            Some(false)
                        }
                    }
                    "none" => {
                        if t > 0 {
                            Some(false)
                        } else if u > 0 {
                            None
                        } else {
                            Some(true)
                        }
                    }
                    _ => {
                        if t > 1 {
                            Some(false)
                        } else if t == 1 {
                            if u > 0 {
                                None
                            } else {
                                Some(true)
                            }
                        } else if u > 0 {
                            None
                        } else {
                            Some(false)
                        }
                    }
                };
                from_tri(res)
            }
            ExprKind::Reduce {
                acc,
                init,
                var,
                list,
                body,
            } => {
                let mut a = self.eval_g(init, row, group)?;
                let l = self.eval_g(list, row, group)?;
                let items = match l {
                    Val::Null => return Ok(Val::Null),
                    Val::List(l) => l,
                    other => return Err(type_err("reduce", &other, None)),
                };
                for it in items {
                    let mut r = row.clone();
                    r.insert(acc.text.clone(), a);
                    r.insert(var.text.clone(), it);
                    a = self.eval_g(body, &r, group)?;
                }
                a
            }
            ExprKind::Exists(b) => Val::Bool(self.exists_body(b, row)?),
            ExprKind::HasLabels(x, names) => {
                let v = self.eval_g(x, row, group)?;
                match &v {
                    Val::Null => Val::Null,
                    Val::Node(_) => {
                        let view = self.view();
                        let Val::Node(t) = &v else { unreachable!() };
                        let have = self.labels_of(t, view)?;
                        let mut ok = true;
                        for n in names {
                            let want = if n.text == "Statement" && !n.text.contains(':') {
                                "Statement".to_string()
                            } else if n.text == "Predicate" && !n.text.contains(':') {
                                let f = self.flags(view)?;
                                let _ = f;
                                // a predicate is the subject of a schema flag
                                let ok_p = self.has_schema_flag(t, view)?;
                                if !ok_p {
                                    ok = false;
                                }
                                continue;
                            } else {
                                let iri = self.vocab.resolve(n)?;
                                self.vocab.render(&iri)
                            };
                            if !have.contains(&want) {
                                ok = false;
                            }
                        }
                        Val::Bool(ok)
                    }
                    Val::Rel(_) => {
                        let mut ok = true;
                        let Some(e) = stmt_of(&v) else { unreachable!() };
                        let view = self.view();
                        let have = self.labels_of(&Value::Stmt(e), view)?;
                        for n in names {
                            let want = if n.text == "Statement" && !n.text.contains(':') {
                                "Statement".to_string()
                            } else {
                                let iri = self.vocab.resolve(n)?;
                                self.vocab.render(&iri)
                            };
                            if !have.contains(&want) {
                                ok = false;
                            }
                        }
                        Val::Bool(ok)
                    }
                    other => return Err(type_err("label predicate", other, None)),
                }
            }
        })
    }

    fn has_schema_flag(&mut self, t: &Value, view: tm_ir::View) -> CResult<bool> {
        use tm_ir::{Op, TermOrVar, TriplePattern};
        for f in tm_core::vocab::SCHEMA_FLAGS {
            let op = Op::Triple(TriplePattern::new(
                tv(t),
                TermOrVar::iri(f),
                TermOrVar::var("x"),
                view,
            ))
            .project(&["x"]);
            if !self.run_op(op)?.rows.is_empty() {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn binary(
        &mut self,
        op: BinOp,
        a: &Expr,
        b: &Expr,
        row: &Row,
        group: Option<&[Row]>,
    ) -> CResult<Val> {
        match op {
            BinOp::And | BinOp::Or | BinOp::Xor => {
                let x = tri(&self.eval_g(a, row, group)?, "boolean operator")?;
                // short-circuit only when the result is decided
                if op == BinOp::And && x == Some(false) {
                    let y = self.eval_g(b, row, group)?;
                    tri(&y, "boolean operator")?;
                    return Ok(Val::Bool(false));
                }
                if op == BinOp::Or && x == Some(true) {
                    let y = self.eval_g(b, row, group)?;
                    tri(&y, "boolean operator")?;
                    return Ok(Val::Bool(true));
                }
                let y = tri(&self.eval_g(b, row, group)?, "boolean operator")?;
                Ok(from_tri(match op {
                    BinOp::And => match (x, y) {
                        (Some(false), _) | (_, Some(false)) => Some(false),
                        (Some(true), Some(true)) => Some(true),
                        _ => None,
                    },
                    BinOp::Or => match (x, y) {
                        (Some(true), _) | (_, Some(true)) => Some(true),
                        (Some(false), Some(false)) => Some(false),
                        _ => None,
                    },
                    _ => match (x, y) {
                        (Some(p), Some(q)) => Some(p != q),
                        _ => None,
                    },
                }))
            }
            _ => {
                let x = self.eval_g(a, row, group)?;
                let y = self.eval_g(b, row, group)?;
                self.binary_vals(op, x, y)
            }
        }
    }

    pub(crate) fn binary_vals(&mut self, op: BinOp, x: Val, y: Val) -> CResult<Val> {
        Ok(match op {
            BinOp::Eq => from_tri(eq3(&x, &y)),
            BinOp::Ne => from_tri(eq3(&x, &y).map(|b| !b)),
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge if matches!((&x, &y), (Val::Float(a), Val::Int(_) | Val::Float(_)) | (Val::Int(_), Val::Float(a)) if a.is_nan()) => {
                Val::Bool(false)
            }
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                from_tri(cmp3(&x, &y).map(|o| match op {
                    BinOp::Lt => o == Ordering::Less,
                    BinOp::Le => o != Ordering::Greater,
                    BinOp::Gt => o == Ordering::Greater,
                    _ => o != Ordering::Less,
                }))
            }
            BinOp::In => match (&x, &y) {
                (_, Val::Null) => Val::Null,
                (_, Val::List(l)) => {
                    let mut unknown = false;
                    for it in l {
                        match eq3(&x, it) {
                            Some(true) => return Ok(Val::Bool(true)),
                            None => unknown = true,
                            _ => {}
                        }
                    }
                    if unknown {
                        Val::Null
                    } else {
                        Val::Bool(false)
                    }
                }
                _ => return Err(type_err("IN", &y, None)),
            },
            BinOp::StartsWith | BinOp::EndsWith | BinOp::Contains => match (&x, &y) {
                (Val::Str(a), Val::Str(b)) => Val::Bool(match op {
                    BinOp::StartsWith => a.starts_with(b.as_str()),
                    BinOp::EndsWith => a.ends_with(b.as_str()),
                    _ => a.contains(b.as_str()),
                }),
                (Val::Null, _) | (_, Val::Null) => Val::Null,
                _ => Val::Null,
            },
            BinOp::Regex => match (&x, &y) {
                (Val::Str(s), Val::Str(p)) => {
                    let re = regex::Regex::new(&format!("^(?:{p})$")).map_err(|e| {
                        CypherError::eval(format!("invalid regular expression: {e}"))
                    })?;
                    Val::Bool(re.is_match(s))
                }
                (Val::Null, _) | (_, Val::Null) => Val::Null,
                _ => return Err(type_err("=~", &x, Some(&y))),
            },
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod | BinOp::Pow => {
                arith(op, &x, &y)?
            }
            BinOp::And | BinOp::Or | BinOp::Xor => unreachable!(),
        })
    }

    fn index(&mut self, base: &Val, idx: &Val) -> CResult<Val> {
        match (base, idx) {
            (Val::Null, _) | (_, Val::Null) => Ok(Val::Null),
            (Val::List(l), Val::Int(i)) => {
                let n = l.len() as i64;
                let k = if *i < 0 { n + i } else { *i };
                Ok(if k < 0 || k >= n {
                    Val::Null
                } else {
                    l[k as usize].clone()
                })
            }
            (Val::Map(m), Val::Str(k)) => Ok(m.get(k).cloned().unwrap_or(Val::Null)),
            (Val::Node(_) | Val::Rel(_), Val::Str(k)) => {
                let name = Name {
                    text: k.clone(),
                    escaped: true,
                    span: crate::Span::new(0, 0),
                };
                self.prop_access(base, &name)
            }
            _ => Err(type_err("indexing", base, Some(idx))),
        }
    }

    fn map_projection(
        &mut self,
        base: &Expr,
        items: &[MapProjItem],
        row: &Row,
        group: Option<&[Row]>,
    ) -> CResult<Val> {
        let b = self.eval_g(base, row, group)?;
        if b.is_null() {
            return Ok(Val::Null);
        }
        let mut out = BTreeMap::new();
        for it in items {
            match it {
                MapProjItem::Prop(k) => {
                    out.insert(k.text.clone(), self.prop_access(&b, k)?);
                }
                MapProjItem::All => match &b {
                    Val::Map(m) => out.extend(m.clone()),
                    Val::Node(_) | Val::Rel(_) => {
                        let props = self.all_props(&b)?;
                        out.extend(props);
                    }
                    other => return Err(type_err("map projection", other, None)),
                },
                MapProjItem::Var(n) => {
                    out.insert(
                        n.text.clone(),
                        row.get(&n.text).cloned().unwrap_or(Val::Null),
                    );
                }
                MapProjItem::Entry(k, x) => {
                    out.insert(k.text.clone(), self.eval_g(x, row, group)?);
                }
            }
        }
        Ok(Val::Map(out))
    }

    /// `properties(x)` of a node or relationship.
    pub(crate) fn all_props(&mut self, b: &Val) -> CResult<BTreeMap<String, Val>> {
        let view = self.view();
        let term = match b {
            Val::Node(t) => t.clone(),
            Val::Rel(e) => Value::Stmt(*e),
            _ => return Ok(BTreeMap::new()),
        };
        self.node_props(&term, view)
    }

    /// `base.key`.
    pub(crate) fn prop_access(&mut self, base: &Val, key: &Name) -> CResult<Val> {
        match base {
            Val::Null => Ok(Val::Null),
            Val::Map(m) => Ok(m.get(&key.text).cloned().unwrap_or(Val::Null)),
            Val::Node(_) | Val::Rel(_) => {
                self.check_not_deleted(base)?;
                let view = self.view();
                if let Some(e) = stmt_of(base) {
                    if !key.text.contains(':') && TEMPORAL.contains(&key.text.as_str()) {
                        let virt = match key.text.as_str() {
                            "txAdded" => irv::TM_TX_ADDED,
                            "txRetracted" => irv::TM_TX_RETRACTED,
                            "validFrom" => irv::TM_VALID_FROM,
                            _ => irv::TM_VALID_TO,
                        };
                        return self.virtual_prop(e, virt, view);
                    }
                    let iri = self.vocab.resolve(key)?;
                    if irv::is_virtual(&iri) {
                        return self.virtual_prop(e, &iri, view);
                    }
                }
                let iri = self.vocab.resolve(key)?;
                let term = match base {
                    Val::Node(t) => t.clone(),
                    Val::Rel(e) => Value::Stmt(*e),
                    _ => unreachable!(),
                };
                let mut vals = self.prop_values(&term, &iri, view)?;
                Ok(match vals.len() {
                    0 => Val::Null,
                    1 => vals.remove(0),
                    _ => Val::List(vals),
                })
            }
            Val::Date(_) | Val::DateTime { .. } | Val::LocalDateTime(_) => {
                temporal_component(base, &key.text)
            }
            other => Err(type_err("property access", other, None)),
        }
    }

    /// Reading a node or relationship deleted earlier in the query is an error.
    pub(crate) fn check_not_deleted(&self, v: &Val) -> CResult<()> {
        if self.writer.deleted.is_empty() {
            return Ok(());
        }
        let key = match v {
            Val::Node(t) => t.lexical(),
            Val::Rel(e) => Value::Stmt(*e).lexical(),
            _ => return Ok(()),
        };
        if self.writer.deleted.contains(&key) {
            return Err(CypherError::eval(
                "EntityNotFound: DeletedEntityAccess: the entity was deleted in this query",
            ));
        }
        Ok(())
    }

    /// True when the existential body has at least one match for `row`.
    pub(crate) fn exists_body(&mut self, b: &ExistsBody, row: &Row) -> CResult<bool> {
        match b {
            ExistsBody::Pattern(p, w) => {
                let rows = self.exec_match(
                    vec![row.clone()],
                    p,
                    w.as_ref(),
                    false,
                    MatchModeExt::Default,
                )?;
                Ok(!rows.is_empty())
            }
            ExistsBody::Query(q) => {
                let (_, rows) = self.run_query(q, vec![row.clone()], false)?;
                Ok(!rows.is_empty())
            }
        }
    }

    /// `order()`-based sort of rows by keys evaluated per row.
    pub(crate) fn sort_values(vals: &mut [(Vec<Val>, usize)], desc: &[bool]) {
        vals.sort_by(|a, b| {
            for (i, d) in desc.iter().enumerate() {
                let o = order(&a.0[i], &b.0[i]);
                let o = if *d { o.reverse() } else { o };
                if o != Ordering::Equal {
                    return o;
                }
            }
            a.1.cmp(&b.1)
        });
    }
}

fn slice(b: &Val, lo: Option<Val>, hi: Option<Val>) -> CResult<Val> {
    let Val::List(l) = b else {
        return if b.is_null() {
            Ok(Val::Null)
        } else {
            Err(type_err("slicing", b, None))
        };
    };
    let n = l.len() as i64;
    let bound = |v: Option<Val>, dflt: i64| -> CResult<Option<i64>> {
        match v {
            None => Ok(Some(dflt)),
            Some(Val::Null) => Ok(None),
            Some(Val::Int(i)) => Ok(Some(if i < 0 { (n + i).max(0) } else { i.min(n) })),
            Some(o) => Err(type_err("slice bound", &o, None)),
        }
    };
    let (Some(s), Some(e)) = (bound(lo, 0)?, bound(hi, n)?) else {
        return Ok(Val::Null);
    };
    if s >= e {
        return Ok(Val::List(Vec::new()));
    }
    Ok(Val::List(l[s as usize..e as usize].to_vec()))
}

fn temporal_component(v: &Val, key: &str) -> CResult<Val> {
    use chrono::{Datelike, Timelike};
    let (ms_local, tz): (i64, Option<i16>) = match v {
        Val::Date(d) => (d * 86_400_000, None),
        Val::DateTime { ms, tz } => (ms + *tz as i64 * 60_000, Some(*tz)),
        Val::LocalDateTime(ms) => (*ms, None),
        _ => return Ok(Val::Null),
    };
    let Some(dt) = chrono::DateTime::from_timestamp_millis(ms_local) else {
        return Ok(Val::Null);
    };
    let dt = dt.naive_utc();
    Ok(match key {
        "year" => Val::Int(dt.year() as i64),
        "month" => Val::Int(dt.month() as i64),
        "day" => Val::Int(dt.day() as i64),
        "hour" => Val::Int(dt.hour() as i64),
        "minute" => Val::Int(dt.minute() as i64),
        "second" => Val::Int(dt.second() as i64),
        "millisecond" => Val::Int((dt.nanosecond() / 1_000_000) as i64),
        "epochMillis" => match v {
            Val::DateTime { ms, .. } => Val::Int(*ms),
            _ => Val::Null,
        },
        "epochSeconds" => match v {
            Val::DateTime { ms, .. } => Val::Int(ms.div_euclid(1000)),
            _ => Val::Null,
        },
        "timezone" | "offset" => match tz {
            Some(0) => Val::Str("Z".into()),
            Some(m) => Val::Str(format!(
                "{}{:02}:{:02}",
                if m < 0 { '-' } else { '+' },
                m.abs() / 60,
                m.abs() % 60
            )),
            None => Val::Null,
        },
        _ => Val::Null,
    })
}
