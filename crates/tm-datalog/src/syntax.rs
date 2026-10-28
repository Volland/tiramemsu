//! The program text: lexer, recursive-descent parser and syntax tree
//! (`lat.md/datalog#Souffle-Style Text`).
//!
//! ```text
//! @prefix ex: <http://example.org/> .
//! reports(X, Y) :- ex:manager(X, Y).
//! reports(X, Z) :- reports(X, Y), ex:manager(Y, Z).
//! ?- reports(ex:alice, X).
//! constraint no_cycle error :- reports(X, X).
//! ```

use std::collections::HashMap;

use tm_core::vocab::{RDF, XSD};
use tm_core::{Error, Result, Value};

/// A position in the text (1-based).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Pos {
    /// Line.
    pub line: usize,
    /// Column.
    pub column: usize,
}

/// A predicate name: an IRI (stored or derived) or a plain name (derived or a
/// reserved atom or builtin).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Pred {
    /// A full IRI.
    Iri(String),
    /// A plain lower-case name.
    Name(String),
}

impl std::fmt::Display for Pred {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Pred::Iri(i) => write!(f, "<{i}>"),
            Pred::Name(n) => f.write_str(n),
        }
    }
}

/// An expression: a term, an arithmetic operation, a function call.
#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    /// A variable.
    Var(String),
    /// The anonymous variable `_`.
    Anon,
    /// A constant.
    Const(Value),
    /// The constant `inf` (an open end).
    Inf,
    /// A binary arithmetic operation.
    Bin(Box<Expr>, BinOp, Box<Expr>),
    /// A builtin function call (`concat(A, B)`, `now()`).
    Call(String, Vec<Expr>),
}

/// Arithmetic operators.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BinOp {
    /// `+`.
    Add,
    /// `-`.
    Sub,
    /// `*`.
    Mul,
    /// `/`.
    Div,
    /// `%`.
    Rem,
}

/// Comparison operators.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CmpOp {
    /// `=`.
    Eq,
    /// `!=`.
    Ne,
    /// `<`.
    Lt,
    /// `<=`.
    Le,
    /// `>`.
    Gt,
    /// `>=`.
    Ge,
}

/// An atom `p(args) [@ E]`.
#[derive(Clone, Debug, PartialEq)]
pub struct Atom {
    /// The predicate.
    pub pred: Pred,
    /// The arguments.
    pub args: Vec<Expr>,
    /// The statement-eid binding of `@ E`.
    pub eid: Option<Expr>,
    /// Where the atom starts.
    pub pos: Pos,
}

/// A metric window `[lo, hi]` in milliseconds; `hi` `None` is `inf`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Window {
    /// Lower bound (≥ 0).
    pub lo: i64,
    /// Upper bound, `None` for `inf`.
    pub hi: Option<i64>,
}

/// The unary metric temporal operators of DatalogMTL.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum TempOp {
    /// `past[a, b] A`: A held at some instant in `[t−b, t−a]`.
    Past,
    /// `always_past[a, b] A`: A held throughout `[t−b, t−a]`.
    AlwaysPast,
    /// `future[a, b] A`: A holds at some instant in `[t+a, t+b]`.
    Future,
    /// `always_future[a, b] A`: A holds throughout `[t+a, t+b]`.
    AlwaysFuture,
}

impl TempOp {
    /// The keyword.
    pub fn name(self) -> &'static str {
        match self {
            TempOp::Past => "past",
            TempOp::AlwaysPast => "always_past",
            TempOp::Future => "future",
            TempOp::AlwaysFuture => "always_future",
        }
    }
}

/// An atom under zero or more temporal operators, outermost first.
#[derive(Clone, Debug, PartialEq)]
pub struct TAtom {
    /// The operators.
    pub ops: Vec<(TempOp, Window)>,
    /// The atom.
    pub atom: Atom,
}

/// Aggregate functions.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum AggFn {
    /// `count`.
    Count,
    /// `sum V`.
    Sum,
    /// `min V`.
    Min,
    /// `max V`.
    Max,
}

/// One element of a body.
#[derive(Clone, Debug, PartialEq)]
pub enum Literal {
    /// A positive atom.
    Pos(TAtom),
    /// `not A` / `!A`.
    Neg(TAtom),
    /// `l op r`.
    Cmp(Expr, CmpOp, Expr, Pos),
    /// `R = fn [V] : { body }`.
    Agg {
        /// The result variable.
        result: String,
        /// The function.
        func: AggFn,
        /// The aggregated expression (sum, min, max).
        over: Option<Expr>,
        /// The aggregate body.
        body: Vec<Literal>,
        /// Where it starts.
        pos: Pos,
    },
    /// `A since[a, b] B` or `A until[a, b] B`.
    Binary {
        /// The left atom (must hold throughout).
        lhs: TAtom,
        /// True for `until`.
        until: bool,
        /// The window.
        window: Window,
        /// The right atom (held once).
        rhs: TAtom,
    },
}

/// Constraint severity.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Severity {
    /// Reported only.
    Warn,
    /// Enforced at commit when the program is enforced.
    Error,
}

impl Severity {
    /// `"warn"` or `"error"`.
    pub fn name(self) -> &'static str {
        match self {
            Severity::Warn => "warn",
            Severity::Error => "error",
        }
    }
}

/// A rule head.
#[derive(Clone, Debug, PartialEq)]
pub struct Head {
    /// Variables introduced by `exists`.
    pub exists: Vec<String>,
    /// A head operator (`always_past` or `always_future`).
    pub op: Option<(TempOp, Window)>,
    /// The atom.
    pub atom: Atom,
}

/// One top-level item.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    /// A rule (a fact has an empty body).
    Rule {
        /// The head.
        head: Head,
        /// The body.
        body: Vec<Literal>,
        /// Where it starts.
        pos: Pos,
    },
    /// `?- body.`
    Goal {
        /// The body.
        body: Vec<Literal>,
        /// Where it starts.
        pos: Pos,
    },
    /// `constraint name sev :- body.`
    Constraint {
        /// The name.
        name: String,
        /// The severity.
        severity: Severity,
        /// The body.
        body: Vec<Literal>,
        /// Where it starts.
        pos: Pos,
    },
}

/// A parsed program.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ast {
    /// The items, in text order.
    pub items: Vec<Item>,
}

// ---------------------------------------------------------------- lexer

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    /// Lower-case identifier or keyword.
    Name(String),
    /// Variable (upper-case or `_` start); `_` alone is anonymous.
    Var(String),
    /// `prefix:local`.
    PName(String, String),
    /// `<iri>`.
    Iri(String),
    /// `"..."` with optional `@lang` or `^^dt` (dt kept raw: Iri or PName).
    Str(String, Option<String>, Option<Box<Tok>>),
    /// A number's lexical form.
    Num(String),
    /// A duration in milliseconds (`30d`).
    Dur(i64),
    Sym(&'static str),
    Eof,
}

struct Lexer<'a> {
    src: &'a [u8],
    i: usize,
    line: usize,
    col: usize,
}

const SYMS: [&str; 22] = [
    ":-", "?-", "!=", "<=", ">=", "^^", "(", ")", "[", "]", "{", "}", ",", ".", ":", "@", "=", "<",
    ">", "+", "-", "!",
];

impl<'a> Lexer<'a> {
    fn err(&self, msg: impl Into<String>) -> Error {
        Error::DatalogParse {
            line: self.line,
            column: self.col,
            message: msg.into(),
        }
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.i).copied()
    }

    fn peek_at(&self, k: usize) -> Option<u8> {
        self.src.get(self.i + k).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let c = self.peek()?;
        self.i += 1;
        if c == b'\n' {
            self.line += 1;
            self.col = 1;
        } else if c & 0xC0 != 0x80 {
            self.col += 1;
        }
        Some(c)
    }

    fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_ascii_whitespace() {
                self.bump();
            } else if c == b'%' || (c == b'/' && self.peek_at(1) == Some(b'/')) {
                while let Some(c) = self.peek() {
                    if c == b'\n' {
                        break;
                    }
                    self.bump();
                }
            } else {
                break;
            }
        }
    }

    fn name_char(c: u8) -> bool {
        c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80
    }

    fn take_while(&mut self, f: impl Fn(u8) -> bool) -> String {
        let start = self.i;
        while self.peek().is_some_and(&f) {
            self.bump();
        }
        String::from_utf8_lossy(&self.src[start..self.i]).into_owned()
    }

    fn next(&mut self) -> Result<(Tok, Pos)> {
        self.skip_ws();
        let pos = Pos {
            line: self.line,
            column: self.col,
        };
        let Some(c) = self.peek() else {
            return Ok((Tok::Eof, pos));
        };
        // IRI
        if c == b'<' {
            let rest = &self.src[self.i + 1..];
            let end = rest
                .iter()
                .position(|&b| b == b'>' || b.is_ascii_whitespace());
            if let Some(n) = end {
                if rest[n] == b'>' && n > 0 && !rest[..n].contains(&b'<') {
                    self.bump();
                    let iri = String::from_utf8_lossy(&rest[..n]).into_owned();
                    for _ in 0..=n {
                        self.bump();
                    }
                    return Ok((Tok::Iri(iri), pos));
                }
            }
        }
        if c == b'"' {
            return self.string(pos);
        }
        if c.is_ascii_digit() {
            let mut lex = self.take_while(|b| b.is_ascii_digit());
            if self.peek() == Some(b'.') && self.peek_at(1).is_some_and(|b| b.is_ascii_digit()) {
                self.bump();
                lex.push('.');
                lex.push_str(&self.take_while(|b| b.is_ascii_digit()));
            }
            if matches!(self.peek(), Some(b'e' | b'E'))
                && self
                    .peek_at(1)
                    .is_some_and(|b| b.is_ascii_digit() || b == b'-' || b == b'+')
            {
                lex.push(self.bump().unwrap_or(b'e') as char);
                if matches!(self.peek(), Some(b'-' | b'+')) {
                    lex.push(self.bump().unwrap_or(b'+') as char);
                }
                lex.push_str(&self.take_while(|b| b.is_ascii_digit()));
            }
            if self.peek().is_some_and(|b| b.is_ascii_alphabetic()) {
                let unit = self.take_while(|b| b.is_ascii_alphabetic());
                let mult = match unit.as_str() {
                    "ms" => 1,
                    "s" => 1_000,
                    "m" => 60_000,
                    "h" => 3_600_000,
                    "d" => 86_400_000,
                    "w" => 604_800_000,
                    _ => return Err(self.err(format!("unknown duration unit `{unit}`"))),
                };
                let n: i64 = lex
                    .parse()
                    .map_err(|_| self.err("a duration needs an integer"))?;
                let ms = n
                    .checked_mul(mult)
                    .ok_or_else(|| self.err("duration out of range"))?;
                return Ok((Tok::Dur(ms), pos));
            }
            return Ok((Tok::Num(lex), pos));
        }
        if c.is_ascii_alphabetic() || c == b'_' || c >= 0x80 {
            let word = self.take_while(Self::name_char);
            // prefixed name: `pre:local` with no space after the colon
            if self.peek() == Some(b':')
                && self.peek_at(1).is_some_and(Self::name_char)
                && self.peek_at(1) != Some(b'-')
            {
                self.bump();
                let mut local = String::new();
                loop {
                    let part = self.take_while(|b| Self::name_char(b) || b == b'-');
                    local.push_str(&part);
                    // an inner dot continues the name; a final one ends the item
                    if self.peek() == Some(b'.') && self.peek_at(1).is_some_and(Self::name_char) {
                        self.bump();
                        local.push('.');
                        continue;
                    }
                    break;
                }
                return Ok((Tok::PName(word, local), pos));
            }
            let first = word.as_bytes()[0];
            if first.is_ascii_uppercase() || first == b'_' {
                return Ok((Tok::Var(word), pos));
            }
            return Ok((Tok::Name(word), pos));
        }
        for s in SYMS {
            if self.src[self.i..].starts_with(s.as_bytes()) {
                for _ in 0..s.len() {
                    self.bump();
                }
                return Ok((Tok::Sym(s), pos));
            }
        }
        for s in ["*", "/", "%"] {
            if self.src[self.i..].starts_with(s.as_bytes()) {
                self.bump();
                return Ok((
                    Tok::Sym(match s {
                        "*" => "*",
                        "/" => "/",
                        _ => "%",
                    }),
                    pos,
                ));
            }
        }
        Err(self.err(format!("unexpected character `{}`", c as char)))
    }

    fn string(&mut self, pos: Pos) -> Result<(Tok, Pos)> {
        self.bump();
        let mut out = Vec::new();
        loop {
            match self.bump() {
                None => return Err(self.err("unterminated string")),
                Some(b'"') => break,
                Some(b'\\') => match self.bump() {
                    Some(b'n') => out.push(b'\n'),
                    Some(b't') => out.push(b'\t'),
                    Some(b'r') => out.push(b'\r'),
                    Some(b'"') => out.push(b'"'),
                    Some(b'\\') => out.push(b'\\'),
                    _ => return Err(self.err("bad escape in string")),
                },
                Some(c) => out.push(c),
            }
        }
        let text = String::from_utf8(out).map_err(|_| self.err("string is not UTF-8"))?;
        if self.peek() == Some(b'@') && self.peek_at(1).is_some_and(|b| b.is_ascii_alphabetic()) {
            self.bump();
            let lang = self.take_while(|b| b.is_ascii_alphanumeric() || b == b'-');
            return Ok((Tok::Str(text, Some(lang), None), pos));
        }
        if self.src[self.i..].starts_with(b"^^") {
            self.bump();
            self.bump();
            let (dt, _) = self.next()?;
            if !matches!(dt, Tok::Iri(_) | Tok::PName(..)) {
                return Err(self.err("`^^` needs a datatype IRI"));
            }
            return Ok((Tok::Str(text, None, Some(Box::new(dt))), pos));
        }
        Ok((Tok::Str(text, None, None), pos))
    }
}

// ---------------------------------------------------------------- parser

/// Prefixes every program may use without declaring them.
pub const DEFAULT_PREFIXES: [(&str, &str); 8] = [
    ("rdf", RDF),
    ("rdfs", "http://www.w3.org/2000/01/rdf-schema#"),
    ("xsd", XSD),
    ("owl", "http://www.w3.org/2002/07/owl#"),
    ("dl", tm_core::vocab::DL),
    ("sys", tm_core::vocab::SYS),
    ("tm", tm_core::vocab::TM),
    ("v", tm_core::vocab::V),
];

struct Parser<'a> {
    lx: Lexer<'a>,
    tok: Tok,
    pos: Pos,
    prefixes: HashMap<String, String>,
}

/// Parses program text.
// @tg: implements:: [[openspec:datalog-language#Program text]] {since: "2026-10-28"}
pub fn parse(text: &str) -> Result<Ast> {
    let mut lx = Lexer {
        src: text.as_bytes(),
        i: 0,
        line: 1,
        col: 1,
    };
    let (tok, pos) = lx.next()?;
    let mut p = Parser {
        lx,
        tok,
        pos,
        prefixes: DEFAULT_PREFIXES
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect(),
    };
    let mut items = Vec::new();
    while p.tok != Tok::Eof {
        if let Some(item) = p.item()? {
            items.push(item);
        }
    }
    Ok(Ast { items })
}

const AGGS: [&str; 4] = ["count", "sum", "min", "max"];

impl Parser<'_> {
    fn err_at(&self, pos: Pos, msg: impl Into<String>) -> Error {
        Error::DatalogParse {
            line: pos.line,
            column: pos.column,
            message: msg.into(),
        }
    }

    fn err(&self, msg: impl Into<String>) -> Error {
        self.err_at(self.pos, msg)
    }

    fn advance(&mut self) -> Result<Tok> {
        let (t, p) = self.lx.next()?;
        self.pos = p;
        Ok(std::mem::replace(&mut self.tok, t))
    }

    fn is(&self, s: &str) -> bool {
        matches!(&self.tok, Tok::Sym(x) if *x == s)
    }

    fn is_name(&self, s: &str) -> bool {
        matches!(&self.tok, Tok::Name(x) if x == s)
    }

    fn expect(&mut self, s: &str) -> Result<()> {
        if self.is(s) {
            self.advance()?;
            Ok(())
        } else {
            Err(self.err(format!("expected `{s}`, found {}", self.describe())))
        }
    }

    fn describe(&self) -> String {
        match &self.tok {
            Tok::Eof => "end of input".into(),
            Tok::Name(n) | Tok::Var(n) => format!("`{n}`"),
            Tok::PName(a, b) => format!("`{a}:{b}`"),
            Tok::Iri(i) => format!("`<{i}>`"),
            Tok::Str(..) => "a string".into(),
            Tok::Num(n) => format!("`{n}`"),
            Tok::Dur(_) => "a duration".into(),
            Tok::Sym(s) => format!("`{s}`"),
        }
    }

    fn iri_of(&self, t: &Tok, pos: Pos) -> Result<String> {
        match t {
            Tok::Iri(i) => Ok(i.clone()),
            Tok::PName(pre, local) => match self.prefixes.get(pre) {
                Some(ns) => Ok(format!("{ns}{local}")),
                None => Err(self.err_at(pos, format!("undeclared prefix `{pre}:`"))),
            },
            _ => Err(self.err_at(pos, "expected an IRI")),
        }
    }

    fn item(&mut self) -> Result<Option<Item>> {
        let pos = self.pos;
        if self.is("@") {
            self.advance()?;
            if !self.is_name("prefix") {
                return Err(self.err("expected `prefix` after `@`"));
            }
            self.advance()?;
            let name = match self.advance()? {
                Tok::Name(n) | Tok::Var(n) => n,
                _ => return Err(self.err_at(pos, "expected a prefix name")),
            };
            self.expect(":")?;
            let iri = match self.advance()? {
                Tok::Iri(i) => i,
                _ => return Err(self.err("expected `<iri>` in `@prefix`")),
            };
            self.expect(".")?;
            self.prefixes.insert(name, iri);
            return Ok(None);
        }
        if self.is("?-") {
            self.advance()?;
            let body = self.body()?;
            self.expect(".")?;
            return Ok(Some(Item::Goal { body, pos }));
        }
        if self.is_name("constraint") {
            self.advance()?;
            let name = match self.advance()? {
                Tok::Name(n) | Tok::Var(n) => n,
                _ => return Err(self.err_at(pos, "expected a constraint name")),
            };
            let severity = match self.advance()? {
                Tok::Name(s) if s == "warn" => Severity::Warn,
                Tok::Name(s) if s == "error" => Severity::Error,
                _ => return Err(self.err("expected `warn` or `error`")),
            };
            self.expect(":-")?;
            let body = self.body()?;
            self.expect(".")?;
            return Ok(Some(Item::Constraint {
                name,
                severity,
                body,
                pos,
            }));
        }
        let head = self.head()?;
        let body = if self.is(":-") {
            self.advance()?;
            self.body()?
        } else {
            Vec::new()
        };
        self.expect(".")?;
        Ok(Some(Item::Rule { head, body, pos }))
    }

    fn head(&mut self) -> Result<Head> {
        let mut exists = Vec::new();
        if self.is_name("exists") {
            self.advance()?;
            loop {
                match self.advance()? {
                    Tok::Var(v) if v != "_" => exists.push(v),
                    _ => return Err(self.err("expected a variable after `exists`")),
                }
                if self.is(",") {
                    self.advance()?;
                } else {
                    break;
                }
            }
            self.expect(":")?;
        }
        let op = self.temp_op()?;
        let atom = self.atom()?;
        Ok(Head { exists, op, atom })
    }

    fn temp_op(&mut self) -> Result<Option<(TempOp, Window)>> {
        let op = match &self.tok {
            Tok::Name(n) => match n.as_str() {
                "past" => TempOp::Past,
                "always_past" => TempOp::AlwaysPast,
                "future" => TempOp::Future,
                "always_future" => TempOp::AlwaysFuture,
                _ => return Ok(None),
            },
            _ => return Ok(None),
        };
        // `past(` would be an atom named past; operators need `[`
        let save_is_bracket = self.lx.src[self.lx.i..]
            .iter()
            .find(|b| !b.is_ascii_whitespace())
            .copied()
            == Some(b'[');
        if !save_is_bracket {
            return Ok(None);
        }
        self.advance()?;
        let w = self.window()?;
        Ok(Some((op, w)))
    }

    fn window(&mut self) -> Result<Window> {
        let pos = self.pos;
        self.expect("[")?;
        let lo = self
            .duration()?
            .ok_or_else(|| self.err("`inf` cannot start a window"))?;
        self.expect(",")?;
        let hi = self.duration()?;
        self.expect("]")?;
        if lo < 0 || hi.is_some_and(|h| h < lo) {
            return Err(self.err_at(pos, "a window needs 0 ≤ lo ≤ hi"));
        }
        Ok(Window { lo, hi })
    }

    /// A duration, a plain number of milliseconds, or `inf` (`None`).
    fn duration(&mut self) -> Result<Option<i64>> {
        match self.advance()? {
            Tok::Dur(ms) => Ok(Some(ms)),
            Tok::Num(n) => n
                .parse::<i64>()
                .map(Some)
                .map_err(|_| self.err("a window bound must be an integer")),
            Tok::Name(n) if n == "inf" => Ok(None),
            _ => Err(self.err("expected a duration or `inf`")),
        }
    }

    fn body(&mut self) -> Result<Vec<Literal>> {
        let mut out = vec![self.literal()?];
        while self.is(",") {
            self.advance()?;
            out.push(self.literal()?);
        }
        Ok(out)
    }

    fn tatom(&mut self) -> Result<TAtom> {
        let mut ops = Vec::new();
        while let Some(op) = self.temp_op()? {
            ops.push(op);
        }
        let atom = self.atom()?;
        Ok(TAtom { ops, atom })
    }

    fn literal(&mut self) -> Result<Literal> {
        let pos = self.pos;
        if self.is_name("not") || self.is("!") {
            self.advance()?;
            return Ok(Literal::Neg(self.tatom()?));
        }
        // aggregate: `R = count : { ... }`
        if let Tok::Var(v) = &self.tok {
            let v = v.clone();
            let rest = &self.lx.src[self.lx.i..];
            let trimmed: Vec<u8> = rest
                .iter()
                .copied()
                .skip_while(|b| b.is_ascii_whitespace())
                .collect();
            if trimmed.first() == Some(&b'=') && trimmed.get(1) != Some(&b'=') {
                let after: String = String::from_utf8_lossy(&trimmed[1..])
                    .trim_start()
                    .chars()
                    .take_while(|c| c.is_ascii_alphabetic())
                    .collect();
                if AGGS.contains(&after.as_str()) {
                    self.advance()?; // var
                    self.expect("=")?;
                    let func = match self.advance()? {
                        Tok::Name(n) => match n.as_str() {
                            "count" => AggFn::Count,
                            "sum" => AggFn::Sum,
                            "min" => AggFn::Min,
                            _ => AggFn::Max,
                        },
                        _ => unreachable!("checked above"),
                    };
                    let over = if func == AggFn::Count {
                        None
                    } else {
                        Some(self.expr()?)
                    };
                    self.expect(":")?;
                    self.expect("{")?;
                    let body = self.body()?;
                    self.expect("}")?;
                    return Ok(Literal::Agg {
                        result: v,
                        func,
                        over,
                        body,
                        pos,
                    });
                }
            }
        }
        // an atom, or a comparison
        let starts_atom = matches!(&self.tok, Tok::PName(..) | Tok::Iri(_))
            || matches!(&self.tok, Tok::Name(n) if !matches!(n.as_str(), "true" | "false" | "inf"));
        if starts_atom {
            let lhs = self.tatom()?;
            if self.is_name("since") || self.is_name("until") {
                let until = self.is_name("until");
                self.advance()?;
                let window = self.window()?;
                let rhs = self.tatom()?;
                return Ok(Literal::Binary {
                    lhs,
                    until,
                    window,
                    rhs,
                });
            }
            if lhs.ops.is_empty() && self.cmp_op().is_some() || self.arith_follows() {
                // the atom was a function call inside a comparison
                let call = atom_to_call(lhs.atom, self)?;
                let l = self.expr_rest(call)?;
                let op = self
                    .cmp_op()
                    .ok_or_else(|| self.err("expected a comparison"))?;
                self.advance()?;
                let r = self.expr()?;
                return Ok(Literal::Cmp(l, op, r, pos));
            }
            return Ok(Literal::Pos(lhs));
        }
        let l = self.expr()?;
        let op = self
            .cmp_op()
            .ok_or_else(|| self.err(format!("expected a comparison, found {}", self.describe())))?;
        self.advance()?;
        let r = self.expr()?;
        Ok(Literal::Cmp(l, op, r, pos))
    }

    fn arith_follows(&self) -> bool {
        ["+", "-", "*", "/", "%"].iter().any(|s| self.is(s))
    }

    fn cmp_op(&self) -> Option<CmpOp> {
        match &self.tok {
            Tok::Sym("=") => Some(CmpOp::Eq),
            Tok::Sym("!=") => Some(CmpOp::Ne),
            Tok::Sym("<") => Some(CmpOp::Lt),
            Tok::Sym("<=") => Some(CmpOp::Le),
            Tok::Sym(">") => Some(CmpOp::Gt),
            Tok::Sym(">=") => Some(CmpOp::Ge),
            _ => None,
        }
    }

    fn atom(&mut self) -> Result<Atom> {
        let pos = self.pos;
        let t = self.advance()?;
        let pred = match &t {
            Tok::Name(n) => Pred::Name(n.clone()),
            Tok::PName(..) | Tok::Iri(_) => Pred::Iri(self.iri_of(&t, pos)?),
            _ => return Err(self.err_at(pos, "expected a predicate")),
        };
        self.expect("(")?;
        let mut args = Vec::new();
        if !self.is(")") {
            args.push(self.expr()?);
            while self.is(",") {
                self.advance()?;
                args.push(self.expr()?);
            }
        }
        self.expect(")")?;
        let eid = if self.is("@") {
            self.advance()?;
            Some(self.primary()?)
        } else {
            None
        };
        Ok(Atom {
            pred,
            args,
            eid,
            pos,
        })
    }

    fn expr(&mut self) -> Result<Expr> {
        let first = self.term_expr()?;
        self.expr_rest(first)
    }

    /// Continues an additive expression whose first operand is parsed.
    fn expr_rest(&mut self, first: Expr) -> Result<Expr> {
        let mut l = self.mul_rest(first)?;
        loop {
            let op = if self.is("+") {
                BinOp::Add
            } else if self.is("-") {
                BinOp::Sub
            } else {
                return Ok(l);
            };
            self.advance()?;
            let r = self.term_expr()?;
            let r = self.mul_rest(r)?;
            l = Expr::Bin(Box::new(l), op, Box::new(r));
        }
    }

    fn mul_rest(&mut self, mut l: Expr) -> Result<Expr> {
        loop {
            let op = if self.is("*") {
                BinOp::Mul
            } else if self.is("/") {
                BinOp::Div
            } else if self.is("%") {
                BinOp::Rem
            } else {
                return Ok(l);
            };
            self.advance()?;
            let r = self.term_expr()?;
            l = Expr::Bin(Box::new(l), op, Box::new(r));
        }
    }

    fn term_expr(&mut self) -> Result<Expr> {
        if self.is("-") {
            self.advance()?;
            let inner = self.term_expr()?;
            return Ok(match inner {
                Expr::Const(Value::Int(i)) => Expr::Const(Value::Int(-i)),
                Expr::Const(Value::Double(d)) => Expr::Const(Value::Double(-d)),
                Expr::Const(Value::Decimal(d)) => Expr::Const(Value::Decimal(format!("-{d}"))),
                e => Expr::Bin(
                    Box::new(Expr::Const(Value::Int(0))),
                    BinOp::Sub,
                    Box::new(e),
                ),
            });
        }
        if self.is("(") {
            self.advance()?;
            let e = self.expr()?;
            self.expect(")")?;
            return Ok(e);
        }
        if let Tok::Name(n) = &self.tok {
            if !matches!(n.as_str(), "true" | "false" | "inf") {
                let name = n.clone();
                self.advance()?;
                self.expect("(")?;
                let mut args = Vec::new();
                if !self.is(")") {
                    args.push(self.expr()?);
                    while self.is(",") {
                        self.advance()?;
                        args.push(self.expr()?);
                    }
                }
                self.expect(")")?;
                return Ok(Expr::Call(name, args));
            }
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<Expr> {
        let pos = self.pos;
        let t = self.advance()?;
        Ok(match t {
            Tok::Var(v) if v == "_" => Expr::Anon,
            Tok::Var(v) => Expr::Var(v),
            Tok::Name(n) if n == "true" => Expr::Const(Value::Bool(true)),
            Tok::Name(n) if n == "false" => Expr::Const(Value::Bool(false)),
            Tok::Name(n) if n == "inf" => Expr::Inf,
            Tok::PName(..) | Tok::Iri(_) => Expr::Const(Value::Iri(self.iri_of(&t, pos)?)),
            Tok::Num(n) => Expr::Const(number(&n)),
            Tok::Dur(ms) => Expr::Const(Value::Int(ms)),
            Tok::Str(s, lang, dt) => Expr::Const(match (lang, dt) {
                (Some(l), _) => Value::literal(&s, None, Some(&l)),
                (None, Some(dt)) => {
                    let dt = self.iri_of(&dt, pos)?;
                    Value::literal(&s, Some(&dt), None)
                }
                (None, None) => Value::Str(s),
            }),
            _ => return Err(self.err_at(pos, format!("expected a term, found {}", describe(&t)))),
        })
    }
}

fn describe(t: &Tok) -> String {
    match t {
        Tok::Eof => "end of input".into(),
        Tok::Sym(s) => format!("`{s}`"),
        other => format!("{other:?}"),
    }
}

fn number(lex: &str) -> Value {
    if lex.contains(['e', 'E']) {
        Value::literal(lex, Some(tm_core::vocab::XSD_DOUBLE), None)
    } else if lex.contains('.') {
        Value::literal(lex, Some(tm_core::vocab::XSD_DECIMAL), None)
    } else {
        Value::literal(lex, Some(tm_core::vocab::XSD_INTEGER), None)
    }
}

fn atom_to_call(a: Atom, p: &Parser<'_>) -> Result<Expr> {
    match (a.pred, a.eid) {
        (Pred::Name(n), None) => Ok(Expr::Call(n, a.args)),
        _ => Err(p.err_at(a.pos, "expected a function call in a comparison")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_reporting_program() {
        let ast = parse(
            "@prefix ex: <http://example.org/> .\n\
             reports(X, Y) :- ex:manager(X, Y).\n\
             reports(X, Z) :- reports(X, Y), ex:manager(Y, Z).\n\
             ?- reports(ex:alice, X).\n\
             constraint no_cycle error :- reports(X, X).\n",
        )
        .unwrap();
        assert_eq!(ast.items.len(), 4);
        match &ast.items[3] {
            Item::Constraint { name, severity, .. } => {
                assert_eq!(name, "no_cycle");
                assert_eq!(*severity, Severity::Error);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn missing_period_reports_end() {
        let e = parse("reports(X, Y) :- v:manager(X, Y)").unwrap_err();
        match e {
            Error::DatalogParse { line, column, .. } => {
                assert_eq!(line, 1);
                assert_eq!(column, 33);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn operators_aggregates_and_since() {
        let ast = parse(
            "alert(X) :- past[0d, 3d] ex:fever(X, true).\n\
             always_future[0d, 7d] blocked(R) :- ex:booked(R, true).\n\
             h(C, N) :- rdf:type(C, ex:Company), N = count : { ex:employs(C, _) }.\n\
             p(S) :- ex:unacked(S, true) since[0, inf] ex:alarm(S, true).\n\
             exists M: ex:manager(X, M) :- rdf:type(X, ex:Employee), not ex:manager(X, _).\n\
             d(T, E) :- ex:due(T, D), E = D + 30d.\n\
             n(Y) :- n(X), Y = X + 1, strlen(\"abc\") > 2.\n"
                .replace("ex:", "v:")
                .as_str(),
        )
        .unwrap();
        assert_eq!(ast.items.len(), 7);
    }
}
