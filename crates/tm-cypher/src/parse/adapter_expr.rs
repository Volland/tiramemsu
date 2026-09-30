//! Expression conversion (upstream AST to [`crate::ast::Expr`]).

use open_cypher::ast as up;

use super::adapter::{convert_pattern, convert_query, Ctx};
use crate::ast::*;
use crate::error::{CResult, CypherError};
use crate::span::Span;

fn int_lit(text: &str, radix: &up::IntegerRadix, neg: bool, span: Span) -> CResult<i64> {
    let t = text.replace('_', "");
    let (digits, base) = match radix {
        up::IntegerRadix::Hexadecimal => (t[2..].to_string(), 16),
        up::IntegerRadix::Octal => {
            if t.starts_with("0o") || t.starts_with("0O") {
                (t[2..].to_string(), 8)
            } else {
                (t.clone(), 8)
            }
        }
        _ => (t.clone(), 10),
    };
    let v = i128::from_str_radix(&digits, base)
        .map_err(|_| CypherError::parse(span, "invalid integer literal"))?;
    let v = if neg { -v } else { v };
    i64::try_from(v).map_err(|_| CypherError::parse(span, "integer literal is too large"))
}

fn ex(kind: ExprKind, span: open_cypher::Span) -> Expr {
    Expr {
        kind,
        span: span.into(),
    }
}

fn boxed(ctx: &Ctx, e: &up::Expr) -> CResult<Box<Expr>> {
    Ok(Box::new(convert_expr(ctx, e)?))
}

fn bin(op: up::BinaryOperator) -> Option<BinOp> {
    use up::BinaryOperator as B;
    Some(match op {
        B::Or => BinOp::Or,
        B::Xor => BinOp::Xor,
        B::And => BinOp::And,
        B::Equal => BinOp::Eq,
        B::NotEqual => BinOp::Ne,
        B::Less => BinOp::Lt,
        B::LessEqual => BinOp::Le,
        B::Greater => BinOp::Gt,
        B::GreaterEqual => BinOp::Ge,
        B::RegexMatch => BinOp::Regex,
        B::In => BinOp::In,
        B::StartsWith => BinOp::StartsWith,
        B::EndsWith => BinOp::EndsWith,
        B::Contains => BinOp::Contains,
        B::Add => BinOp::Add,
        B::Subtract => BinOp::Sub,
        B::Multiply => BinOp::Mul,
        B::Divide => BinOp::Div,
        B::Remainder => BinOp::Mod,
        B::Power => BinOp::Pow,
        _ => return None,
    })
}

/// Converts an upstream expression.
pub(super) fn convert_expr(ctx: &Ctx, e: &up::Expr) -> CResult<Expr> {
    use up::ExprKind as K;
    let sp: Span = e.span.into();
    Ok(match &e.kind {
        K::Literal(l) => ex(
            ExprKind::Lit(match &l.kind {
                up::LiteralKind::Null => Lit::Null,
                up::LiteralKind::Boolean(b) => Lit::Bool(*b),
                up::LiteralKind::Integer(i) => Lit::Int(int_lit(&i.text, &i.radix, false, sp)?),
                up::LiteralKind::Float(f) => {
                    let t = f.text.replace('_', "");
                    let t = t.trim_end_matches(['f', 'F', 'd', 'D']);
                    let x: f64 = t
                        .parse()
                        .map_err(|_| CypherError::parse(sp, "invalid float"))?;
                    if x.is_infinite() {
                        return Err(CypherError::parse(sp, "floating point overflow"));
                    }
                    Lit::Float(x)
                }
                up::LiteralKind::String(s) => Lit::Str(s.value.clone()),
                _ => return Err(CypherError::unsupported("literal", Some(sp))),
            }),
            e.span,
        ),
        K::Variable(n) => ex(ExprKind::Var(n.kind.text.clone()), e.span),
        K::Parameter(p) => ex(
            ExprKind::Param(match &p.name {
                up::ParameterName::Named(n) | up::ParameterName::Positional(n) => n.clone(),
                _ => String::new(),
            }),
            e.span,
        ),
        K::Wildcard => ex(ExprKind::CountStar, e.span),
        K::List(xs) => ex(
            ExprKind::List(
                xs.iter()
                    .map(|x| convert_expr(ctx, x))
                    .collect::<CResult<_>>()?,
            ),
            e.span,
        ),
        K::Map(es) => {
            let mut out = Vec::new();
            for m in es {
                out.push((ctx.name(&m.key), convert_expr(ctx, &m.value)?));
            }
            ex(ExprKind::Map(out), e.span)
        }
        K::MapProjection(mp) => {
            let mut items = Vec::new();
            for it in &mp.items {
                items.push(match &it.kind {
                    up::MapProjectionItemKind::Property(n) => MapProjItem::Prop(ctx.name(n)),
                    up::MapProjectionItemKind::AllProperties => MapProjItem::All,
                    up::MapProjectionItemKind::Variable(n) => MapProjItem::Var(ctx.name(n)),
                    up::MapProjectionItemKind::Entry(m) => {
                        MapProjItem::Entry(ctx.name(&m.key), convert_expr(ctx, &m.value)?)
                    }
                    _ => return Err(CypherError::unsupported("map projection item", Some(sp))),
                });
            }
            ex(ExprKind::MapProj(boxed(ctx, &mp.base)?, items), e.span)
        }
        K::Unary { operator, operand } => match operator.kind {
            up::UnaryOperator::Not => ex(ExprKind::Unary(UnOp::Not, boxed(ctx, operand)?), e.span),
            up::UnaryOperator::Plus => {
                ex(ExprKind::Unary(UnOp::Plus, boxed(ctx, operand)?), e.span)
            }
            _ => {
                if let K::Literal(l) = &operand.kind {
                    if let up::LiteralKind::Integer(i) = &l.kind {
                        return Ok(ex(
                            ExprKind::Lit(Lit::Int(int_lit(&i.text, &i.radix, true, sp)?)),
                            e.span,
                        ));
                    }
                }
                ex(ExprKind::Unary(UnOp::Neg, boxed(ctx, operand)?), e.span)
            }
        },
        K::Binary {
            left,
            operator,
            right,
        } => {
            let Some(op) = bin(operator.kind) else {
                return Err(CypherError::unsupported(
                    "the `||` concatenation operator",
                    Some(operator.span.into()),
                ));
            };
            let is_cmp = |o: BinOp| {
                matches!(
                    o,
                    BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge
                )
            };
            // a < b < c is (a < b) AND (b < c)
            if is_cmp(op) {
                if let K::Binary {
                    right: lr,
                    operator: lop,
                    ..
                } = &left.kind
                {
                    if bin(lop.kind).is_some_and(is_cmp) {
                        let l = convert_expr(ctx, left)?;
                        let mid = convert_expr(ctx, lr)?;
                        let r = convert_expr(ctx, right)?;
                        let cmp2 = Expr {
                            kind: ExprKind::Binary(op, Box::new(mid), Box::new(r)),
                            span: e.span.into(),
                        };
                        return Ok(ex(
                            ExprKind::Binary(BinOp::And, Box::new(l), Box::new(cmp2)),
                            e.span,
                        ));
                    }
                }
            }
            ex(
                ExprKind::Binary(op, boxed(ctx, left)?, boxed(ctx, right)?),
                e.span,
            )
        }
        K::Is {
            expression,
            negated,
            predicate,
        } => match predicate {
            up::IsPredicate::Null => {
                ex(ExprKind::IsNull(boxed(ctx, expression)?, *negated), e.span)
            }
            up::IsPredicate::Label(le) => {
                let names = label_names(ctx, le)?;
                let inner = ex(ExprKind::HasLabels(boxed(ctx, expression)?, names), e.span);
                if *negated {
                    ex(ExprKind::Unary(UnOp::Not, Box::new(inner)), e.span)
                } else {
                    inner
                }
            }
            _ => return Err(CypherError::unsupported("IS :: type predicate", Some(sp))),
        },
        K::Property { expression, key } => ex(
            ExprKind::Prop(boxed(ctx, expression)?, ctx.name(key)),
            e.span,
        ),
        K::DynamicProperty { .. } => {
            return Err(CypherError::unsupported(
                "dynamic property access",
                Some(sp),
            ))
        }
        K::Index { expression, index } => ex(
            ExprKind::Index(boxed(ctx, expression)?, boxed(ctx, index)?),
            e.span,
        ),
        K::Slice {
            expression,
            lower,
            upper,
        } => {
            let opt = |x: &Option<Box<up::Expr>>| -> CResult<Option<Box<Expr>>> {
                match x {
                    Some(x) => Ok(Some(boxed(ctx, x)?)),
                    None => Ok(None),
                }
            };
            ex(
                ExprKind::Slice(boxed(ctx, expression)?, opt(lower)?, opt(upper)?),
                e.span,
            )
        }
        K::Function(f) => {
            let name = f
                .name
                .parts
                .iter()
                .map(|p| p.kind.text.clone())
                .collect::<Vec<_>>()
                .join(".");
            let distinct = matches!(
                f.quantifier.as_ref().map(|q| q.kind),
                Some(up::SetQuantifier::Distinct)
            );
            if name.eq_ignore_ascii_case("count")
                && f.arguments.len() == 1
                && matches!(f.arguments[0].kind, K::Wildcard)
            {
                return Ok(ex(ExprKind::CountStar, e.span));
            }
            let args = f
                .arguments
                .iter()
                .map(|a| convert_expr(ctx, a))
                .collect::<CResult<_>>()?;
            ex(
                ExprKind::Call {
                    name,
                    distinct,
                    args,
                },
                e.span,
            )
        }
        K::Case(c) => {
            let mut alts = Vec::new();
            for a in &c.alternatives {
                // a `WHEN a, b` list is a disjunction of equality tests for the simple form
                let when = a
                    .when
                    .first()
                    .ok_or_else(|| CypherError::parse(a.span.into(), "empty WHEN"))?;
                if a.when.len() > 1 {
                    return Err(CypherError::unsupported(
                        "CASE WHEN with several values",
                        Some(a.span.into()),
                    ));
                }
                alts.push((convert_expr(ctx, when)?, convert_expr(ctx, &a.then)?));
            }
            ex(
                ExprKind::Case {
                    operand: match &c.operand {
                        Some(o) => Some(boxed(ctx, o)?),
                        None => None,
                    },
                    alts,
                    els: match &c.else_expression {
                        Some(o) => Some(boxed(ctx, o)?),
                        None => None,
                    },
                },
                e.span,
            )
        }
        K::ListComprehension(l) => {
            let opt = |x: &Option<Box<up::Expr>>| -> CResult<Option<Box<Expr>>> {
                match x {
                    Some(x) => Ok(Some(boxed(ctx, x)?)),
                    None => Ok(None),
                }
            };
            ex(
                ExprKind::ListComp {
                    var: ctx.name(&l.variable),
                    list: boxed(ctx, &l.list)?,
                    pred: opt(&l.predicate)?,
                    proj: opt(&l.projection)?,
                },
                e.span,
            )
        }
        K::PatternComprehension(_) => {
            return Err(CypherError::unsupported("pattern comprehension", Some(sp)))
        }
        K::Reduce(r) => ex(
            ExprKind::Reduce {
                acc: ctx.name(&r.accumulator),
                init: boxed(ctx, &r.initial)?,
                var: ctx.name(&r.variable),
                list: boxed(ctx, &r.list)?,
                body: boxed(ctx, &r.expression)?,
            },
            e.span,
        ),
        K::QuantifiedPredicate(q) => ex(
            ExprKind::Quantifier {
                kind: match q.kind.kind {
                    up::QuantifiedPredicateKind::All => "all",
                    up::QuantifiedPredicateKind::Any => "any",
                    up::QuantifiedPredicateKind::None => "none",
                    _ => "single",
                }
                .to_string(),
                var: ctx.name(&q.variable),
                list: boxed(ctx, &q.list)?,
                pred: boxed(ctx, &q.predicate)?,
            },
            e.span,
        ),
        K::Subquery(s) => match s.kind.kind {
            up::SubqueryExpressionKind::Exists => {
                let q = convert_query(ctx, &s.query)?;
                ex(ExprKind::Exists(Box::new(ExistsBody::Query(q))), e.span)
            }
            up::SubqueryExpressionKind::Count => {
                return Err(CypherError::unsupported("COUNT { } subquery", Some(sp)))
            }
            _ => return Err(CypherError::unsupported("COLLECT { } subquery", Some(sp))),
        },
        K::Pattern(p) => ex(
            ExprKind::Exists(Box::new(ExistsBody::Pattern(
                convert_pattern(ctx, p)?,
                None,
            ))),
            e.span,
        ),
        K::Parenthesized(i) => convert_expr(ctx, i)?,
        _ => return Err(CypherError::parse(sp, "invalid expression")),
    })
}

fn label_names(ctx: &Ctx, le: &up::LabelExpression) -> CResult<Vec<Name>> {
    use up::LabelExpressionKind as K;
    let sp: Span = le.span.into();
    if ctx.text(sp).contains('&') {
        return Err(CypherError::unsupported(
            "label expression with `&`",
            Some(sp),
        ));
    }
    match &le.kind {
        K::Name(n) => Ok(vec![ctx.name(n)]),
        K::And(xs) => {
            let mut out = Vec::new();
            for x in xs {
                out.extend(label_names(ctx, x)?);
            }
            Ok(out)
        }
        K::Parenthesized(i) => label_names(ctx, i),
        _ => Err(CypherError::unsupported("label expression", Some(sp))),
    }
}
