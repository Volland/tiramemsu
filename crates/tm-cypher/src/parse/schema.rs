//! Vector index commands (`lat.md/query#Vector Recall#Writes From Query
//! Languages`): `CREATE VECTOR INDEX`, `DROP [VECTOR] INDEX` and `SHOW [VECTOR]
//! INDEX[ES]`. The upstream parser has no schema commands, so they are recognised
//! on the token stream before it runs; a recognised command is the whole query.

use std::collections::BTreeMap;

use open_cypher::{lex, Token, TokenKind};

use crate::ast::Name;
use crate::error::{CResult, CypherError};
use crate::span::Span;
use crate::value::CypherValue;

/// The `OPTIONS` of `CREATE VECTOR INDEX`.
#[derive(Clone, Debug, PartialEq)]
pub enum IndexOptions {
    /// A literal map.
    Map(BTreeMap<String, CypherValue>),
    /// A `$parameter` holding the map.
    Param(String),
}

/// A vector index command.
#[derive(Clone, Debug, PartialEq)]
pub enum SchemaCommand {
    /// `CREATE VECTOR INDEX name [IF NOT EXISTS] OPTIONS …`.
    Create {
        /// The index (model) name.
        name: Name,
        /// `IF NOT EXISTS`.
        if_not_exists: bool,
        /// The options.
        options: IndexOptions,
    },
    /// `DROP [VECTOR] INDEX name [IF EXISTS]`.
    Drop {
        /// The index (model) name.
        name: Name,
        /// `IF EXISTS`.
        if_exists: bool,
    },
    /// `SHOW [VECTOR] INDEX[ES]`.
    Show,
}

impl SchemaCommand {
    /// True for create and drop.
    pub fn writes(&self) -> bool {
        !matches!(self, SchemaCommand::Show)
    }
}

struct Cur<'a> {
    src: &'a str,
    toks: Vec<Token>,
    i: usize,
}

impl<'a> Cur<'a> {
    fn peek(&self) -> Option<Token> {
        self.toks.get(self.i).copied()
    }
    fn text(&self, t: Token) -> &'a str {
        t.text(self.src).unwrap_or("")
    }
    fn word(&self, n: usize) -> Option<String> {
        let t = self.toks.get(self.i + n)?;
        matches!(t.kind, TokenKind::Identifier | TokenKind::Keyword(_))
            .then(|| self.text(*t).to_ascii_lowercase())
    }
    fn eat(&mut self, w: &str) -> bool {
        if self.word(0).as_deref() == Some(w) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn here(&self) -> Span {
        match self.peek() {
            Some(t) => t.span.into(),
            None => Span::new(self.src.len(), self.src.len()),
        }
    }
    fn expect(&mut self, w: &str) -> CResult<()> {
        if self.eat(w) {
            Ok(())
        } else {
            Err(CypherError::parse(
                self.here(),
                format!("expected {}", w.to_ascii_uppercase()),
            ))
        }
    }
    fn expect_kind(&mut self, k: TokenKind, what: &str) -> CResult<Token> {
        match self.peek() {
            Some(t) if t.kind == k => {
                self.i += 1;
                Ok(t)
            }
            _ => Err(CypherError::parse(self.here(), format!("expected {what}"))),
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
                out.push(match n {
                    'n' => '\n',
                    't' => '\t',
                    other => other,
                });
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// An index name: an identifier or a backticked name.
fn name(c: &mut Cur) -> CResult<Name> {
    let Some(t) = c.peek() else {
        return Err(CypherError::parse(c.here(), "expected an index name"));
    };
    let span: Span = t.span.into();
    let (text, escaped) = match t.kind {
        TokenKind::Identifier | TokenKind::Keyword(_) => (c.text(t).to_string(), false),
        TokenKind::EscapedIdentifier => {
            let s = c.text(t);
            (s[1..s.len() - 1].replace("``", "`"), true)
        }
        _ => return Err(CypherError::parse(span, "expected an index name")),
    };
    c.i += 1;
    Ok(Name {
        text,
        escaped,
        span,
    })
}

/// A map key: an identifier, a backticked name or a string.
fn key(c: &mut Cur) -> CResult<String> {
    let Some(t) = c.peek() else {
        return Err(CypherError::parse(c.here(), "expected a map key"));
    };
    let k = match t.kind {
        TokenKind::Identifier | TokenKind::Keyword(_) => c.text(t).to_string(),
        TokenKind::EscapedIdentifier => {
            let s = c.text(t);
            s[1..s.len() - 1].replace("``", "`")
        }
        TokenKind::String => unquote(c.text(t)),
        _ => return Err(CypherError::parse(t.span.into(), "expected a map key")),
    };
    c.i += 1;
    Ok(k)
}

/// A literal: a number, string, boolean, `null`, list or map.
fn literal(c: &mut Cur) -> CResult<CypherValue> {
    let Some(t) = c.peek() else {
        return Err(CypherError::parse(c.here(), "expected a literal"));
    };
    let span: Span = t.span.into();
    let neg = t.kind == TokenKind::Minus;
    if neg {
        c.i += 1;
    }
    let Some(t) = c.peek() else {
        return Err(CypherError::parse(c.here(), "expected a literal"));
    };
    let bad = || CypherError::parse(span, "OPTIONS takes literal values only");
    let v = match t.kind {
        TokenKind::Integer => {
            let txt = c.text(t).replace('_', "");
            let n: i64 = txt
                .parse()
                .map_err(|_| CypherError::parse(span, "integer out of range"))?;
            CypherValue::Integer(if neg { -n } else { n })
        }
        TokenKind::Float => {
            let x: f64 = c
                .text(t)
                .replace('_', "")
                .parse()
                .map_err(|_| CypherError::parse(span, "invalid float"))?;
            CypherValue::Float(if neg { -x } else { x })
        }
        _ if neg => return Err(bad()),
        TokenKind::String => CypherValue::String(unquote(c.text(t))),
        TokenKind::LeftBrace => return map(c).map(CypherValue::Map),
        TokenKind::LeftBracket => {
            c.i += 1;
            let mut xs = Vec::new();
            if c.peek().map(|t| t.kind) != Some(TokenKind::RightBracket) {
                loop {
                    xs.push(literal(c)?);
                    if c.peek().map(|t| t.kind) == Some(TokenKind::Comma) {
                        c.i += 1;
                    } else {
                        break;
                    }
                }
            }
            c.expect_kind(TokenKind::RightBracket, "]")?;
            return Ok(CypherValue::List(xs));
        }
        TokenKind::Keyword(_) | TokenKind::Identifier => {
            match c.text(t).to_ascii_lowercase().as_str() {
                "true" => CypherValue::Boolean(true),
                "false" => CypherValue::Boolean(false),
                "null" => CypherValue::Null,
                _ => return Err(bad()),
            }
        }
        _ => return Err(bad()),
    };
    c.i += 1;
    Ok(v)
}

/// A literal map `{k: v, …}` at the cursor.
fn map(c: &mut Cur) -> CResult<BTreeMap<String, CypherValue>> {
    c.expect_kind(TokenKind::LeftBrace, "{")?;
    let mut out = BTreeMap::new();
    if c.peek().map(|t| t.kind) == Some(TokenKind::RightBrace) {
        c.i += 1;
        return Ok(out);
    }
    loop {
        let k = key(c)?;
        c.expect_kind(TokenKind::Colon, ":")?;
        let v = literal(c)?;
        out.insert(k, v);
        match c.peek().map(|t| t.kind) {
            Some(TokenKind::Comma) => c.i += 1,
            _ => break,
        }
    }
    c.expect_kind(TokenKind::RightBrace, "}")?;
    Ok(out)
}

fn end(c: &mut Cur) -> CResult<()> {
    if c.peek().map(|t| t.kind) == Some(TokenKind::Semicolon) {
        c.i += 1;
    }
    match c.peek() {
        None => Ok(()),
        Some(t) => Err(CypherError::parse(
            t.span.into(),
            "a vector index command must be the whole query",
        )),
    }
}

/// Recognises a vector index command; `None` when `text` is not one.
///
/// # Errors
///
/// `Parse` for a malformed command, `Unsupported` for an index target
/// (`FOR … ON …`) or a non-vector `CREATE INDEX`.
// @lat: [[query#Vector Recall#Writes From Query Languages]]
pub fn recognise(text: &str) -> CResult<Option<SchemaCommand>> {
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
    let start = c.here();
    let first = c.word(0);
    let is_create = first.as_deref() == Some("create")
        && matches!(c.word(1).as_deref(), Some("vector" | "index"))
        && c.toks.get(1).map(|t| t.kind) != Some(TokenKind::LeftParen);
    let is_drop = first.as_deref() == Some("drop");
    let is_show = first.as_deref() == Some("show");
    if is_create {
        c.i += 1;
        if !c.eat("vector") {
            return Err(CypherError::unsupported(
                "CREATE INDEX other than CREATE VECTOR INDEX",
                Some(start),
            ));
        }
        c.expect("index")?;
        let name = name(&mut c)?;
        let mut if_not_exists = false;
        if c.eat("if") {
            c.expect("not")?;
            c.expect("exists")?;
            if_not_exists = true;
        }
        if c.word(0).as_deref() == Some("for") {
            return Err(CypherError::unsupported(
                "FOR … ON … in CREATE VECTOR INDEX (embeddings belong to statements)",
                Some(c.here()),
            ));
        }
        c.expect("options")?;
        let options = match c.peek() {
            Some(t) if t.kind == TokenKind::Parameter => {
                c.i += 1;
                IndexOptions::Param(c.text(t)[1..].to_string())
            }
            _ => IndexOptions::Map(map(&mut c)?),
        };
        end(&mut c)?;
        return Ok(Some(SchemaCommand::Create {
            name,
            if_not_exists,
            options,
        }));
    }
    if is_drop {
        c.i += 1;
        c.eat("vector");
        if c.word(0).as_deref() != Some("index") {
            return Err(CypherError::unsupported(
                "DROP other than DROP [VECTOR] INDEX",
                Some(start),
            ));
        }
        c.i += 1;
        let name = name(&mut c)?;
        let mut if_exists = false;
        if c.eat("if") {
            c.expect("exists")?;
            if_exists = true;
        }
        end(&mut c)?;
        return Ok(Some(SchemaCommand::Drop { name, if_exists }));
    }
    if is_show {
        c.i += 1;
        c.eat("vector");
        if !(c.eat("indexes") || c.eat("index")) {
            return Err(CypherError::unsupported(
                "SHOW other than SHOW [VECTOR] INDEXES",
                Some(start),
            ));
        }
        end(&mut c)?;
        return Ok(Some(SchemaCommand::Show));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_with_options() {
        let c = recognise(
            "CREATE VECTOR INDEX `urn:ex:m` IF NOT EXISTS OPTIONS {dimensions: 3, metric: 'dot', \
             indexConfig: {`vector.dimensions`: 3}}",
        )
        .unwrap()
        .unwrap();
        let SchemaCommand::Create {
            name,
            if_not_exists,
            options: IndexOptions::Map(m),
        } = c
        else {
            panic!("{c:?}")
        };
        assert_eq!(name.text, "urn:ex:m");
        assert!(name.escaped && if_not_exists);
        assert_eq!(m["dimensions"], CypherValue::Integer(3));
        assert_eq!(m["metric"], CypherValue::String("dot".into()));
    }

    #[test]
    fn drop_show_and_others() {
        assert_eq!(
            recognise("drop index m if exists;").unwrap(),
            Some(SchemaCommand::Drop {
                name: Name {
                    text: "m".into(),
                    escaped: false,
                    span: Span::new(11, 12)
                },
                if_exists: true
            })
        );
        assert_eq!(
            recognise("SHOW VECTOR INDEXES").unwrap(),
            Some(SchemaCommand::Show)
        );
        assert_eq!(recognise("CREATE (n:Index)").unwrap(), None);
        assert_eq!(recognise("MATCH (n) RETURN n").unwrap(), None);
        assert!(matches!(
            recognise("CREATE VECTOR INDEX m FOR ()-[r]-() ON (r.e) OPTIONS {}"),
            Err(CypherError::Unsupported { .. })
        ));
        assert!(matches!(
            recognise("CREATE VECTOR INDEX m OPTIONS {dimensions: 3} RETURN 1"),
            Err(CypherError::Parse { .. })
        ));
    }
}
