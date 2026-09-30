//! The path text parser: SPARQL 1.1 property path syntax (precedence `|` < `/` <
//! `^` < postfix) plus `{m,n}`, `{m,}` and `{n}`. Atoms are `<iri>`, CURIEs
//! (declared prefixes and `sys:`/`tm:`/`v:`/`rdf:`/`rdfs:`/`xsd:`) and bare names
//! through `@vocab`. Errors carry byte offsets.

use tm_core::vocab::{RDF, SYS, TM, XSD};
use tm_core::{Dialect, Error, Result, Span, Vocab};
use tm_ir::PathExpr;

/// Where bare names and declared prefixes come from; a database source is read
/// only when the text needs it (`sys:`, `tm:`, `rdf:`, `xsd:` and `<iri>` never do).
pub trait VocabSource {
    /// The current vocabulary.
    fn vocab(&mut self) -> Result<&Vocab>;
}

impl VocabSource for Vocab {
    fn vocab(&mut self) -> Result<&Vocab> {
        Ok(self)
    }
}

/// Parses path text into an AST.
pub fn parse(text: &str, vocab: &mut dyn VocabSource) -> Result<PathExpr> {
    let mut p = Parser {
        text,
        s: text.as_bytes(),
        i: 0,
        vocab,
    };
    let e = p.alt()?;
    p.ws();
    if p.i < p.s.len() {
        return Err(p.err(p.i, format!("unexpected `{}`", p.rest_char())));
    }
    Ok(e)
}

struct Parser<'a> {
    text: &'a str,
    s: &'a [u8],
    i: usize,
    vocab: &'a mut dyn VocabSource,
}

fn is_name(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b'%') || c >= 0x80
}

impl Parser<'_> {
    fn err(&self, offset: usize, msg: String) -> Error {
        let before = &self.text[..offset.min(self.text.len())];
        let line = before.matches('\n').count() + 1;
        let column = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
        Error::parse(
            Dialect::Path,
            Some(Span {
                line,
                column,
                offset,
            }),
            msg,
        )
    }

    fn rest_char(&self) -> char {
        self.text[self.i..].chars().next().unwrap_or(' ')
    }

    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.ws();
        self.s.get(self.i).copied()
    }

    fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn alt(&mut self) -> Result<PathExpr> {
        let mut xs = vec![self.seq()?];
        while self.eat(b'|') {
            xs.push(self.seq()?);
        }
        Ok(if xs.len() == 1 {
            xs.remove(0)
        } else {
            PathExpr::Alt(xs)
        })
    }

    fn seq(&mut self) -> Result<PathExpr> {
        let mut xs = vec![self.unary()?];
        while self.eat(b'/') {
            xs.push(self.unary()?);
        }
        Ok(if xs.len() == 1 {
            xs.remove(0)
        } else {
            PathExpr::Seq(xs)
        })
    }

    fn unary(&mut self) -> Result<PathExpr> {
        if self.eat(b'^') {
            Ok(PathExpr::Inverse(Box::new(self.unary()?)))
        } else {
            self.elt()
        }
    }

    fn elt(&mut self) -> Result<PathExpr> {
        let mut p = self.primary()?;
        loop {
            match self.peek() {
                Some(b'*') => {
                    self.i += 1;
                    p = PathExpr::ZeroOrMore(Box::new(p));
                }
                Some(b'+') => {
                    self.i += 1;
                    p = PathExpr::OneOrMore(Box::new(p));
                }
                Some(b'?') => {
                    self.i += 1;
                    p = PathExpr::ZeroOrOne(Box::new(p));
                }
                Some(b'{') => {
                    let at = self.i;
                    self.i += 1;
                    let min = self.num()?;
                    let max = if self.eat(b',') {
                        if self.peek() == Some(b'}') {
                            None
                        } else {
                            Some(self.num()?)
                        }
                    } else {
                        Some(min)
                    };
                    if !self.eat(b'}') {
                        return Err(self.err(self.i, "expected `}`".into()));
                    }
                    if max.is_some_and(|m| m < min) {
                        return Err(self.err(at, format!("repetition maximum below minimum {min}")));
                    }
                    p = PathExpr::Repeat {
                        inner: Box::new(p),
                        min,
                        max,
                    };
                }
                _ => return Ok(p),
            }
        }
    }

    fn num(&mut self) -> Result<u32> {
        self.ws();
        let st = self.i;
        while self.i < self.s.len() && self.s[self.i].is_ascii_digit() {
            self.i += 1;
        }
        self.text[st..self.i]
            .parse()
            .map_err(|_| self.err(st, "expected a number".into()))
    }

    fn primary(&mut self) -> Result<PathExpr> {
        match self.peek() {
            Some(b'(') => {
                self.i += 1;
                let e = self.alt()?;
                if !self.eat(b')') {
                    return Err(self.err(self.i, "expected `)`".into()));
                }
                Ok(e)
            }
            Some(b'<') => {
                let st = self.i + 1;
                let Some(len) = self.text[st..].find('>') else {
                    return Err(self.err(self.i, "unterminated `<iri>`".into()));
                };
                self.i = st + len + 1;
                Ok(PathExpr::iri(&self.text[st..st + len]))
            }
            Some(c) if c.is_ascii_alphabetic() || c == b'_' || c >= 0x80 => {
                let st = self.i;
                while self.i < self.s.len() && (is_name(self.s[self.i]) || self.s[self.i] == b':') {
                    self.i += 1;
                }
                // a trailing `.` belongs to the surrounding text, not the name
                let word = &self.text[st..self.i];
                match word.split_once(':') {
                    Some((pre, local)) => {
                        let fixed = match pre {
                            "sys" => Some(SYS),
                            "tm" => Some(TM),
                            "rdf" => Some(RDF),
                            "xsd" => Some(XSD),
                            _ => None,
                        };
                        let base = match fixed {
                            Some(b) => Some(b.to_string()),
                            None => self.vocab.vocab()?.prefix_iri(pre),
                        };
                        match base {
                            Some(base) => Ok(PathExpr::iri(format!("{base}{local}"))),
                            None => Err(self.err(st, format!("unknown prefix `{pre}`"))),
                        }
                    }
                    None => {
                        let iri = self
                            .vocab
                            .vocab()?
                            .resolve_text(word, false)
                            .map_err(|m| self.err(st, m))?;
                        Ok(PathExpr::iri(iri))
                    }
                }
            }
            Some(_) => Err(self.err(
                self.i,
                format!("expected a predicate, found `{}`", self.rest_char()),
            )),
            None => Err(self.err(self.i, "unexpected end of path".into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tm_ir::display::path_text;

    fn v(l: &str) -> String {
        format!("urn:tiramemsu:v:{l}")
    }

    fn vocab() -> Vocab {
        Vocab {
            prefixes: vec![("schema".into(), "https://schema.org/".into())],
            ..Vocab::default()
        }
    }

    fn offset(text: &str) -> usize {
        match parse(text, &mut vocab()) {
            Err(Error::Parse {
                dialect: Dialect::Path,
                span: Some(s),
                ..
            }) => s.offset,
            other => panic!("{text}: {other:?}"),
        }
    }

    // path-evaluation "Path expression operators": every operator and the precedence
    #[test]
    fn operators_and_precedence() {
        let p = |t: &str| parse(t, &mut vocab()).unwrap();
        let a = || PathExpr::iri(v("a"));
        let b = || PathExpr::iri(v("b"));
        let c = || PathExpr::iri(v("c"));
        assert_eq!(p("a"), a());
        assert_eq!(p("a/b"), PathExpr::Seq(vec![a(), b()]));
        assert_eq!(p("a|b"), PathExpr::Alt(vec![a(), b()]));
        assert_eq!(p("^a"), PathExpr::Inverse(Box::new(a())));
        assert_eq!(p("a*"), a().star());
        assert_eq!(p("a+"), a().plus());
        assert_eq!(p("a?"), PathExpr::ZeroOrOne(Box::new(a())));
        // `|` < `/` < `^` < postfix
        assert_eq!(
            p("a/b|c"),
            PathExpr::Alt(vec![PathExpr::Seq(vec![a(), b()]), c()])
        );
        assert_eq!(
            p("^a/b"),
            PathExpr::Seq(vec![PathExpr::Inverse(Box::new(a())), b()])
        );
        assert_eq!(
            p("^a+"),
            PathExpr::Inverse(Box::new(a().plus())),
            "postfix binds tighter than ^"
        );
        assert_eq!(p("(a/b)+"), PathExpr::Seq(vec![a(), b()]).plus());
        assert_eq!(p(" a / b "), p("a/b"));
        let rep = |min, max| PathExpr::Repeat {
            inner: Box::new(a()),
            min,
            max,
        };
        assert_eq!(p("a{2,3}"), rep(2, Some(3)));
        assert_eq!(p("a{2,}"), rep(2, None));
        assert_eq!(p("a{4}"), rep(4, Some(4)));
    }

    // path-table-function "Wildcard atom" / "Full IRI and CURIE atoms" / "Bare names resolve through vocab"
    #[test]
    fn atoms() {
        let p = |t: &str| parse(t, &mut vocab()).unwrap();
        assert_eq!(
            p("SUPPORTED_BY/sys:subject"),
            PathExpr::Seq(vec![
                PathExpr::iri(v("SUPPORTED_BY")),
                PathExpr::iri(tm_ir::vocab::SYS_SUBJECT)
            ])
        );
        assert_eq!(
            p("sys:anyRelationship"),
            PathExpr::iri(tm_ir::vocab::SYS_ANY_RELATIONSHIP)
        );
        assert_eq!(
            p("<https://schema.org/knows>|schema:follows"),
            PathExpr::Alt(vec![
                PathExpr::iri("https://schema.org/knows"),
                PathExpr::iri("https://schema.org/follows")
            ])
        );
        assert_eq!(p("v:x"), PathExpr::iri(v("x")));
        // the canonical printer's output parses back
        let e = p("(schema:a|^b)+/sys:subject*");
        assert_eq!(p(&path_text(&e)), e);
    }

    // path-table-function "Malformed path text" / "Unknown prefix"
    #[test]
    fn errors_carry_offsets() {
        assert_eq!(offset("knows//likes"), 6);
        assert_eq!(offset("(knows"), 6);
        assert_eq!(offset("a{3,1}"), 1);
        assert_eq!(offset("a{x}"), 2);
        assert_eq!(offset("<abc"), 0);
        assert_eq!(offset(""), 0);
        assert_eq!(offset("a b"), 2);
        match parse("nope:knows", &mut vocab()) {
            Err(Error::Parse { msg, span, .. }) => {
                assert!(msg.contains("unknown prefix `nope`"), "{msg}");
                assert_eq!(span.unwrap().offset, 0);
            }
            other => panic!("{other:?}"),
        }
    }

    use proptest::prelude::*;
    use tm_ir::display::path_text_canonical;

    fn arb_ast() -> impl Strategy<Value = PathExpr> {
        let leaf = prop_oneof![
            Just(PathExpr::iri("urn:x:a")),
            Just(PathExpr::iri(v("b"))),
            Just(PathExpr::iri(tm_ir::vocab::SYS_SUBJECT)),
            Just(PathExpr::iri(tm_ir::vocab::SYS_ANY_RELATIONSHIP)),
        ];
        leaf.prop_recursive(4, 24, 3, |inner| {
            prop_oneof![
                inner.clone().prop_map(|e| PathExpr::Inverse(Box::new(e))),
                prop::collection::vec(inner.clone(), 2..4).prop_map(PathExpr::Seq),
                prop::collection::vec(inner.clone(), 2..4).prop_map(PathExpr::Alt),
                inner
                    .clone()
                    .prop_map(|e| PathExpr::ZeroOrMore(Box::new(e))),
                inner.clone().prop_map(|e| PathExpr::OneOrMore(Box::new(e))),
                inner.clone().prop_map(|e| PathExpr::ZeroOrOne(Box::new(e))),
                (inner, 0u32..4, prop::option::of(0u32..4)).prop_map(|(e, min, extra)| {
                    PathExpr::Repeat {
                        inner: Box::new(e),
                        min,
                        max: extra.map(|x| min + x),
                    }
                }),
            ]
        })
    }

    proptest! {
        // task 2.5: canonical text -> parse is the identity on random ASTs
        #[test]
        fn print_parse_is_identity(e in arb_ast()) {
            let text = path_text_canonical(&e);
            prop_assert_eq!(parse(&text, &mut vocab()).unwrap(), e, "{}", text);
        }
    }
}
