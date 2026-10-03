//! The extension pre-pass: recognises `USE AS OF | HISTORY | VALID AT` at scope
//! starts, and `REPEATABLE ELEMENTS` / `DIFFERENT RELATIONSHIPS` and `TIME
//! RESPECTING [AFTER t] [ARRIVAL AS name]` after `MATCH`, records them, and blanks
//! them with spaces so that every span of the upstream parser still refers to the
//! original text (design Decision 2).

use open_cypher::{lex, Keyword, Token, TokenKind};

use crate::ast::{MatchModeExt, Name, TemporalMatch, TimeArg, TimeSel, TxClause};
use crate::error::{CResult, CypherError};
use crate::span::Span;

/// A recognised `USE` clause.
#[derive(Clone, Debug, PartialEq)]
pub struct ScopeExt {
    /// Byte where the `USE` keyword starts.
    pub at_byte: usize,
    /// Start of the first token after the clause (the next clause's start).
    pub next_tok: usize,
    /// The selectors.
    pub time: TimeSel,
}

/// A recognised match mode keyword pair.
#[derive(Clone, Debug, PartialEq)]
pub struct MatchExt {
    /// Start of the clause (`OPTIONAL` or `MATCH`).
    pub clause_start: usize,
    /// The mode.
    pub mode: MatchModeExt,
}

/// A recognised `TIME RESPECTING …` after `MATCH`.
#[derive(Clone, Debug, PartialEq)]
pub struct TemporalExt {
    /// Start of the clause (`OPTIONAL` or `MATCH`).
    pub clause_start: usize,
    /// The modifier.
    pub temporal: TemporalMatch,
}

/// The output of the pre-pass.
#[derive(Debug)]
pub struct Prepass {
    /// The text with every extension replaced by spaces of the same byte length.
    pub blanked: String,
    /// Time selectors.
    pub scopes: Vec<ScopeExt>,
    /// Match modes.
    pub matches: Vec<MatchExt>,
    /// Time-respecting modifiers.
    pub temporals: Vec<TemporalExt>,
}

struct Cur<'a> {
    src: &'a str,
    toks: Vec<Token>,
    i: usize,
}

impl<'a> Cur<'a> {
    fn peek(&self, n: usize) -> Option<Token> {
        self.toks.get(self.i + n).copied()
    }
    fn text(&self, t: Token) -> &'a str {
        t.text(self.src).unwrap_or("")
    }
    fn is_word(&self, t: Option<Token>, w: &str) -> bool {
        match t {
            Some(t) => {
                matches!(t.kind, TokenKind::Identifier | TokenKind::Keyword(_))
                    && self.text(t).eq_ignore_ascii_case(w)
            }
            None => false,
        }
    }
}

fn unquote(s: &str) -> String {
    let inner = &s[1..s.len().saturating_sub(1).max(1)];
    let mut out = String::new();
    let mut it = inner.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            if let Some(n) = it.next() {
                out.push(n);
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn time_arg(c: &mut Cur, what: &str) -> CResult<(TimeArg, Span)> {
    let Some(t) = c.peek(0) else {
        return Err(CypherError::parse(
            Span::new(c.src.len(), c.src.len()),
            format!("{what} needs an argument"),
        ));
    };
    let sp: Span = t.span.into();
    // a negative integer: `-` then the digits
    if t.kind == TokenKind::Minus && c.peek(1).map(|x| x.kind) == Some(TokenKind::Integer) {
        let n = c.peek(1).unwrap();
        let txt = c.text(n).replace('_', "");
        let end: Span = n.span.into();
        let v: i64 = format!("-{txt}")
            .parse()
            .map_err(|_| CypherError::parse(sp.cover(end), "integer out of range"))?;
        c.i += 2;
        return Ok((TimeArg::Int(v), sp.cover(end)));
    }
    match t.kind {
        TokenKind::Integer => {
            c.i += 1;
            let txt = c.text(t).replace('_', "");
            let v: i64 = txt
                .parse()
                .map_err(|_| CypherError::parse(sp, "integer out of range"))?;
            Ok((TimeArg::Int(v), sp))
        }
        TokenKind::Float => Err(CypherError::parse(
            sp,
            format!("{what} needs an integer, not a float"),
        )),
        TokenKind::Parameter => {
            c.i += 1;
            Ok((TimeArg::Param(c.text(t)[1..].to_string()), sp))
        }
        TokenKind::Identifier => {
            let name = c.text(t).to_ascii_lowercase();
            let is_dt = name == "datetime";
            if (is_dt || name == "date")
                && c.peek(1).map(|x| x.kind) == Some(TokenKind::LeftParen)
                && c.peek(2).map(|x| x.kind) == Some(TokenKind::String)
                && c.peek(3).map(|x| x.kind) == Some(TokenKind::RightParen)
            {
                let s = unquote(c.text(c.peek(2).unwrap()));
                let end: Span = c.peek(3).unwrap().span.into();
                c.i += 4;
                let arg = if is_dt {
                    TimeArg::DateTime(s)
                } else {
                    TimeArg::Date(s)
                };
                return Ok((arg, sp.cover(end)));
            }
            Err(CypherError::parse(
                sp,
                format!("{what} takes an integer, a parameter, datetime('…') or date('…')"),
            ))
        }
        _ => Err(CypherError::parse(
            sp,
            format!("{what} takes an integer, a parameter, datetime('…') or date('…')"),
        )),
    }
}

/// Parses a `USE …` clause at the cursor (on the `USE` token).
fn use_clause(c: &mut Cur) -> CResult<TimeSel> {
    let start: Span = c.peek(0).unwrap().span.into();
    c.i += 1;
    let mut tx: Option<TxClause> = None;
    let mut valid = None;
    let mut end = start;
    loop {
        let t = c.peek(0);
        if matches!(t, Some(t) if t.kind == TokenKind::Keyword(Keyword::As))
            && c.is_word(c.peek(1), "of")
        {
            let sp: Span = t.unwrap().span.into();
            if tx.is_some() {
                return Err(CypherError::parse(
                    sp,
                    "two transaction-time selectors in USE",
                ));
            }
            c.i += 2;
            let (a, asp) = time_arg(c, "AS OF")?;
            end = asp;
            tx = Some(TxClause::AsOf(a, asp));
        } else if c.is_word(t, "history") {
            let sp: Span = t.unwrap().span.into();
            if tx.is_some() {
                return Err(CypherError::parse(
                    sp,
                    "two transaction-time selectors in USE",
                ));
            }
            c.i += 1;
            end = sp;
            tx = Some(TxClause::History);
        } else if c.is_word(t, "valid") && c.is_word(c.peek(1), "at") {
            let sp: Span = t.unwrap().span.into();
            if valid.is_some() {
                return Err(CypherError::parse(sp, "two VALID AT selectors in USE"));
            }
            c.i += 2;
            let (a, asp) = time_arg(c, "VALID AT")?;
            end = asp;
            valid = Some((a, asp));
        } else {
            break;
        }
    }
    if tx.is_none() && valid.is_none() {
        let sp = t_span(c.peek(0), start);
        return Err(CypherError::unsupported("USE GRAPH", Some(start.cover(sp))));
    }
    Ok(TimeSel {
        tx,
        valid,
        span: start.cover(end),
    })
}

/// Parses `TIME RESPECTING [AFTER t] [ARRIVAL AS name]` at the cursor (on
/// `TIME`); the cursor ends after the modifier.
// @lat: [[query#Temporal Path Syntax#Cypher Temporal Paths]]
fn temporal_match(c: &mut Cur) -> CResult<TemporalMatch> {
    let start: Span = c.peek(0).unwrap().span.into();
    let mut end: Span = c.peek(1).unwrap().span.into();
    c.i += 2;
    let mut after = None;
    if c.is_word(c.peek(0), "after") {
        c.i += 1;
        let (a, sp) = time_arg(c, "TIME RESPECTING AFTER")?;
        end = sp;
        after = Some((a, sp));
    }
    let mut arrival = None;
    if c.is_word(c.peek(0), "arrival") {
        let kw: Span = c.peek(0).unwrap().span.into();
        let as_ok = matches!(c.peek(1), Some(t) if t.kind == TokenKind::Keyword(Keyword::As));
        let name = c
            .peek(2)
            .filter(|t| matches!(t.kind, TokenKind::Identifier | TokenKind::EscapedIdentifier));
        let (true, Some(n)) = (as_ok, name) else {
            return Err(CypherError::parse(kw, "ARRIVAL needs `AS <variable>`"));
        };
        let raw = c.text(n);
        let escaped = n.kind == TokenKind::EscapedIdentifier;
        let text = if escaped {
            raw.trim_matches('`').to_string()
        } else {
            raw.to_string()
        };
        end = n.span.into();
        c.i += 3;
        arrival = Some(Name {
            text,
            escaped,
            span: end,
        });
    }
    Ok(TemporalMatch {
        after,
        arrival,
        span: start.cover(end),
    })
}

fn t_span(t: Option<Token>, d: Span) -> Span {
    t.map_or(d, |t| t.span.into())
}

/// Runs the pre-pass over `text`.
pub fn run(text: &str) -> CResult<Prepass> {
    run_in(text, false)
}

/// Like [`run`]; `body` is true for the text of a `CALL { }` body, where an
/// importing `WITH v1, v2` at the start may precede the `USE`.
pub fn run_in(text: &str, body: bool) -> CResult<Prepass> {
    let lexed = lex(text);
    let toks: Vec<Token> = lexed
        .tokens
        .into_iter()
        .filter(|t| !t.kind.is_trivia())
        .collect();
    let mut c = Cur {
        src: text,
        toks,
        i: 0,
    };
    let mut scopes = Vec::new();
    let mut matches = Vec::new();
    let mut temporals = Vec::new();
    let mut blank: Vec<(usize, usize)> = Vec::new();
    // Positions (token indexes) where a scope starts.
    let mut starts = vec![0usize];
    let n = c.toks.len();
    if body
        && matches!(
            c.toks.first().map(|x| x.kind),
            Some(TokenKind::Keyword(Keyword::With))
        )
    {
        let mut k = 1;
        while matches!(
            c.toks.get(k).map(|x| x.kind),
            Some(TokenKind::Identifier | TokenKind::EscapedIdentifier)
        ) {
            k += 1;
            if c.toks.get(k).map(|x| x.kind) == Some(TokenKind::Comma) {
                k += 1;
            } else {
                break;
            }
        }
        if k > 1 {
            starts.push(k);
        }
    }
    for i in 0..n {
        let t = c.toks[i];
        match t.kind {
            TokenKind::Keyword(Keyword::Union) => {
                let mut j = i + 1;
                if let Some(x) = c.toks.get(j) {
                    if matches!(x.kind, TokenKind::Keyword(Keyword::All | Keyword::Distinct)) {
                        j += 1;
                    }
                }
                starts.push(j);
            }
            TokenKind::Keyword(Keyword::Call) => {
                let mut j = i + 1;
                if c.toks.get(j).map(|x| x.kind) == Some(TokenKind::LeftParen) {
                    while j < n && c.toks[j].kind != TokenKind::RightParen {
                        j += 1;
                    }
                    j += 1;
                }
                if c.toks.get(j).map(|x| x.kind) == Some(TokenKind::LeftBrace) {
                    j += 1;
                    starts.push(j);
                    // importing WITH v1, v2
                    if matches!(
                        c.toks.get(j).map(|x| x.kind),
                        Some(TokenKind::Keyword(Keyword::With))
                    ) {
                        let mut k = j + 1;
                        loop {
                            let ok = matches!(
                                c.toks.get(k).map(|x| x.kind),
                                Some(TokenKind::Identifier | TokenKind::EscapedIdentifier)
                            );
                            if !ok {
                                break;
                            }
                            k += 1;
                            if c.toks.get(k).map(|x| x.kind) == Some(TokenKind::Comma) {
                                k += 1;
                            } else {
                                break;
                            }
                        }
                        if k > j + 1 {
                            starts.push(k);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    starts.sort_unstable();
    starts.dedup();
    for s in starts {
        if s < n && c.is_word(Some(c.toks[s]), "use") {
            c.i = s;
            let at = c.toks[s].span.start;
            let time = use_clause(&mut c)?;
            let next_tok = c.peek(0).map_or(text.len(), |t| t.span.start);
            blank.push((at, time.span.end));
            scopes.push(ScopeExt {
                at_byte: at,
                next_tok,
                time,
            });
        }
    }
    // Match modes.
    for i in 0..n {
        if c.toks[i].kind != TokenKind::Keyword(Keyword::Match) {
            continue;
        }
        let clause_start = if i > 0 && c.toks[i - 1].kind == TokenKind::Keyword(Keyword::Optional) {
            c.toks[i - 1].span.start
        } else {
            c.toks[i].span.start
        };
        let a = c.toks.get(i + 1).copied();
        let b = c.toks.get(i + 2).copied();
        let mode = if c.is_word(a, "repeatable") && c.is_word(b, "elements") {
            Some(MatchModeExt::Repeatable)
        } else if c.is_word(a, "different") && c.is_word(b, "relationships") {
            Some(MatchModeExt::Default)
        } else {
            None
        };
        let mut next = i + 1;
        if let Some(mode) = mode {
            blank.push((a.unwrap().span.start, b.unwrap().span.end));
            matches.push(MatchExt { clause_start, mode });
            next = i + 3;
        }
        if c.is_word(c.toks.get(next).copied(), "time")
            && c.is_word(c.toks.get(next + 1).copied(), "respecting")
        {
            c.i = next;
            let temporal = temporal_match(&mut c)?;
            blank.push((temporal.span.start, temporal.span.end));
            temporals.push(TemporalExt {
                clause_start,
                temporal,
            });
        }
    }
    let mut bytes = text.as_bytes().to_vec();
    for (s, e) in blank {
        for b in &mut bytes[s..e] {
            // keep newlines and multi-byte characters' byte count; spaces are ASCII
            *b = b' ';
        }
    }
    let blanked = String::from_utf8(bytes)
        .map_err(|_| CypherError::parse(Span::new(0, 0), "extension blanking broke UTF-8 text"))?;
    Ok(Prepass {
        blanked,
        scopes,
        matches,
        temporals,
    })
}
