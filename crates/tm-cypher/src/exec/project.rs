//! `WITH` / `RETURN`: projection, aggregation, `DISTINCT`, ordering, `SKIP`/`LIMIT`.

use std::collections::{HashMap, HashSet};

use super::{Exec, Row};
use crate::ast::*;
use crate::error::{CResult, CypherError};
use crate::funcs;
use crate::value::{group_key, Val};

/// True when the expression contains an aggregate call.
pub fn contains_agg(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::CountStar => true,
        ExprKind::Call { name, args, .. } => {
            funcs::is_aggregate(name) || args.iter().any(contains_agg)
        }
        ExprKind::List(xs) => xs.iter().any(contains_agg),
        ExprKind::Map(es) => es.iter().any(|(_, x)| contains_agg(x)),
        ExprKind::MapProj(b, items) => {
            contains_agg(b)
                || items
                    .iter()
                    .any(|i| matches!(i, MapProjItem::Entry(_, x) if contains_agg(x)))
        }
        ExprKind::Unary(_, x)
        | ExprKind::IsNull(x, _)
        | ExprKind::Prop(x, _)
        | ExprKind::HasLabels(x, _) => contains_agg(x),
        ExprKind::Binary(_, a, b) | ExprKind::Index(a, b) => contains_agg(a) || contains_agg(b),
        ExprKind::Slice(a, l, h) => {
            contains_agg(a)
                || l.as_deref().is_some_and(contains_agg)
                || h.as_deref().is_some_and(contains_agg)
        }
        ExprKind::Case { operand, alts, els } => {
            operand.as_deref().is_some_and(contains_agg)
                || alts.iter().any(|(w, t)| contains_agg(w) || contains_agg(t))
                || els.as_deref().is_some_and(contains_agg)
        }
        ExprKind::ListComp {
            list, pred, proj, ..
        } => {
            contains_agg(list)
                || pred.as_deref().is_some_and(contains_agg)
                || proj.as_deref().is_some_and(contains_agg)
        }
        ExprKind::Quantifier { list, pred, .. } => contains_agg(list) || contains_agg(pred),
        ExprKind::Reduce {
            init, list, body, ..
        } => contains_agg(init) || contains_agg(list) || contains_agg(body),
        _ => false,
    }
}

/// (output row, source row for ORDER BY, group of an aggregating projection)
type Projected = (Row, Row, Option<Vec<Row>>);

impl Exec<'_> {
    fn count_arg(&mut self, e: &Expr, what: &str) -> CResult<usize> {
        match self.eval(e, &Row::new())? {
            Val::Int(i) if i >= 0 => Ok(i as usize),
            Val::Int(_) => Err(CypherError::eval(format!("{what} must not be negative"))),
            other => Err(CypherError::eval(format!(
                "{what} must be an integer, got {}",
                other.type_name()
            ))),
        }
    }

    /// Projects `rows` through `p`; `scope` names the variables in scope (for `*`).
    pub(crate) fn project(
        &mut self,
        rows: Vec<Row>,
        p: &Projection,
        where_: Option<&Expr>,
        scope: &[String],
    ) -> CResult<(Vec<Row>, Vec<String>)> {
        let mut names: Vec<String> = Vec::new();
        let mut exprs: Vec<Expr> = Vec::new();
        if p.star {
            for n in scope {
                names.push(n.clone());
                exprs.push(Expr {
                    kind: ExprKind::Var(n.clone()),
                    span: p.span,
                });
            }
        }
        for it in &p.items {
            names.push(match &it.alias {
                Some(a) => a.text.clone(),
                None => it.text.clone(),
            });
            exprs.push(it.expr.clone());
        }
        let has_agg = exprs.iter().any(contains_agg);
        // (output row, source row for ORDER BY, group)
        let mut out: Vec<Projected> = Vec::new();
        if has_agg {
            let key_idx: Vec<usize> = (0..exprs.len())
                .filter(|i| !contains_agg(&exprs[*i]))
                .collect();
            let mut order: Vec<String> = Vec::new();
            let mut groups: HashMap<String, Vec<Row>> = HashMap::new();
            if key_idx.is_empty() {
                order.push(String::new());
                groups.insert(String::new(), rows);
            } else {
                for r in rows {
                    let mut k = String::new();
                    for i in &key_idx {
                        let v = self.eval(&exprs[*i], &r)?;
                        k.push_str(&group_key(&v));
                        k.push('\u{1}');
                    }
                    if !groups.contains_key(&k) {
                        order.push(k.clone());
                    }
                    groups.entry(k).or_default().push(r);
                }
            }
            for k in order {
                let g = groups.remove(&k).unwrap_or_default();
                let first = g.first().cloned().unwrap_or_default();
                let mut row = Row::new();
                for (i, e) in exprs.iter().enumerate() {
                    let v = self.eval_g(e, &first, Some(&g))?;
                    row.insert(names[i].clone(), v);
                }
                out.push((row, first, Some(g)));
            }
        } else {
            for r in rows {
                let mut row = Row::new();
                for (i, e) in exprs.iter().enumerate() {
                    let v = self.eval(e, &r)?;
                    row.insert(names[i].clone(), v);
                }
                out.push((row, r, None));
            }
        }
        if p.distinct {
            let mut seen = HashSet::new();
            out.retain(|(row, _, _)| {
                let key: Vec<String> = names
                    .iter()
                    .map(|n| group_key(row.get(n).unwrap_or(&Val::Null)))
                    .collect();
                seen.insert(key.join("\u{1}"))
            });
        }
        if !p.order.is_empty() {
            let mut keyed: Vec<(Vec<Val>, usize)> = Vec::with_capacity(out.len());
            for (i, (row, src, g)) in out.iter().enumerate() {
                let mut merged = src.clone();
                for (k, v) in row {
                    merged.insert(k.clone(), v.clone());
                }
                let mut ks = Vec::with_capacity(p.order.len());
                for s in &p.order {
                    ks.push(self.eval_g(&s.expr, &merged, g.as_deref())?);
                }
                keyed.push((ks, i));
            }
            let desc: Vec<bool> = p.order.iter().map(|s| s.desc).collect();
            Exec::sort_values(&mut keyed, &desc);
            let mut slots: Vec<Option<Projected>> = out.into_iter().map(Some).collect();
            out = keyed
                .into_iter()
                .filter_map(|(_, i)| slots[i].take())
                .collect();
        }
        let skip = match &p.skip {
            Some(e) => self.count_arg(e, "SKIP")?,
            None => 0,
        };
        let limit = match &p.limit {
            Some(e) => Some(self.count_arg(e, "LIMIT")?),
            None => None,
        };
        let window = out.into_iter().skip(skip).take(limit.unwrap_or(usize::MAX));
        let mut rows: Vec<Row> = Vec::new();
        for (r, src, g) in window {
            if let Some(w) = where_ {
                // the WHERE of a WITH also sees the variables bound before it
                let mut merged = src;
                for (k, v) in &r {
                    merged.insert(k.clone(), v.clone());
                }
                if !matches!(self.eval_g(w, &merged, g.as_deref())?, Val::Bool(true)) {
                    continue;
                }
            }
            rows.push(r);
        }
        Ok((rows, names))
    }
}
