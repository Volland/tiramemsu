//! The only module that knows the upstream `open-cypher` AST: converts it into
//! [`crate::ast`] and reports constructs outside the v1 subset as `Unsupported`.

use open_cypher::ast as up;
use open_cypher::ast::{ClauseKind, QueryKind, StatementKind};

use super::prepass::{self, Prepass};
use crate::ast::*;
use crate::error::{CResult, CypherError};
use crate::span::Span;

/// An extracted `CALL { … }` subquery (the upstream grammar has no such clause).
pub(super) struct Sub {
    pub(super) query: Query,
    pub(super) imports: Vec<Name>,
}

pub(super) struct Ctx<'a> {
    pub(super) src: &'a str,
    pub(super) pre: &'a Prepass,
    pub(super) subs: &'a [Sub],
}

impl Ctx<'_> {
    pub(super) fn text(&self, s: Span) -> String {
        s.text(self.src).unwrap_or("").trim().to_string()
    }

    pub(super) fn name(&self, n: &up::Name) -> Name {
        Name {
            text: n.kind.text.clone(),
            escaped: n.kind.escaped,
            span: n.span.into(),
        }
    }

    fn take_ext(&self, at: usize) -> Option<TimeSel> {
        self.pre
            .scopes
            .iter()
            .find(|e| e.next_tok == at)
            .map(|e| e.time.clone())
    }
}

/// Parses `text` (extensions included) into the internal AST.
pub fn parse(text: &str) -> CResult<Query> {
    parse_range(text, 0, text.len())
}

/// Parses the bytes `lo..hi` of `text`, keeping every offset of the original.
pub(super) fn parse_range(text: &str, lo: usize, hi: usize) -> CResult<Query> {
    let masked = super::subq::mask(text, lo, hi);
    let (main, subs) = super::subq::extract(text, &masked)?;
    let pre = prepass::run_in(&main, lo != 0 || hi != text.len())?;
    let parsed = match open_cypher::parse(&pre.blanked) {
        Ok(p) => p,
        Err(errs) => return Err(map_parse_errors(text, &pre.blanked, &errs)),
    };
    let ctx = Ctx {
        src: text,
        pre: &pre,
        subs: &subs,
    };
    let Some(stmt) = parsed.program.statements.first() else {
        return Err(CypherError::parse(Span::new(lo, hi), "empty query"));
    };
    match &stmt.kind {
        StatementKind::Query(q) => convert_query(&ctx, &q.query),
        _ => Err(CypherError::parse(stmt.span.into(), "invalid statement")),
    }
}

fn map_parse_errors(_text: &str, lexed: &str, errs: &open_cypher::ParseErrors) -> CypherError {
    let first = errs.diagnostics().first();
    let (span, msg) = match first {
        Some(d) => (Span::from(d.primary_span), d.message.clone()),
        None => (Span::new(0, 0), "syntax error".to_string()),
    };
    // Schema commands and other statements outside the subset are recognised by
    // their first words.
    let all: Vec<open_cypher::Token> = open_cypher::lex(lexed)
        .tokens
        .into_iter()
        .filter(|t| !t.kind.is_trivia())
        .collect();
    let word = |i: usize| -> String {
        all.get(i)
            .map(|t| t.text(lexed).unwrap_or("").to_ascii_uppercase())
            .unwrap_or_default()
    };
    for i in 0..all.len() {
        let after_dot = i > 0 && all[i - 1].kind == open_cypher::TokenKind::Dot;
        if !after_dot
            && word(i) == "FOREACH"
            && all.get(i + 1).map(|t| t.kind) == Some(open_cypher::TokenKind::LeftParen)
        {
            return CypherError::unsupported("FOREACH", Some(all[i].span.into()));
        }
        if word(i) == "LOAD" && word(i + 1) == "CSV" {
            return CypherError::unsupported("LOAD CSV", Some(all[i].span.into()));
        }
    }
    let words = [word(0), word(1)];
    let w0 = words.first().map(String::as_str).unwrap_or("");
    let w1 = words.get(1).map(String::as_str).unwrap_or("");
    if matches!(w0, "DROP" | "SHOW" | "ALTER" | "GRANT" | "DENY" | "REVOKE")
        || (w0 == "CREATE"
            && matches!(
                w1,
                "INDEX"
                    | "CONSTRAINT"
                    | "TEXT"
                    | "RANGE"
                    | "POINT"
                    | "LOOKUP"
                    | "FULLTEXT"
                    | "VECTOR"
                    | "DATABASE"
                    | "OR"
                    | "USER"
                    | "ROLE"
            ))
    {
        return CypherError::unsupported(format!("schema command `{w0} {w1}`"), Some(span));
    }
    CypherError::parse(span, msg)
}

pub(super) fn convert_query(ctx: &Ctx, q: &up::Query) -> CResult<Query> {
    let QueryKind::Regular(r) = &q.kind else {
        return Err(CypherError::parse(q.span.into(), "invalid query"));
    };
    let mut parts = vec![convert_single(ctx, &r.head)?];
    let mut unions = Vec::new();
    for u in &r.unions {
        unions.push(matches!(u.operator.kind, up::UnionOperator::All));
        parts.push(convert_single(ctx, &u.query)?);
    }
    Ok(Query {
        parts,
        unions,
        span: q.span.into(),
    })
}

fn convert_single(ctx: &Ctx, sq: &up::SingleQuery) -> CResult<SingleQuery> {
    let clauses_up = &sq.kind.clauses;
    let mut time = clauses_up.first().and_then(|c| ctx.take_ext(c.span.start));
    if time.is_none() {
        if let (Some(a), Some(b)) = (clauses_up.first(), clauses_up.get(1)) {
            if matches!(a.kind, ClauseKind::With(_)) {
                time = ctx.take_ext(b.span.start);
            }
        }
    }
    let mut clauses = Vec::new();
    for c in clauses_up {
        clauses.push(convert_clause(ctx, c)?);
    }
    Ok(SingleQuery {
        time,
        clauses,
        span: sq.span.into(),
    })
}

fn labels_and(ctx: &Ctx, le: &up::LabelExpression) -> CResult<Vec<Vec<Name>>> {
    use up::LabelExpressionKind as K;
    let sp: Span = le.span.into();
    if ctx.text(sp).contains('&') {
        return Err(CypherError::unsupported(
            "label expression with `&`",
            Some(sp),
        ));
    }
    fn one(ctx: &Ctx, le: &up::LabelExpression) -> CResult<Vec<Name>> {
        match &le.kind {
            K::Name(n) => Ok(vec![ctx.name(n)]),
            K::Parenthesized(i) => one(ctx, i),
            K::Or(xs) => {
                let mut out = Vec::new();
                for x in xs {
                    out.extend(one(ctx, x)?);
                }
                Ok(out)
            }
            K::Not(_) => Err(CypherError::unsupported(
                "label expression with `!`",
                Some(le.span.into()),
            )),
            K::Wildcard => Err(CypherError::unsupported(
                "label expression with `%`",
                Some(le.span.into()),
            )),
            _ => Err(CypherError::unsupported(
                "label expression",
                Some(le.span.into()),
            )),
        }
    }
    match &le.kind {
        K::And(xs) => xs.iter().map(|x| one(ctx, x)).collect(),
        K::Parenthesized(i) => labels_and(ctx, i),
        _ => Ok(vec![one(ctx, le)?]),
    }
}

fn plain_labels(ctx: &Ctx, le: &up::LabelExpression) -> CResult<Vec<Name>> {
    let l = labels_and(ctx, le)?;
    let mut out = Vec::new();
    for g in l {
        if g.len() != 1 {
            return Err(CypherError::unsupported(
                "label disjunction in SET/REMOVE",
                Some(le.span.into()),
            ));
        }
        out.extend(g);
    }
    Ok(out)
}

fn convert_node(ctx: &Ctx, n: &up::NodePattern, span: Span) -> CResult<NodePat> {
    if n.where_clause.is_some() {
        return Err(CypherError::unsupported(
            "WHERE inside a node pattern",
            Some(span),
        ));
    }
    Ok(NodePat {
        var: n.variable.as_ref().map(|v| ctx.name(v)),
        labels: match &n.labels {
            Some(le) => labels_and(ctx, le)?,
            None => Vec::new(),
        },
        props: match &n.properties {
            Some(p) => Some(super::adapter_expr::convert_expr(ctx, p)?),
            None => None,
        },
        span,
    })
}

fn convert_rel(ctx: &Ctx, r: &up::RelationshipPattern, span: Span) -> CResult<RelPat> {
    if let Some(q) = &r.legacy_quantifier {
        return Err(CypherError::unsupported(
            "variable-length relationships",
            Some(q.span.into()),
        ));
    }
    if let Some(q) = &r.graph_quantifier {
        return Err(CypherError::unsupported(
            "quantified relationship patterns",
            Some(q.span.into()),
        ));
    }
    if r.where_clause.is_some() {
        return Err(CypherError::unsupported(
            "WHERE inside a relationship pattern",
            Some(span),
        ));
    }
    let types = match &r.labels {
        Some(le) => {
            let l = labels_and(ctx, le)?;
            if l.len() != 1 {
                return Err(CypherError::unsupported(
                    "relationship type conjunction",
                    Some(le.span.into()),
                ));
            }
            l.into_iter().next().unwrap_or_default()
        }
        None => Vec::new(),
    };
    let dir = match r.direction.kind {
        up::RelationshipDirection::Right => Dir::Right,
        up::RelationshipDirection::Left => Dir::Left,
        up::RelationshipDirection::Both => {
            return Err(CypherError::unsupported(
                "bidirectional arrow pattern `<-[]->`",
                Some(span),
            ))
        }
        _ => Dir::Either,
    };
    Ok(RelPat {
        var: r.variable.as_ref().map(|v| ctx.name(v)),
        types,
        dir,
        props: match &r.properties {
            Some(p) => Some(super::adapter_expr::convert_expr(ctx, p)?),
            None => None,
        },
        span,
    })
}

pub(super) fn convert_part(ctx: &Ctx, p: &up::PatternPart) -> CResult<PatternPart> {
    let k = &p.kind;
    if let Some(sel) = &k.selector {
        return Err(CypherError::unsupported(
            "path selector",
            Some(sel.span.into()),
        ));
    }
    let mut nodes = Vec::new();
    let mut rels = Vec::new();
    let mut expect_node = true;
    for f in &k.path.kind.factors {
        let sp: Span = f.span.into();
        match &f.kind {
            up::PathFactorKind::Node(n) if expect_node => {
                nodes.push(convert_node(ctx, n, sp)?);
                expect_node = false;
            }
            up::PathFactorKind::Relationship(r) if !expect_node => {
                rels.push(convert_rel(ctx, r, sp)?);
                expect_node = true;
            }
            up::PathFactorKind::LegacyShortest { all, .. } => {
                return Err(CypherError::unsupported(
                    if *all {
                        "allShortestPaths"
                    } else {
                        "shortestPath"
                    },
                    Some(sp),
                ))
            }
            up::PathFactorKind::QuantifiedNode { .. }
            | up::PathFactorKind::Parenthesized { .. }
            | up::PathFactorKind::Subpath { .. } => {
                return Err(CypherError::unsupported(
                    "quantified path patterns",
                    Some(sp),
                ))
            }
            _ => return Err(CypherError::parse(sp, "malformed path pattern")),
        }
    }
    if nodes.len() != rels.len() + 1 {
        return Err(CypherError::parse(p.span.into(), "malformed path pattern"));
    }
    Ok(PatternPart {
        binding: k.binding.as_ref().map(|b| ctx.name(b)),
        nodes,
        rels,
        span: p.span.into(),
    })
}

pub(super) fn convert_pattern(ctx: &Ctx, p: &up::Pattern) -> CResult<Pattern> {
    p.parts.iter().map(|x| convert_part(ctx, x)).collect()
}

fn convert_projection(ctx: &Ctx, p: &up::ProjectionClause) -> CResult<Projection> {
    let mut star = false;
    let mut items = Vec::new();
    for it in &p.items {
        match &it.kind {
            up::ProjectionItemKind::Wildcard => star = true,
            up::ProjectionItemKind::Expression { expression, alias } => {
                items.push(ProjItem {
                    expr: super::adapter_expr::convert_expr(ctx, expression)?,
                    alias: alias.as_ref().map(|a| ctx.name(a)),
                    text: ctx.text(expression.span.into()),
                    span: it.span.into(),
                });
            }
            _ => {
                return Err(CypherError::unsupported(
                    "projection item",
                    Some(it.span.into()),
                ))
            }
        }
    }
    let mut order = Vec::new();
    if let Some(ob) = &p.order_by {
        for s in &ob.items {
            order.push(SortItem {
                expr: super::adapter_expr::convert_expr(ctx, &s.expression)?,
                desc: matches!(
                    s.direction.as_ref().map(|d| d.kind),
                    Some(up::SortDirection::Descending)
                ),
            });
        }
    }
    let opt = |e: &Option<up::Expr>| -> CResult<Option<Expr>> {
        match e {
            Some(e) => Ok(Some(super::adapter_expr::convert_expr(ctx, e)?)),
            None => Ok(None),
        }
    };
    Ok(Projection {
        distinct: matches!(
            p.quantifier.as_ref().map(|q| q.kind),
            Some(up::SetQuantifier::Distinct)
        ),
        star,
        items,
        order,
        skip: match opt(&p.skip)? {
            Some(s) => Some(s),
            None => opt(&p.offset)?,
        },
        limit: opt(&p.limit)?,
        span: p.span.into(),
    })
}

fn convert_set(ctx: &Ctx, s: &up::SetClause) -> CResult<Vec<SetItem>> {
    let mut out = Vec::new();
    for it in &s.items {
        let span: Span = it.span.into();
        out.push(match &it.kind {
            up::SetItemKind::Property { target, value } => {
                let t = super::adapter_expr::convert_expr(ctx, target)?;
                if !matches!(t.kind, ExprKind::Prop(..)) {
                    return Err(CypherError::unsupported(
                        "dynamic property assignment",
                        Some(span),
                    ));
                }
                SetItem::Prop {
                    target: t,
                    value: super::adapter_expr::convert_expr(ctx, value)?,
                    span,
                }
            }
            up::SetItemKind::ReplaceProperties { target, value } => SetItem::Replace {
                target: ctx.name(target),
                value: super::adapter_expr::convert_expr(ctx, value)?,
                span,
            },
            up::SetItemKind::MergeProperties { target, value } => SetItem::Merge {
                target: ctx.name(target),
                value: super::adapter_expr::convert_expr(ctx, value)?,
                span,
            },
            up::SetItemKind::Labels { target, labels } => SetItem::Labels {
                target: ctx.name(target),
                labels: plain_labels(ctx, labels)?,
                span,
            },
            _ => return Err(CypherError::unsupported("SET item", Some(span))),
        });
    }
    Ok(out)
}

fn convert_clause(ctx: &Ctx, c: &up::Clause) -> CResult<Clause> {
    use super::adapter_expr::convert_expr;
    let span: Span = c.span.into();
    Ok(match &c.kind {
        ClauseKind::Match(m) => {
            if let Some(mode) = &m.mode {
                return Err(CypherError::unsupported(
                    "GQL path modes (WALK, TRAIL, SIMPLE, ACYCLIC)",
                    Some(mode.span.into()),
                ));
            }
            let mode = ctx
                .pre
                .matches
                .iter()
                .find(|e| e.clause_start == c.span.start)
                .map_or(MatchModeExt::Default, |e| e.mode);
            Clause::Match {
                optional: m.optional,
                mode,
                pattern: convert_pattern(ctx, &m.pattern)?,
                where_: match &m.where_clause {
                    Some(w) => Some(convert_expr(ctx, w)?),
                    None => None,
                },
                span,
            }
        }
        ClauseKind::Unwind(u) => Clause::Unwind {
            expr: convert_expr(ctx, &u.expression)?,
            var: ctx.name(&u.variable),
        },
        ClauseKind::Create(cr) => Clause::Create {
            pattern: convert_pattern(ctx, &cr.pattern)?,
            span,
        },
        ClauseKind::Merge(m) => {
            let mut on_create = Vec::new();
            let mut on_match = Vec::new();
            for a in &m.actions {
                let items = convert_set(ctx, &a.set)?;
                match a.trigger.kind {
                    up::MergeTrigger::Create => on_create.extend(items),
                    _ => on_match.extend(items),
                }
            }
            Clause::Merge {
                part: convert_part(ctx, &m.pattern)?,
                on_create,
                on_match,
                span,
            }
        }
        ClauseKind::Set(s) => Clause::Set(convert_set(ctx, s)?),
        ClauseKind::Remove(r) => {
            let mut items = Vec::new();
            for it in &r.items {
                items.push(match &it.kind {
                    up::RemoveItemKind::Property(e) => RemoveItem::Prop(convert_expr(ctx, e)?),
                    up::RemoveItemKind::Labels { target, labels } => RemoveItem::Labels {
                        target: ctx.name(target),
                        labels: plain_labels(ctx, labels)?,
                    },
                    _ => {
                        return Err(CypherError::unsupported(
                            "REMOVE item",
                            Some(it.span.into()),
                        ))
                    }
                });
            }
            Clause::Remove(items)
        }
        ClauseKind::Delete(d) => Clause::Delete {
            detach: matches!(d.mode.kind, up::DeleteMode::Detach),
            exprs: d
                .expressions
                .iter()
                .map(|e| convert_expr(ctx, e))
                .collect::<CResult<_>>()?,
            span,
        },
        ClauseKind::With(w) => Clause::With {
            proj: convert_projection(ctx, &w.projection)?,
            where_: match &w.where_clause {
                Some(e) => Some(convert_expr(ctx, e)?),
                None => None,
            },
        },
        ClauseKind::Return(r) => Clause::Return(convert_projection(ctx, &r.projection)?),
        ClauseKind::Foreach(_) => return Err(CypherError::unsupported("FOREACH", Some(span))),
        ClauseKind::LoadCsv(_) => return Err(CypherError::unsupported("LOAD CSV", Some(span))),
        ClauseKind::Call(call) => convert_call(ctx, call, span)?,
        ClauseKind::Use(_) => {
            return Err(CypherError::parse(
                span,
                "USE is only allowed at the start of a query or subquery",
            ))
        }
        _ => return Err(CypherError::unsupported("this clause", Some(span))),
    })
}

fn convert_call(ctx: &Ctx, call: &up::CallClause, span: Span) -> CResult<Clause> {
    match &call.target {
        up::CallTarget::Subquery { .. } => Err(CypherError::unsupported(
            "CALL { } in this position",
            Some(span),
        )),
        up::CallTarget::Procedure { name, arguments } => {
            if let Some(idx) = name
                .parts
                .first()
                .and_then(|p| p.kind.text.strip_prefix("__tm_sq"))
                .and_then(|n| n.parse::<usize>().ok())
            {
                let sub = &ctx.subs[idx];
                return Ok(Clause::Subquery {
                    body: Box::new(sub.query.clone()),
                    imports: sub.imports.clone(),
                    span,
                });
            }
            let name_s = name
                .parts
                .iter()
                .map(|p| p.kind.text.clone())
                .collect::<Vec<_>>()
                .join(".");
            let mut args = Vec::new();
            for a in arguments.iter().flatten() {
                args.push(super::adapter_expr::convert_expr(ctx, a)?);
            }
            let mut yields = Vec::new();
            if let Some(y) = &call.yield_clause {
                if y.where_clause.is_some() {
                    return Err(CypherError::unsupported(
                        "YIELD … WHERE",
                        Some(y.span.into()),
                    ));
                }
                for it in &y.items {
                    if let up::ProjectionItemKind::Expression { expression, alias } = &it.kind {
                        if let up::ExprKind::Variable(n) = &expression.kind {
                            yields.push((ctx.name(n), alias.as_ref().map(|a| ctx.name(a))));
                            continue;
                        }
                    }
                    return Err(CypherError::unsupported("YIELD item", Some(it.span.into())));
                }
            }
            Ok(Clause::Procedure {
                name: name_s,
                args,
                yields,
                span,
            })
        }
        _ => Err(CypherError::unsupported("CALL form", Some(span))),
    }
}
