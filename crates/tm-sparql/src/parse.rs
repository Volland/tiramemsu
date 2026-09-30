//! Parser construction (predeclared prefixes), query-vs-update dispatch and
//! parse-error spans (design D2, D3, D4).

use spargebra::{Query, SparqlParser, SparqlSyntaxError, Update};
use tm_core::{Error, Result, Span};

use crate::env::Env;
use crate::error::parse_error;

/// A parsed request.
#[derive(Debug, Clone)]
pub enum Parsed {
    /// A query (`SELECT`, `ASK`, `CONSTRUCT`, `DESCRIBE`).
    Query(Box<Query>),
    /// An update request.
    Update(Update),
}

/// A parser with the predeclared prefixes of design D4: `rdf`, `rdfs`, `xsd`, `sys`,
/// `tm`, `v` (the database `@vocab`) and every row of the database prefix table.
pub fn parser(env: &Env) -> Result<SparqlParser> {
    let mut p = SparqlParser::new();
    let fixed: [(&str, &str); 5] = [
        ("rdf", tm_core::vocab::RDF),
        ("rdfs", "http://www.w3.org/2000/01/rdf-schema#"),
        ("xsd", tm_core::vocab::XSD),
        ("sys", tm_core::vocab::SYS),
        ("tm", tm_core::vocab::TM),
    ];
    let bad =
        |name: &str, e: String| parse_error(None, format!("invalid IRI for prefix `{name}`: {e}"));
    for (name, iri) in fixed {
        p = p
            .with_prefix(name, iri)
            .map_err(|e| bad(name, e.to_string()))?;
    }
    p = p
        .with_prefix("v", env.vocab.as_str())
        .map_err(|e| bad("v", e.to_string()))?;
    for (name, iri) in &env.prefixes {
        p = p
            .with_prefix(name.as_str(), iri.as_str())
            .map_err(|e| bad(name, e.to_string()))?;
    }
    Ok(p)
}

/// The 1-based line and column of byte `offset` in `text`.
pub fn line_col(text: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(text.len());
    let before = &text[..offset];
    let line = before.matches('\n').count() + 1;
    let start = before.rfind('\n').map_or(0, |i| i + 1);
    (line, before[start..].chars().count() + 1)
}

/// The byte offset of the 1-based `line`/`column` in `text`.
pub fn offset_of(text: &str, line: usize, column: usize) -> usize {
    let mut start = 0;
    for _ in 1..line {
        match text[start..].find('\n') {
            Some(i) => start += i + 1,
            None => return text.len(),
        }
    }
    text[start..]
        .char_indices()
        .nth(column.saturating_sub(1))
        .map_or(text.len(), |(i, _)| start + i)
}

/// The position of a syntax error (design D3). `spargebra` 0.4.7 keeps the `peg`
/// error behind a transparent wrapper whose `source()` chain does not reach it, so
/// the position is read from the `error at L:C: expected …` Display text (the
/// downcast is still tried first, in case a later version exposes it).
///
/// `peg` records the furthest position any alternative reached. When the
/// grammar's catch-all character (`[_]`) is among the expected tokens, that
/// position is one character past the offending one, so it is stepped back to
/// the character that could not be parsed.
pub fn span_of(err: &SparqlSyntaxError, text: &str) -> Option<Span> {
    let mut cur: Option<&(dyn std::error::Error + 'static)> = Some(err);
    let mut offset = None;
    let mut catch_all = false;
    while let Some(e) = cur {
        if let Some(pe) = e.downcast_ref::<peg::error::ParseError<peg::str::LineCol>>() {
            offset = Some(pe.location.offset.min(text.len()));
            catch_all = pe.expected.tokens().any(|t| t == "[_]");
            break;
        }
        cur = e.source();
    }
    let msg = err.to_string();
    if offset.is_none() {
        let s = display_span(&msg)?;
        offset = Some(offset_of(text, s.line, s.column));
        catch_all = msg.contains("[_]");
    }
    let mut offset = offset?;
    if catch_all && offset > 0 {
        // skip the whitespace the parser consumed after the offending character,
        // then step back onto that character
        offset = text[..offset].trim_end().len();
        offset = text[..offset]
            .char_indices()
            .next_back()
            .map_or(0, |(i, _)| i);
    }
    let (line, column) = line_col(text, offset);
    Some(Span {
        line,
        column,
        offset,
    })
}

/// Parses `error at L:C:` out of a Display text.
pub fn display_span(text: &str) -> Option<Span> {
    let rest = text.strip_prefix("error at ")?;
    let (l, r) = rest.split_once(':')?;
    let (c, _) = r.split_once(':')?;
    Some(Span {
        line: l.parse().ok()?,
        column: c.parse().ok()?,
        offset: 0,
    })
}

/// The first prefixed name whose prefix is neither declared in the text nor
/// predeclared: `(byte offset, prefix)`. `peg` loses spargebra's "Prefix not
/// found" message behind a later, further failure, so it is recovered here.
pub fn undeclared_prefix(text: &str, env: &Env) -> Option<(usize, String)> {
    let mut known: Vec<String> = ["rdf", "rdfs", "xsd", "sys", "tm", "v"]
        .iter()
        .map(|s| s.to_string())
        .chain(env.prefixes.iter().map(|(n, _)| n.clone()))
        .collect();
    let b = text.as_bytes();
    let mut i = 0;
    let mut found = None;
    while i < b.len() {
        match b[i] {
            b'#' => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'<' => {
                // an IRI reference (or a comparison operator followed by a space)
                match text[i + 1..].find(['>', ' ', '\n', '\t']) {
                    Some(e) if b[i + 1 + e] == b'>' => i += e + 2,
                    _ => i += 1,
                }
            }
            q @ (b'"' | b'\'') => {
                i += 1;
                while i < b.len() && b[i] != q {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
                i += 1;
            }
            c if c.is_ascii_alphabetic() => {
                let st = i;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'-')
                {
                    i += 1;
                }
                if i < b.len() && b[i] == b':' {
                    let word = &text[st..i];
                    if text[..st]
                        .to_ascii_uppercase()
                        .trim_end()
                        .ends_with("PREFIX")
                    {
                        known.push(word.to_string());
                    } else if found.is_none() && !known.iter().any(|k| k == word) {
                        found = Some((st, word.to_string()));
                    }
                }
            }
            _ => i += 1,
        }
    }
    found.filter(|(_, p)| !known.contains(p))
}

fn syntax_error(e: &SparqlSyntaxError, text: &str, env: &Env) -> Error {
    if let Some((off, prefix)) = undeclared_prefix(text, env) {
        let (line, column) = line_col(text, off);
        return parse_error(
            Some(Span {
                line,
                column,
                offset: off,
            }),
            format!("prefix `{prefix}:` not found"),
        );
    }
    parse_error(span_of(e, text), e.to_string())
}

/// Which operator classes of the text have no parenthesised right operand, so that
/// a right-nested chain in the algebra can only come from an unparenthesised
/// chain (see [`crate::lower::expr`]: spargebra 0.4.7 parses `a - b - c` as
/// `a - (b - c)`).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct AssocHints {
    /// No `+` or `-` is followed by `(`: additive chains may be re-associated.
    pub additive: bool,
    /// No `*` or `/` is followed by `(`: multiplicative chains may be re-associated.
    pub multiplicative: bool,
}

/// Scans `text` (outside strings, IRIs and comments) for an operator of each class
/// followed by an opening parenthesis.
pub fn assoc_hints(text: &str) -> AssocHints {
    let b = text.as_bytes();
    let (mut add_paren, mut mul_paren) = (false, false);
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'#' => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'<' => match text[i + 1..].find(['>', ' ', '\n', '\t']) {
                Some(e) if b[i + 1 + e] == b'>' => i += e + 2,
                _ => i += 1,
            },
            q @ (b'"' | b'\'') => {
                i += 1;
                while i < b.len() && b[i] != q {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
                i += 1;
            }
            c @ (b'+' | b'-' | b'*' | b'/') => {
                let next = text[i + 1..].trim_start().chars().next();
                if next == Some('(') {
                    if matches!(c, b'+' | b'-') {
                        add_paren = true;
                    } else {
                        mul_paren = true;
                    }
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    AssocHints {
        additive: !add_paren,
        multiplicative: !mul_paren,
    }
}

const UPDATE_KEYWORDS: [&str; 10] = [
    "INSERT", "DELETE", "LOAD", "CLEAR", "CREATE", "DROP", "ADD", "MOVE", "COPY", "WITH",
];

/// The first keyword after the `BASE` / `PREFIX` prologue, upper-cased.
pub fn first_keyword(text: &str) -> Option<String> {
    let mut rest = text;
    loop {
        rest = rest.trim_start();
        if let Some(r) = rest.strip_prefix('#') {
            rest = r.split_once('\n').map_or("", |(_, t)| t);
            continue;
        }
        let word: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphabetic())
            .collect();
        let up = word.to_ascii_uppercase();
        if up == "BASE" || up == "PREFIX" {
            // skip the declaration: `PREFIX name: <iri>` / `BASE <iri>`
            let after = &rest[word.len()..];
            match after.find('>') {
                Some(i) => rest = &after[i + 1..],
                None => return None,
            }
            continue;
        }
        return (!up.is_empty()).then_some(up);
    }
}

/// The operation keywords of an update request, one per `;`-separated operation,
/// upper-cased. `spargebra` rewrites `ADD`, `MOVE` and `COPY` into other operations,
/// so the unsupported ones are recognised here, on the text.
pub fn operation_keywords(text: &str) -> Vec<String> {
    let b = text.as_bytes();
    let (mut i, mut start, mut depth) = (0, 0, 0usize);
    let mut segments = Vec::new();
    while i < b.len() {
        match b[i] {
            b'#' => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'<' => {
                if let Some(e) = text[i + 1..].find(['>', ' ', '\n', '\t']) {
                    if b[i + 1 + e] == b'>' {
                        i += e + 2;
                        continue;
                    }
                }
            }
            q @ (b'"' | b'\'') => {
                i += 1;
                while i < b.len() && b[i] != q {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            b'{' => depth += 1,
            b'}' => depth = depth.saturating_sub(1),
            b';' if depth == 0 => {
                segments.push(&text[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    segments.push(&text[start..]);
    segments.into_iter().filter_map(first_keyword).collect()
}

/// Parses `text` as a query only (syntax tests): no dispatch, no path check.
pub fn parse_query_only(text: &str, env: &Env) -> Result<Query> {
    parser(env)?
        .parse_query(text)
        .map_err(|e| syntax_error(&e, text, env))
}

/// Parses `text` as an update only (syntax tests): no dispatch, no path check.
pub fn parse_update_only(text: &str, env: &Env) -> Result<Update> {
    parser(env)?
        .parse_update(text)
        .map_err(|e| syntax_error(&e, text, env))
}

/// Parses `text` as a query, else as an update (design D2).
pub fn parse(text: &str, env: &Env) -> Result<Parsed> {
    parse_inner(text, env)
}

fn parse_inner(text: &str, env: &Env) -> Result<Parsed> {
    let qerr = match parser(env)?.parse_query(text) {
        Ok(q) => return Ok(Parsed::Query(Box::new(q))),
        Err(e) => e,
    };
    let uerr = match parser(env)?.parse_update(text) {
        Ok(u) => return Ok(Parsed::Update(u)),
        Err(e) => e,
    };
    let is_update = first_keyword(text).is_some_and(|k| UPDATE_KEYWORDS.contains(&k.as_str()));
    Err(syntax_error(
        if is_update { &uerr } else { &qerr },
        text,
        env,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tm_ir::View;

    fn env() -> Env {
        Env::new(View::NOW)
    }

    // sparql-query "Predeclared prefixes": Default vocabulary prefix
    #[test]
    fn v_prefix_resolves_without_declaration() {
        let Parsed::Query(q) = parse(
            "SELECT ?c WHERE { <urn:tiramemsu:v:alice> v:worksAt ?c }",
            &env(),
        )
        .unwrap() else {
            panic!()
        };
        assert!(q.to_string().contains("<urn:tiramemsu:v:worksAt>"));
    }

    // sparql-query "Predeclared prefixes": Query prefix overrides
    #[test]
    fn query_prefix_overrides() {
        let Parsed::Query(q) = parse(
            "PREFIX v: <http://example.org/> SELECT ?c WHERE { ?c v:x 1 }",
            &env(),
        )
        .unwrap() else {
            panic!()
        };
        assert!(q.to_string().contains("<http://example.org/x>"));
    }

    // sparql-query "Predeclared prefixes": Database prefix table
    #[test]
    fn database_prefix_table() {
        let mut e = env();
        e.prefixes
            .push(("schema".into(), "https://schema.org/".into()));
        let Parsed::Query(q) = parse("SELECT ?n WHERE { ?p schema:name ?n }", &e).unwrap() else {
            panic!()
        };
        assert!(q.to_string().contains("<https://schema.org/name>"));
    }

    // sparql-query "Parse errors report position": Undeclared prefix
    #[test]
    fn undeclared_prefix() {
        let Err(Error::Parse { dialect, msg, .. }) =
            parse("SELECT ?s WHERE { ?s ex:p ?o }", &env())
        else {
            panic!()
        };
        assert_eq!(dialect, tm_core::Dialect::Sparql);
        assert!(msg.to_lowercase().contains("prefix"), "{msg}");
    }

    #[test]
    fn dispatch_reports_the_right_parser_error() {
        // update keyword: the update parser's error
        let e = parse("INSERT DATA { ?s <http://x/p> 1 }", &env()).unwrap_err();
        assert!(matches!(e, Error::Parse { .. }));
        assert_eq!(
            first_keyword("PREFIX a: <http://a/> INSERT DATA {}").as_deref(),
            Some("INSERT")
        );
        assert_eq!(
            first_keyword("# c\nselect ?x {}").as_deref(),
            Some("SELECT")
        );
        assert!(matches!(
            parse("INSERT DATA { <a:b> <a:c> <a:d> }", &env()).unwrap(),
            Parsed::Update(_)
        ));
        assert!(matches!(
            parse("SELECT * { ?s ?p ?o }", &env()).unwrap(),
            Parsed::Query(_)
        ));
        // other text: the query parser's error (a line-1 error inside a SELECT)
        let Err(Error::Parse { span: Some(s), .. }) = parse("SELECT ?x WHERE { ?x ?y }", &env())
        else {
            panic!()
        };
        assert_eq!(s.line, 1);
    }

    // sparql-query "Parse errors report position": Syntax error location
    #[test]
    fn syntax_error_location() {
        let text = "SELECT ?s WHERE {\n  ?s v:p ?o\n  FILTER(\n}";
        let Err(Error::Parse { span: Some(s), .. }) = parse(text, &env()) else {
            panic!()
        };
        assert_eq!((s.line, s.column), (4, 1));
        assert_eq!(&text[s.offset..], "}");
    }

    #[test]
    fn syntax_error_location_with_trailing_whitespace() {
        let text = "SELECT ?s WHERE {\n  ?s v:p ?o\n  FILTER(\n}\n\n";
        let Err(Error::Parse { span: Some(s), .. }) = parse(text, &env()) else {
            panic!()
        };
        assert_eq!((s.line, s.column), (4, 1));
    }

    #[test]
    fn display_fallback_parses() {
        let s = display_span("error at 4:1: expected x").unwrap();
        assert_eq!((s.line, s.column), (4, 1));
        assert!(display_span("nothing").is_none());
    }
}
