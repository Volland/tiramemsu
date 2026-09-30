//! `CALL { … }` subqueries, `FOREACH` and friends are not part of the upstream
//! grammar. The pre-pass cuts every outermost `CALL { … }` out of the text and
//! parses the body recursively over the same byte offsets (everything outside the
//! body is masked with spaces). In the text seen by the upstream parser the
//! subquery is replaced by a placeholder procedure call `CALL __tm_sqN()` of the
//! same byte length, so the clause keeps its position and its span.

use open_cypher::{lex, Keyword, Token, TokenKind};

use super::adapter::{parse_range, Sub};
use crate::ast::Name;
use crate::error::{CResult, CypherError};
use crate::span::Span;

/// `text` with everything outside `lo..hi` replaced by spaces.
pub(super) fn mask(text: &str, lo: usize, hi: usize) -> String {
    if lo == 0 && hi == text.len() {
        return text.to_string();
    }
    let mut b = text.as_bytes().to_vec();
    for (i, c) in b.iter_mut().enumerate() {
        if i < lo || i >= hi {
            *c = b' ';
        }
    }
    String::from_utf8_lossy(&b).into_owned()
}

fn blank_range(b: &mut [u8], lo: usize, hi: usize) {
    for c in &mut b[lo..hi] {
        *c = b' ';
    }
}

/// Cuts the outermost `CALL { }` subqueries out of `masked`.
pub(super) fn extract(orig: &str, masked: &str) -> CResult<(String, Vec<Sub>)> {
    let toks: Vec<Token> = lex(masked)
        .tokens
        .into_iter()
        .filter(|t| !t.kind.is_trivia())
        .collect();
    let mut out = masked.as_bytes().to_vec();
    let mut subs: Vec<Sub> = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        if toks[i].kind != TokenKind::Keyword(Keyword::Call) {
            i += 1;
            continue;
        }
        let call = toks[i];
        let mut j = i + 1;
        let mut scope: Option<Vec<Name>> = None;
        if toks.get(j).map(|t| t.kind) == Some(TokenKind::LeftParen) {
            // `CALL (a, b) {` or a procedure call `CALL p(…)`? Only when a `{` follows the `)`.
            let mut k = j + 1;
            let mut names = Vec::new();
            let mut simple = true;
            while k < toks.len() && toks[k].kind != TokenKind::RightParen {
                match toks[k].kind {
                    TokenKind::Identifier | TokenKind::EscapedIdentifier => {
                        let t = toks[k];
                        let raw = t.text(orig).unwrap_or("");
                        let esc = t.kind == TokenKind::EscapedIdentifier;
                        names.push(Name {
                            text: if esc {
                                raw.trim_matches('`').to_string()
                            } else {
                                raw.to_string()
                            },
                            escaped: esc,
                            span: t.span.into(),
                        });
                    }
                    TokenKind::Comma => {}
                    _ => simple = false,
                }
                k += 1;
            }
            if simple && toks.get(k + 1).map(|t| t.kind) == Some(TokenKind::LeftBrace) {
                scope = Some(names);
                j = k + 1;
            } else {
                i += 1;
                continue;
            }
        }
        if toks.get(j).map(|t| t.kind) != Some(TokenKind::LeftBrace) {
            i += 1;
            continue;
        }
        // matching brace
        let mut depth = 0i32;
        let mut k = j;
        let mut close = None;
        while k < toks.len() {
            match toks[k].kind {
                TokenKind::LeftBrace => depth += 1,
                TokenKind::RightBrace => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(k);
                        break;
                    }
                }
                _ => {}
            }
            k += 1;
        }
        let Some(close) = close else {
            return Err(CypherError::parse(
                toks[j].span.into(),
                "unclosed delimiter `{`",
            ));
        };
        if let (Some(a), Some(b)) = (toks.get(close + 1), toks.get(close + 2)) {
            if a.kind == TokenKind::Keyword(Keyword::In)
                && b.text(masked)
                    .is_some_and(|t| t.eq_ignore_ascii_case("transactions"))
            {
                return Err(CypherError::unsupported(
                    "CALL { } IN TRANSACTIONS",
                    Some(Span::from(a.span).cover(b.span.into())),
                ));
            }
        }
        let (lo, hi) = (toks[j].span.end, toks[close].span.start);
        let body = parse_range(orig, lo, hi)?;
        let mut imports = scope.unwrap_or_default();
        let mut body = body;
        if imports.is_empty() {
            imports = take_import_with(&mut body);
        }
        let idx = subs.len();
        let region = (call.span.start, toks[close].span.end);
        let ph = format!("CALL __tm_sq{idx}()");
        if ph.len() > region.1 - region.0 {
            return Err(CypherError::parse(
                Span::new(region.0, region.1),
                "subquery too short",
            ));
        }
        blank_range(&mut out, region.0, region.1);
        out[region.0..region.0 + ph.len()].copy_from_slice(ph.as_bytes());
        subs.push(Sub {
            query: body,
            imports,
        });
        i = close + 1;
    }
    let main = String::from_utf8(out)
        .map_err(|_| CypherError::parse(Span::new(0, 0), "subquery extraction broke UTF-8"))?;
    Ok((main, subs))
}

/// Removes an importing `WITH v1, v2` from the front of a body and returns its names.
fn take_import_with(body: &mut crate::ast::Query) -> Vec<Name> {
    use crate::ast::{Clause, ExprKind};
    let Some(first) = body.parts.first_mut() else {
        return Vec::new();
    };
    if let Some(Clause::With { proj, where_ }) = first.clauses.first() {
        let simple = where_.is_none()
            && !proj.distinct
            && !proj.star
            && proj.order.is_empty()
            && proj.skip.is_none()
            && proj.limit.is_none()
            && !proj.items.is_empty()
            && proj
                .items
                .iter()
                .all(|it| it.alias.is_none() && matches!(&it.expr.kind, ExprKind::Var(_)));
        if simple {
            let names = proj
                .items
                .iter()
                .map(|it| Name {
                    text: it.text.trim_matches('`').to_string(),
                    escaped: false,
                    span: it.expr.span,
                })
                .collect();
            first.clauses.remove(0);
            return names;
        }
    }
    Vec::new()
}
