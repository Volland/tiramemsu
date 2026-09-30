//! Semantic checks that run before anything executes: undefined variables, kind
//! conflicts, aggregate placement, `UNION` columns, missing parameters, `SKIP`/`LIMIT`
//! arguments, writes on read-only handles and writes under a time clause.

use std::collections::BTreeSet;

use super::scope::{Kind, Scope};
use crate::ast::*;
use crate::error::{CResult, CypherError};
use crate::funcs;
use crate::span::Span;

/// What the checks need to know about the call.
pub struct CheckOpts<'a> {
    /// Names of the supplied parameters.
    pub params: &'a BTreeSet<String>,
    /// True when the handle can write.
    pub writable: bool,
}

/// A result column of a (sub)query.
#[derive(Clone, Debug)]
pub struct Col {
    /// Column name.
    pub name: String,
    /// The kind it carries.
    pub kind: Kind,
    /// Where it was written.
    pub span: Span,
}

/// Runs every check on a whole query.
pub fn check(q: &Query, opts: &CheckOpts) -> CResult<()> {
    if let Some((name, span)) = first_write(q) {
        if !opts.writable {
            return Err(CypherError::unsupported(
                format!("{name} on a read-only view"),
                Some(span),
            ));
        }
        for p in &q.parts {
            if let Some(t) = &p.time {
                let now = t.tx.is_none() && t.valid.is_none();
                if !now {
                    return Err(CypherError::unsupported(
                        "USE with a write clause (writes apply to the current state)",
                        Some(t.span),
                    ));
                }
            }
        }
    }
    check_query(q, &Scope::new(), opts, true).map(|_| ())
}

fn clause_write_name(c: &Clause) -> Option<(&'static str, Span)> {
    Some(match c {
        Clause::Create { span, .. } => ("CREATE", *span),
        Clause::Merge { span, .. } => ("MERGE", *span),
        Clause::Set(items) => ("SET", set_span(items)),
        Clause::Remove(_) => ("REMOVE", Span::new(0, 0)),
        Clause::Delete { detach, span, .. } => {
            (if *detach { "DETACH DELETE" } else { "DELETE" }, *span)
        }
        _ => return None,
    })
}

fn set_span(items: &[SetItem]) -> Span {
    match items.first() {
        Some(SetItem::Prop { span, .. })
        | Some(SetItem::Replace { span, .. })
        | Some(SetItem::Merge { span, .. })
        | Some(SetItem::Labels { span, .. }) => *span,
        None => Span::new(0, 0),
    }
}

/// The first write clause anywhere in the query (subqueries included).
pub fn first_write(q: &Query) -> Option<(&'static str, Span)> {
    for p in &q.parts {
        for c in &p.clauses {
            if let Some(w) = clause_write_name(c) {
                return Some(w);
            }
            if let Clause::Subquery { body, .. } = c {
                if let Some(w) = first_write(body) {
                    return Some(w);
                }
            }
        }
    }
    None
}

fn check_query(q: &Query, imports: &Scope, opts: &CheckOpts, top: bool) -> CResult<Vec<Col>> {
    let mut first_cols: Option<Vec<Col>> = None;
    if q.unions.iter().any(|a| *a) && q.unions.iter().any(|a| !*a) {
        return Err(CypherError::parse(
            q.span,
            "UNION and UNION ALL cannot be mixed",
        ));
    }
    for (i, part) in q.parts.iter().enumerate() {
        let cols = check_single(part, imports, opts)?;
        if q.parts.len() > 1 {
            let Some(c) = &cols else {
                return Err(CypherError::parse(
                    part.span,
                    "each UNION branch must end with RETURN",
                ));
            };
            match &first_cols {
                None => first_cols = Some(c.clone()),
                Some(f) => {
                    let a: Vec<&str> = f.iter().map(|c| c.name.as_str()).collect();
                    let b: Vec<&str> = c.iter().map(|c| c.name.as_str()).collect();
                    if a != b {
                        return Err(CypherError::parse(
                            part.span,
                            format!("UNION branches must return the same columns ({a:?} vs {b:?})"),
                        ));
                    }
                }
            }
            let _ = i;
        } else {
            first_cols = cols;
        }
    }
    let _ = top;
    Ok(first_cols.unwrap_or_default())
}

/// Checks one branch; returns its columns when it ends with `RETURN`.
fn check_single(s: &SingleQuery, imports: &Scope, opts: &CheckOpts) -> CResult<Option<Vec<Col>>> {
    let mut scope = imports.clone();
    let mut cols = None;
    let n = s.clauses.len();
    if let Some(t) = &s.time {
        check_time(t, opts)?;
    }
    for (i, c) in s.clauses.iter().enumerate() {
        if let Clause::Return(p) = c {
            if i + 1 != n {
                return Err(CypherError::parse(p.span, "RETURN must be the last clause"));
            }
        }
        cols = check_clause(c, &mut scope, opts)?.or(cols);
    }
    Ok(cols)
}

fn bind_pattern(p: &Pattern, scope: &mut Scope, opts: &CheckOpts, create: bool) -> CResult<()> {
    let mut rel_names: Vec<&str> = Vec::new();
    let mut names_in_pattern: Vec<&str> = Vec::new();
    for part in p {
        for n in &part.nodes {
            if let Some(v) = &n.var {
                names_in_pattern.push(&v.text);
            }
        }
        for r in &part.rels {
            if let Some(v) = &r.var {
                names_in_pattern.push(&v.text);
                if rel_names.contains(&v.text.as_str()) {
                    return Err(CypherError::parse(
                        v.span,
                        format!(
                            "relationship variable `{}` is used twice in one pattern",
                            v.text
                        ),
                    ));
                }
                rel_names.push(&v.text);
            }
        }
    }
    for part in p {
        if let Some(b) = &part.binding {
            if scope.get(&b.text).is_some() || names_in_pattern.contains(&b.text.as_str()) {
                return Err(CypherError::parse(
                    b.span,
                    format!("variable `{}` is already bound", b.text),
                ));
            }
        }
        for n in &part.nodes {
            if let Some(v) = &n.var {
                if create
                    && scope.get(&v.text).is_some()
                    && (!n.labels.is_empty() || n.props.is_some())
                {
                    return Err(CypherError::parse(
                        v.span,
                        format!(
                            "variable `{}` is already bound; labels and properties cannot be added",
                            v.text
                        ),
                    ));
                }
                scope.bind_node(v)?;
            }
        }
        for r in &part.rels {
            if let Some(v) = &r.var {
                if create && scope.get(&v.text).is_some() {
                    return Err(CypherError::parse(
                        v.span,
                        format!("variable `{}` already declared", v.text),
                    ));
                }
                scope.bind_rel(v)?;
            }
            if create {
                if r.types.len() != 1 {
                    return Err(CypherError::parse(
                        r.span,
                        "a created relationship needs exactly one type",
                    ));
                }
                if r.dir == Dir::Either {
                    return Err(CypherError::parse(
                        r.span,
                        "a created relationship needs a direction",
                    ));
                }
            }
        }
        if let Some(b) = &part.binding {
            scope.set(&b.text, Kind::Path);
        }
    }
    for part in p {
        for n in &part.nodes {
            if let Some(e) = &n.props {
                check_expr(e, scope, opts, Ctx::plain())?;
            }
        }
        for r in &part.rels {
            if let Some(e) = &r.props {
                check_expr(e, scope, opts, Ctx::plain())?;
            }
        }
    }
    Ok(())
}

fn check_clause(c: &Clause, scope: &mut Scope, opts: &CheckOpts) -> CResult<Option<Vec<Col>>> {
    match c {
        Clause::Match {
            pattern, where_, ..
        } => {
            bind_pattern(pattern, scope, opts, false)?;
            if let Some(w) = where_ {
                check_expr(w, scope, opts, Ctx::where_())?;
            }
        }
        Clause::Unwind { expr, var } => {
            check_expr(expr, scope, opts, Ctx::plain())?;
            scope.declare_value(&var.text);
        }
        Clause::Create { pattern, .. } => bind_pattern(pattern, scope, opts, true)?,
        Clause::Merge {
            part,
            on_create,
            on_match,
            ..
        } => {
            let p = vec![part.clone()];
            bind_pattern(&p, scope, opts, false)?;
            for r in &part.rels {
                if r.types.len() != 1 {
                    return Err(CypherError::parse(
                        r.span,
                        "MERGE needs exactly one relationship type",
                    ));
                }
                if r.dir == Dir::Either {
                    // undirected MERGE creates left to right; allowed
                }
            }
            for it in on_create.iter().chain(on_match) {
                check_set_item(it, scope, opts)?;
            }
        }
        Clause::Set(items) => {
            for it in items {
                check_set_item(it, scope, opts)?;
            }
        }
        Clause::Remove(items) => {
            for it in items {
                match it {
                    RemoveItem::Prop(e) => {
                        check_expr(e, scope, opts, Ctx::plain())?;
                    }
                    RemoveItem::Labels { target, .. } => {
                        need_var(&target.text, target.span, scope)?
                    }
                }
            }
        }
        Clause::Delete { exprs, .. } => {
            for e in exprs {
                check_expr(e, scope, opts, Ctx::plain())?;
                if matches!(
                    e.kind,
                    ExprKind::Lit(_)
                        | ExprKind::Binary(..)
                        | ExprKind::List(_)
                        | ExprKind::Map(_)
                        | ExprKind::HasLabels(..)
                ) {
                    return Err(CypherError::parse(
                        e.span,
                        "DELETE needs a node, relationship or path",
                    ));
                }
            }
        }
        Clause::With { proj, where_ } => {
            let (new_scope, _) = check_projection(proj, scope, opts, true)?;
            if let Some(w) = where_ {
                // the WHERE of a WITH also sees the variables bound before it
                let mut wide = scope.clone();
                for n in new_scope.names() {
                    wide.set(&n, new_scope.get(&n).unwrap_or(Kind::Value));
                }
                check_expr(w, &wide, opts, Ctx::where_())?;
            }
            *scope = new_scope;
        }
        Clause::Return(proj) => {
            let (_, cols) = check_projection(proj, scope, opts, false)?;
            return Ok(Some(cols));
        }
        Clause::Subquery {
            body,
            imports,
            span,
        } => {
            let mut inner = Scope::new();
            for i in imports {
                match scope.get(&i.text) {
                    Some(k) => inner.set(&i.text, k),
                    None => return Err(Scope::undefined(&i.text, i.span)),
                }
            }
            let cols = check_query(body, &inner, opts, false)?;
            let _ = span;
            for col in cols {
                if scope.get(&col.name).is_some() {
                    return Err(CypherError::parse(
                        col.span,
                        format!(
                            "returned variable `{}` shadows a variable of the outer query",
                            col.name
                        ),
                    ));
                }
                scope.set(&col.name, col.kind);
            }
        }
        Clause::Procedure {
            name,
            args,
            yields,
            span,
        } => {
            let known = ["db.labels", "db.relationshiptypes", "db.propertykeys"];
            if !known.contains(&name.to_ascii_lowercase().as_str()) {
                return Err(CypherError::unsupported(
                    format!("procedure `{name}`"),
                    Some(*span),
                ));
            }
            for a in args {
                check_expr(a, scope, opts, Ctx::plain())?;
            }
            for (col, alias) in yields {
                let n = alias.as_ref().unwrap_or(col);
                scope.declare_value(&n.text);
            }
        }
    }
    Ok(None)
}

fn need_var(name: &str, span: Span, scope: &Scope) -> CResult<()> {
    if scope.get(name).is_none() {
        return Err(Scope::undefined(name, span));
    }
    Ok(())
}

fn check_set_item(it: &SetItem, scope: &Scope, opts: &CheckOpts) -> CResult<()> {
    match it {
        SetItem::Prop { target, value, .. } => {
            check_expr(target, scope, opts, Ctx::plain())?;
            check_expr(value, scope, opts, Ctx::plain()).map(|_| ())
        }
        SetItem::Replace { target, value, .. } | SetItem::Merge { target, value, .. } => {
            need_var(&target.text, target.span, scope)?;
            check_expr(value, scope, opts, Ctx::plain()).map(|_| ())
        }
        SetItem::Labels { target, .. } => need_var(&target.text, target.span, scope),
    }
}

fn is_const_arg(e: &Expr) -> Option<Span> {
    match &e.kind {
        ExprKind::Var(_) => Some(e.span),
        ExprKind::Unary(_, x) => is_const_arg(x),
        ExprKind::Binary(_, a, b) => is_const_arg(a).or_else(|| is_const_arg(b)),
        ExprKind::Call { args, .. } => args.iter().find_map(is_const_arg),
        ExprKind::Prop(x, _) => is_const_arg(x),
        _ => None,
    }
}

fn check_limit(e: &Expr, what: &str, scope: &Scope, opts: &CheckOpts) -> CResult<()> {
    match &e.kind {
        ExprKind::Lit(Lit::Int(i)) if *i < 0 => {
            return Err(CypherError::parse(
                e.span,
                format!("{what} must not be negative"),
            ))
        }
        ExprKind::Lit(Lit::Int(_)) => {}
        ExprKind::Lit(Lit::Float(_)) | ExprKind::Lit(Lit::Str(_)) | ExprKind::Lit(Lit::Bool(_)) => {
            return Err(CypherError::parse(
                e.span,
                format!("{what} must be an integer"),
            ))
        }
        _ => {
            if let Some(sp) = is_const_arg(e) {
                return Err(CypherError::parse(
                    sp,
                    format!("{what} must not depend on variables"),
                ));
            }
            check_expr(e, scope, opts, Ctx::plain())?;
        }
    }
    Ok(())
}

/// Checks a `WITH`/`RETURN` body and returns the scope after it and its columns.
fn check_projection(
    p: &Projection,
    scope: &Scope,
    opts: &CheckOpts,
    is_with: bool,
) -> CResult<(Scope, Vec<Col>)> {
    let mut out = Scope::new();
    let mut cols: Vec<Col> = Vec::new();
    let mut any_agg = false;
    if p.star {
        if scope.is_empty() && p.items.is_empty() {
            return Err(CypherError::parse(
                p.span,
                "RETURN * has no variables in scope",
            ));
        }
        for n in scope.names() {
            out.set(&n, scope.get(&n).unwrap_or(Kind::Value));
            cols.push(Col {
                name: n.clone(),
                kind: scope.get(&n).unwrap_or(Kind::Value),
                span: p.span,
            });
        }
    }
    for it in &p.items {
        let agg = check_expr(
            &it.expr,
            scope,
            opts,
            Ctx {
                allow_agg: true,
                ..Ctx::plain()
            },
        )?;
        any_agg |= agg;
        if agg {
            ambiguous_aggregation(&it.expr)?;
        }
        let name = match &it.alias {
            Some(a) => a.text.clone(),
            None => it.text.clone(),
        };
        if is_with && it.alias.is_none() && !matches!(it.expr.kind, ExprKind::Var(_)) {
            return Err(CypherError::parse(
                it.span,
                "an expression in WITH needs an alias",
            ));
        }
        let kind = match &it.expr.kind {
            ExprKind::Var(v) => scope.get(v).unwrap_or(Kind::Value),
            _ => Kind::Value,
        };
        if out.get(&name).is_some() && !p.star {
            return Err(CypherError::parse(
                it.span,
                format!("duplicate column name `{name}`"),
            ));
        }
        if p.star && out.get(&name).is_some() {
            return Err(CypherError::parse(
                it.span,
                format!("duplicate column name `{name}`"),
            ));
        }
        out.set(&name, kind);
        cols.push(Col {
            name,
            kind,
            span: it.span,
        });
    }
    if !p.order.is_empty() {
        if p.distinct || any_agg {
            // after DISTINCT or aggregation an ORDER BY item is computed from the projected
            // columns; input variables are visible only inside aggregate arguments, and an
            // expression identical to a projected item is that column
            let shapes: Vec<String> = p.items.iter().map(|it| shape(&it.expr)).collect();
            let mut out_scope = Scope::new();
            for n in out.names() {
                out_scope.set(&n, out.get(&n).unwrap_or(Kind::Value));
            }
            for s in &p.order {
                check_order_expr(&s.expr, &out_scope, scope, &shapes, opts, any_agg)?;
            }
        } else {
            let mut ord_scope = scope.clone();
            for n in out.names() {
                ord_scope.set(&n, out.get(&n).unwrap_or(Kind::Value));
            }
            for s in &p.order {
                check_expr(&s.expr, &ord_scope, opts, Ctx::plain())?;
            }
        }
    }
    if let Some(s) = &p.skip {
        check_limit(s, "SKIP", scope, opts)?;
    }
    if let Some(l) = &p.limit {
        check_limit(l, "LIMIT", scope, opts)?;
    }
    Ok((out, cols))
}

/// Expression-check context.
#[derive(Copy, Clone)]
pub struct Ctx {
    allow_agg: bool,
    in_agg: bool,
    in_where: bool,
}

impl Ctx {
    fn plain() -> Ctx {
        Ctx {
            allow_agg: false,
            in_agg: false,
            in_where: false,
        }
    }

    fn where_() -> Ctx {
        Ctx {
            in_where: true,
            ..Ctx::plain()
        }
    }
}

/// Checks an expression; returns true when it contains an aggregate.
fn check_expr(e: &Expr, scope: &Scope, opts: &CheckOpts, cx: Ctx) -> CResult<bool> {
    let mut agg = false;
    let mut sub = |x: &Expr, scope: &Scope| -> CResult<()> {
        agg |= check_expr(x, scope, opts, cx)?;
        Ok(())
    };
    match &e.kind {
        ExprKind::Lit(_) => {}
        ExprKind::Var(v) => {
            if scope.get(v).is_none() {
                return Err(Scope::undefined(v, e.span));
            }
        }
        ExprKind::Param(p) => {
            if !opts.params.contains(p) {
                return Err(CypherError::parse(
                    e.span,
                    format!("missing parameter `${p}`"),
                ));
            }
        }
        ExprKind::CountStar => {
            agg_position(e, cx)?;
            return Ok(true);
        }
        ExprKind::List(xs) => {
            for x in xs {
                sub(x, scope)?;
            }
        }
        ExprKind::Map(es) => {
            for (_, x) in es {
                sub(x, scope)?;
            }
        }
        ExprKind::MapProj(b, items) => {
            sub(b, scope)?;
            for it in items {
                match it {
                    MapProjItem::Var(n) => need_var(&n.text, n.span, scope)?,
                    MapProjItem::Entry(_, x) => sub(x, scope)?,
                    _ => {}
                }
            }
        }
        ExprKind::Unary(_, x)
        | ExprKind::IsNull(x, _)
        | ExprKind::Prop(x, _)
        | ExprKind::HasLabels(x, _) => sub(x, scope)?,
        ExprKind::Binary(_, a, b) | ExprKind::Index(a, b) => {
            sub(a, scope)?;
            sub(b, scope)?;
        }
        ExprKind::Slice(a, lo, hi) => {
            sub(a, scope)?;
            if let Some(x) = lo {
                sub(x, scope)?;
            }
            if let Some(x) = hi {
                sub(x, scope)?;
            }
        }
        ExprKind::Call { name, args, .. } => {
            if !funcs::is_known(name) {
                return Err(CypherError::unsupported(
                    format!("function `{name}`"),
                    Some(e.span),
                ));
            }
            if funcs::is_aggregate(name) {
                agg_position(e, cx)?;
                let inner = Ctx {
                    allow_agg: false,
                    in_agg: true,
                    in_where: cx.in_where,
                };
                for a in args {
                    check_expr(a, scope, opts, inner)?;
                }
                return Ok(true);
            }
            for a in args {
                sub(a, scope)?;
            }
        }
        ExprKind::Case { operand, alts, els } => {
            if let Some(o) = operand {
                sub(o, scope)?;
            }
            for (w, t) in alts {
                sub(w, scope)?;
                sub(t, scope)?;
            }
            if let Some(x) = els {
                sub(x, scope)?;
            }
        }
        ExprKind::ListComp {
            var,
            list,
            pred,
            proj,
        } => {
            sub(list, scope)?;
            let mut inner = scope.clone();
            inner.declare_value(&var.text);
            let no_agg = Ctx {
                allow_agg: false,
                in_agg: false,
                ..cx
            };
            if let Some(p) = pred {
                check_expr(p, &inner, opts, no_agg)?;
            }
            if let Some(p) = proj {
                check_expr(p, &inner, opts, no_agg)?;
            }
        }
        ExprKind::Quantifier {
            var, list, pred, ..
        } => {
            sub(list, scope)?;
            let mut inner = scope.clone();
            inner.declare_value(&var.text);
            sub(pred, &inner)?;
        }
        ExprKind::Reduce {
            acc,
            init,
            var,
            list,
            body,
        } => {
            sub(init, scope)?;
            sub(list, scope)?;
            let mut inner = scope.clone();
            inner.declare_value(&acc.text);
            inner.declare_value(&var.text);
            sub(body, &inner)?;
        }
        ExprKind::Exists(b) => match &**b {
            ExistsBody::Pattern(p, w) => {
                // a bare pattern predicate: only in WHERE, over bound variables, with a relationship
                if !cx.in_where {
                    return Err(CypherError::parse(
                        e.span,
                        "a pattern expression is only allowed in a WHERE predicate",
                    ));
                }
                if p.iter().all(|x| x.rels.is_empty()) {
                    return Err(CypherError::parse(
                        e.span,
                        "a pattern predicate needs a relationship",
                    ));
                }
                for part in p {
                    for n in &part.nodes {
                        if let Some(v) = &n.var {
                            if scope.get(&v.text).is_none() {
                                return Err(Scope::undefined(&v.text, v.span));
                            }
                        }
                    }
                    for r in &part.rels {
                        if let Some(v) = &r.var {
                            if scope.get(&v.text).is_none() {
                                return Err(Scope::undefined(&v.text, v.span));
                            }
                        }
                    }
                }
                let mut inner = scope.clone();
                bind_pattern(p, &mut inner, opts, false)?;
                if let Some(w) = w {
                    check_expr(w, &inner, opts, Ctx::where_())?;
                }
            }
            ExistsBody::Query(q) => {
                if let Some((name, span)) = first_write(q) {
                    return Err(CypherError::unsupported(
                        format!("{name} inside EXISTS"),
                        Some(span),
                    ));
                }
                check_query(q, scope, opts, false)?;
            }
        },
    }
    Ok(agg)
}

fn agg_position(e: &Expr, cx: Ctx) -> CResult<()> {
    if cx.in_agg {
        return Err(CypherError::parse(
            e.span,
            "aggregate functions cannot be nested",
        ));
    }
    if !cx.allow_agg {
        return Err(CypherError::parse(
            e.span,
            "aggregate function used in a position that does not allow it",
        ));
    }
    Ok(())
}

/// Argument validation of a `USE` clause (task 9.5).
fn check_time(t: &TimeSel, opts: &CheckOpts) -> CResult<()> {
    let chk_param = |a: &TimeArg, sp: Span| -> CResult<()> {
        if let TimeArg::Param(p) = a {
            if !opts.params.contains(p) {
                return Err(CypherError::parse(sp, format!("missing parameter `${p}`")));
            }
        }
        Ok(())
    };
    if let Some(TxClause::AsOf(a, sp)) = &t.tx {
        chk_param(a, *sp)?;
        if matches!(a, TimeArg::Date(_)) {
            return Err(CypherError::parse(
                *sp,
                "AS OF takes a transaction number or a datetime, not a date",
            ));
        }
    }
    if let Some((a, sp)) = &t.valid {
        chk_param(a, *sp)?;
        if matches!(a, TimeArg::Int(_)) {
            return Err(CypherError::parse(
                *sp,
                "VALID AT takes a date or a datetime, not an integer",
            ));
        }
    }
    Ok(())
}

/// A span-free rendering of common expression forms; `#` marks forms that never match.
fn shape(e: &Expr) -> String {
    match &e.kind {
        ExprKind::Var(v) => format!("v:{v}"),
        ExprKind::Lit(l) => format!("l:{l:?}"),
        ExprKind::Param(p) => format!("p:{p}"),
        ExprKind::Prop(b, k) => format!("({}).{}", shape(b), k.text),
        ExprKind::CountStar => "count(*)".into(),
        ExprKind::Call {
            name,
            distinct,
            args,
        } => format!(
            "{}({}{})",
            name.to_ascii_lowercase(),
            if *distinct { "distinct " } else { "" },
            args.iter().map(shape).collect::<Vec<_>>().join(",")
        ),
        ExprKind::Binary(op, a, b) => format!("({} {op:?} {})", shape(a), shape(b)),
        ExprKind::Unary(op, a) => format!("({op:?} {})", shape(a)),
        ExprKind::Index(a, b) => format!("({})[{}]", shape(a), shape(b)),
        _ => format!("#{}-{}", e.span.start, e.span.end),
    }
}

/// Checks an `ORDER BY` item of a DISTINCT or aggregating projection.
fn check_order_expr(
    e: &Expr,
    out: &Scope,
    input: &Scope,
    shapes: &[String],
    opts: &CheckOpts,
    allow_agg: bool,
) -> CResult<()> {
    if shapes.contains(&shape(e)) {
        return Ok(());
    }
    match &e.kind {
        ExprKind::CountStar => Err(CypherError::parse(
            e.span,
            "an aggregate in ORDER BY must also appear in the projection",
        )),
        ExprKind::Call { name, args, .. } if funcs::is_aggregate(name) => {
            let _ = (args, input, allow_agg);
            Err(CypherError::parse(
                e.span,
                "an aggregate in ORDER BY must also appear in the projection",
            ))
        }
        ExprKind::Var(v) => {
            if out.get(v).is_some() {
                Ok(())
            } else {
                Err(Scope::undefined(v, e.span))
            }
        }
        ExprKind::Lit(_) | ExprKind::Param(_) => check_expr(e, out, opts, Ctx::plain()).map(|_| ()),
        ExprKind::Prop(b, _)
        | ExprKind::Unary(_, b)
        | ExprKind::IsNull(b, _)
        | ExprKind::HasLabels(b, _) => check_order_expr(b, out, input, shapes, opts, allow_agg),
        ExprKind::Binary(_, a, b) | ExprKind::Index(a, b) => {
            check_order_expr(a, out, input, shapes, opts, allow_agg)?;
            check_order_expr(b, out, input, shapes, opts, allow_agg)
        }
        ExprKind::Call { args, .. } => {
            for a in args {
                check_order_expr(a, out, input, shapes, opts, allow_agg)?;
            }
            Ok(())
        }
        _ => check_expr(
            e,
            out,
            opts,
            Ctx {
                allow_agg,
                ..Ctx::plain()
            },
        )
        .map(|_| ()),
    }
}

/// True when the expression mentions a variable outside of aggregate arguments.
fn has_free_var(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Var(_) => true,
        ExprKind::CountStar => false,
        ExprKind::Call { name, args, .. } => {
            !funcs::is_aggregate(name) && args.iter().any(has_free_var)
        }
        ExprKind::Prop(b, _) | ExprKind::Unary(_, b) | ExprKind::IsNull(b, _) => has_free_var(b),
        ExprKind::Binary(_, a, b) | ExprKind::Index(a, b) => has_free_var(a) || has_free_var(b),
        ExprKind::List(xs) => xs.iter().any(has_free_var),
        _ => false,
    }
}

/// An expression that mixes an aggregate with a compound expression of grouping
/// variables is ambiguous (openCypher `AmbiguousAggregationExpression`).
fn ambiguous_aggregation(e: &Expr) -> CResult<()> {
    match &e.kind {
        ExprKind::Binary(_, a, b) => {
            for side in [a, b] {
                if crate::exec::project::contains_agg(side) {
                    ambiguous_aggregation(side)?;
                } else if has_free_var(side)
                    && !matches!(side.kind, ExprKind::Var(_) | ExprKind::Prop(..))
                {
                    return Err(CypherError::parse(
                        side.span,
                        "a compound expression of grouping keys is ambiguous next to an aggregate",
                    ));
                }
            }
            Ok(())
        }
        ExprKind::Unary(_, x) | ExprKind::Prop(x, _) => ambiguous_aggregation(x),
        _ => Ok(()),
    }
}
