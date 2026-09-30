//! Printers: the canonical S-expression form of an IR (for snapshots and
//! debugging), and the property-path text of a [`PathExpr`] (the `path` argument
//! of `tm_path`, whose grammar `add-path-engine` owns), with its parser.

use std::fmt::{self, Write as _};

use tm_core::{ValidSel, Value};

use crate::agg::{Agg, Key};
use crate::expr::{Expr, LookupMode};
use crate::op::{IrQuery, Op};
use crate::path::PathExpr;
use crate::term::TermOrVar;
use crate::view::{TimeRef, TxSel, View};

/// Namespaces printed as CURIEs in path text.
pub const PATH_PREFIXES: [(&str, &str); 5] = [
    ("sys", "urn:tiramemsu:sys:"),
    ("tm", "urn:tiramemsu:tm:"),
    ("v", "urn:tiramemsu:v:"),
    ("xsd", "http://www.w3.org/2001/XMLSchema#"),
    ("rdf", "http://www.w3.org/1999/02/22-rdf-syntax-ns#"),
];

fn view_text(v: &View) -> String {
    let tx = match v.tx {
        TxSel::Now => "now".to_string(),
        TxSel::AsOf(TimeRef::Tx(t)) => format!("asOf/tx:{t}"),
        TxSel::AsOf(TimeRef::Instant(ms)) => format!("asOf/instant:{ms}"),
        TxSel::History => "history".to_string(),
    };
    match v.valid {
        ValidSel::Unfiltered => tx,
        ValidSel::At(d) => format!("{tx};validAt/{d}"),
    }
}

/// Compact text of a constant: numbers and booleans bare, others as in `Value`'s
/// `Display`.
fn constant(c: &Value) -> String {
    match c {
        Value::Int(i) => i.to_string(),
        Value::Bool(b) => b.to_string(),
        other => other.to_string(),
    }
}

fn term(t: &TermOrVar) -> String {
    match t {
        TermOrVar::Var(v) => v.to_string(),
        TermOrVar::Const(c) => constant(c),
        TermOrVar::Id(id) => format!("#{id:?}"),
        TermOrVar::Param(p) => format!("${p}"),
    }
}

/// The canonical one-line S-expression of an expression.
pub fn expr_sexpr(e: &Expr) -> String {
    let list = |head: &str, xs: &[Expr]| {
        let mut s = format!("({head}");
        for x in xs {
            s.push(' ');
            s.push_str(&expr_sexpr(x));
        }
        s.push(')');
        s
    };
    match e {
        Expr::Var(v) => v.to_string(),
        Expr::Const(c) => constant(c),
        Expr::Param(p) => format!("${p}"),
        Expr::Cmp(op, a, b) => format!("({} {} {})", op.symbol(), expr_sexpr(a), expr_sexpr(b)),
        Expr::SameTerm(a, b) => format!("(sameTerm {} {})", expr_sexpr(a), expr_sexpr(b)),
        Expr::And(xs) => list("and", xs),
        Expr::Or(xs) => list("or", xs),
        Expr::Not(a) => format!("(not {})", expr_sexpr(a)),
        Expr::Bound(v) => format!("(bound {v})"),
        Expr::In(a, xs, neg) => {
            let head = if *neg { "notIn" } else { "in" };
            let mut s = format!("({head} {}", expr_sexpr(a));
            for x in xs {
                s.push(' ');
                s.push_str(&expr_sexpr(x));
            }
            s.push(')');
            s
        }
        Expr::Arith(op, a, b) => format!("({} {} {})", op.symbol(), expr_sexpr(a), expr_sexpr(b)),
        Expr::Neg(a) => format!("(neg {})", expr_sexpr(a)),
        Expr::Coalesce(xs) => list("coalesce", xs),
        Expr::If(a, b, c) => format!("(if {} {} {})", expr_sexpr(a), expr_sexpr(b), expr_sexpr(c)),
        Expr::Func(f, xs) => list(f.name(), xs),
        Expr::Exists(op, neg) => {
            let head = if *neg { "notExists" } else { "exists" };
            format!("({head} {})", sexpr_line(op))
        }
        Expr::Lookup(l) => format!(
            "(lookup {} {} :view {}{}{})",
            expr_sexpr(&l.subject),
            term(&l.pred),
            view_text(&l.view),
            if l.multi == LookupMode::ListIfMany {
                " :list"
            } else {
                ""
            },
            if l.include_volatile { " :volatile" } else { "" }
        ),
        Expr::List(xs) => list("list", xs),
    }
}

fn key(k: &Key) -> String {
    if k.descending {
        format!("(desc {})", expr_sexpr(&k.expr))
    } else {
        format!("(asc {})", expr_sexpr(&k.expr))
    }
}

fn agg(a: &Agg) -> String {
    let arg = a.arg.as_ref().map_or("*".to_string(), expr_sexpr);
    let d = if a.distinct { "distinct " } else { "" };
    let sep = match &a.func {
        crate::agg::AggFunc::GroupConcat { sep } => format!(" :sep {sep:?}"),
        _ => String::new(),
    };
    format!("({} {} {d}{arg}{sep})", a.var, a.func.name())
}

fn vars(vs: &[crate::var::Var]) -> String {
    let v: Vec<String> = vs.iter().map(|v| v.to_string()).collect();
    format!("[{}]", v.join(" "))
}

/// The ` :graph …` suffix of a pattern's graph selector (nothing for `Any`).
fn graph_text(g: &crate::op::GraphSel, out: &mut String) {
    match g {
        crate::op::GraphSel::Any => {}
        crate::op::GraphSel::Set(gs) => {
            let gs: Vec<String> = gs.iter().map(term).collect();
            let _ = write!(out, " :graph ({})", gs.join(" "));
        }
        crate::op::GraphSel::Var(v) => {
            let _ = write!(out, " :graph {v}");
        }
    }
}

fn write_op(out: &mut String, op: &Op, depth: usize, pretty: bool) {
    let nl = |out: &mut String, d: usize| {
        if pretty {
            out.push('\n');
            for _ in 0..d {
                out.push_str("  ");
            }
        } else {
            out.push(' ');
        }
    };
    match op {
        Op::Triple(t) => {
            let _ = write!(
                out,
                "(triple {} {} {} :view {}",
                term(&t.s),
                term(&t.p),
                term(&t.o),
                view_text(&t.view)
            );
            if let Some(e) = &t.eid {
                let _ = write!(out, " :eid {e}");
            }
            if let Some(g) = t.iso_group {
                let _ = write!(out, " :group {g}");
            }
            if t.include_volatile {
                out.push_str(" :volatile");
            }
            graph_text(&t.graph, out);
            out.push(')');
        }
        Op::Path(p) => {
            let _ = write!(
                out,
                "(path {} {} {:?} :mode {} :view {}",
                term(&p.start),
                term(&p.end),
                path_text(&p.path),
                p.mode.sql_name(),
                view_text(&p.view)
            );
            if let Some(m) = p.max_hops {
                let _ = write!(out, " :maxHops {m}");
            }
            if let Some(b) = &p.bind_path {
                let _ = write!(out, " :bind {b}");
            }
            graph_text(&p.graph, out);
            out.push(')');
        }
        Op::Values(v) => {
            let _ = write!(out, "(values {}", vars(&v.vars));
            for r in &v.rows {
                let cells: Vec<String> = r
                    .iter()
                    .map(|c| c.as_ref().map_or("UNDEF".to_string(), term))
                    .collect();
                let _ = write!(out, " ({})", cells.join(" "));
            }
            out.push(')');
        }
        Op::Unnest(u) => {
            let _ = write!(out, "(unnest {} {}", u.var, expr_sexpr(&u.list));
            nl(out, depth + 1);
            write_op(out, &u.input, depth + 1, pretty);
            out.push(')');
        }
        Op::Join(j) => {
            out.push_str("(join");
            if !j.null_safe.is_empty() {
                let _ = write!(out, " :nullSafe {}", vars(&j.null_safe));
            }
            for i in &j.inputs {
                nl(out, depth + 1);
                write_op(out, i, depth + 1, pretty);
            }
            out.push(')');
        }
        Op::LeftJoin(l) => {
            out.push_str("(leftJoin");
            if let Some(c) = &l.cond {
                let _ = write!(out, " :cond {}", expr_sexpr(c));
            }
            nl(out, depth + 1);
            write_op(out, &l.left, depth + 1, pretty);
            nl(out, depth + 1);
            write_op(out, &l.right, depth + 1, pretty);
            out.push(')');
        }
        Op::Filter(f) => {
            let _ = write!(out, "(filter {}", expr_sexpr(&f.cond));
            nl(out, depth + 1);
            write_op(out, &f.input, depth + 1, pretty);
            out.push(')');
        }
        Op::Union(u) => {
            out.push_str("(union");
            for i in &u.inputs {
                nl(out, depth + 1);
                write_op(out, i, depth + 1, pretty);
            }
            out.push(')');
        }
        Op::Extend(e) => {
            let _ = write!(out, "(extend {} {}", e.var, expr_sexpr(&e.expr));
            nl(out, depth + 1);
            write_op(out, &e.input, depth + 1, pretty);
            out.push(')');
        }
        Op::Aggregate(a) => {
            let aggs: Vec<String> = a.aggs.iter().map(agg).collect();
            let _ = write!(out, "(aggregate {} [{}]", vars(&a.group), aggs.join(" "));
            nl(out, depth + 1);
            write_op(out, &a.input, depth + 1, pretty);
            out.push(')');
        }
        Op::Project(p) => {
            let head = if p.distinct {
                "projectDistinct"
            } else {
                "project"
            };
            let _ = write!(out, "({head} {}", vars(&p.vars));
            nl(out, depth + 1);
            write_op(out, &p.input, depth + 1, pretty);
            out.push(')');
        }
        Op::OrderLimit(o) => {
            let keys: Vec<String> = o.keys.iter().map(key).collect();
            let _ = write!(out, "(orderLimit [{}]", keys.join(" "));
            if let Some(s) = &o.skip {
                let _ = write!(out, " :skip {}", term(s));
            }
            if let Some(l) = &o.limit {
                let _ = write!(out, " :limit {}", term(l));
            }
            nl(out, depth + 1);
            write_op(out, &o.input, depth + 1, pretty);
            out.push(')');
        }
        Op::RowNumber(r) => {
            let keys: Vec<String> = r.order.iter().map(key).collect();
            let _ = write!(
                out,
                "(rowNumber {} :partition {} :order [{}]",
                r.var,
                vars(&r.partition),
                keys.join(" ")
            );
            nl(out, depth + 1);
            write_op(out, &r.input, depth + 1, pretty);
            out.push(')');
        }
    }
}

/// The canonical, indented S-expression of an operator tree.
pub fn sexpr(op: &Op) -> String {
    let mut s = String::new();
    write_op(&mut s, op, 0, true);
    s
}

/// The canonical S-expression on one line.
pub fn sexpr_line(op: &Op) -> String {
    let mut s = String::new();
    write_op(&mut s, op, 0, false);
    s
}

impl fmt::Display for IrQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = &self.semantics;
        writeln!(f, "; {:?} {:?} {:?}", s.match_mode, s.missing, s.graph_set)?;
        f.write_str(&sexpr(&self.root))
    }
}

impl fmt::Display for Op {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&sexpr(self))
    }
}

// ---------------------------------------------------------------------------
// Path text

fn atom(v: &Value, canon: bool) -> String {
    match v {
        Value::Iri(iri) => {
            for (p, ns) in PATH_PREFIXES {
                // `v:` follows the database `@vocab` when parsed, so the canonical
                // text spells those IRIs out
                if canon && p == "v" {
                    continue;
                }
                if let Some(local) = iri.strip_prefix(ns) {
                    if !local.is_empty()
                        && local
                            .chars()
                            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
                    {
                        return format!("{p}:{local}");
                    }
                }
            }
            format!("<{iri}>")
        }
        other => format!("<{}>", other.lexical()),
    }
}

fn prec(p: &PathExpr) -> u8 {
    match p {
        PathExpr::Alt(_) => 0,
        PathExpr::Seq(_) => 1,
        PathExpr::Inverse(_) => 2,
        PathExpr::ZeroOrMore(_)
        | PathExpr::OneOrMore(_)
        | PathExpr::ZeroOrOne(_)
        | PathExpr::Repeat { .. } => 3,
        PathExpr::Pred(_) => 4,
    }
}

fn wrap(p: &PathExpr, min: u8, canon: bool) -> String {
    let s = text(p, canon);
    if prec(p) < min {
        format!("({s})")
    } else {
        s
    }
}

/// Prints a path in the `tm_path` path grammar: SPARQL 1.1 property-path syntax
/// plus `{m,n}` / `{m,}` / `{n}`. Atoms are `<iri>` or CURIEs of
/// [`PATH_PREFIXES`] (`sys:anyRelationship` is the Cypher wildcard).
pub fn path_text(p: &PathExpr) -> String {
    text(p, false)
}

/// The canonical `tm_path` text: like [`path_text`], but every IRI outside the
/// fixed `sys:`, `tm:`, `rdf:` and `xsd:` namespaces is written in full, so it
/// means the same whatever `@vocab` the database has.
pub fn path_text_canonical(p: &PathExpr) -> String {
    text(p, true)
}

fn text(p: &PathExpr, canon: bool) -> String {
    match p {
        PathExpr::Pred(v) => atom(v, canon),
        PathExpr::Inverse(x) => {
            // `^` applies to a path element (a primary with an optional modifier)
            format!("^{}", wrap(x, 3, canon))
        }
        PathExpr::Seq(xs) => xs
            .iter()
            .map(|x| wrap(x, 2, canon))
            .collect::<Vec<_>>()
            .join("/"),
        PathExpr::Alt(xs) => xs
            .iter()
            .map(|x| wrap(x, 1, canon))
            .collect::<Vec<_>>()
            .join("|"),
        PathExpr::ZeroOrMore(x) => format!("{}*", wrap(x, 4, canon)),
        PathExpr::OneOrMore(x) => format!("{}+", wrap(x, 4, canon)),
        PathExpr::ZeroOrOne(x) => format!("{}?", wrap(x, 4, canon)),
        PathExpr::Repeat { inner, min, max } => {
            let m = match max {
                Some(n) if n == min => format!("{{{min}}}"),
                Some(n) => format!("{{{min},{n}}}"),
                None => format!("{{{min},}}"),
            };
            format!("{}{m}", wrap(inner, 4, canon))
        }
    }
}

/// Parses path text printed by [`path_text`] (and ordinary SPARQL 1.1 paths over
/// IRIs and the CURIEs of [`PATH_PREFIXES`]).
pub fn parse_path(text: &str) -> Result<PathExpr, String> {
    let mut p = PathParser {
        s: text.as_bytes(),
        i: 0,
        text,
    };
    let e = p.alt()?;
    p.ws();
    if p.i != p.s.len() {
        return Err(format!("unexpected input at {}", p.i));
    }
    Ok(e)
}

struct PathParser<'a> {
    s: &'a [u8],
    i: usize,
    text: &'a str,
}

impl PathParser<'_> {
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

    fn alt(&mut self) -> Result<PathExpr, String> {
        let mut xs = vec![self.seq()?];
        while self.eat(b'|') {
            xs.push(self.seq()?);
        }
        Ok(if xs.len() == 1 {
            xs.pop().expect("one")
        } else {
            PathExpr::Alt(xs)
        })
    }

    fn seq(&mut self) -> Result<PathExpr, String> {
        let mut xs = vec![self.elt_or_inverse()?];
        while self.eat(b'/') {
            xs.push(self.elt_or_inverse()?);
        }
        Ok(if xs.len() == 1 {
            xs.pop().expect("one")
        } else {
            PathExpr::Seq(xs)
        })
    }

    fn elt_or_inverse(&mut self) -> Result<PathExpr, String> {
        if self.eat(b'^') {
            Ok(PathExpr::Inverse(Box::new(self.elt()?)))
        } else {
            self.elt()
        }
    }

    fn elt(&mut self) -> Result<PathExpr, String> {
        let mut p = self.primary()?;
        loop {
            // modifiers bind tightly: no whitespace skipping before them
            match self.s.get(self.i) {
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
                        return Err(format!("expected '}}' at {}", self.i));
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

    fn num(&mut self) -> Result<u32, String> {
        self.ws();
        let st = self.i;
        while self.i < self.s.len() && self.s[self.i].is_ascii_digit() {
            self.i += 1;
        }
        self.text[st..self.i]
            .parse()
            .map_err(|_| format!("expected a number at {st}"))
    }

    fn primary(&mut self) -> Result<PathExpr, String> {
        match self.peek() {
            Some(b'(') => {
                self.i += 1;
                let e = self.alt()?;
                if !self.eat(b')') {
                    return Err(format!("expected ')' at {}", self.i));
                }
                Ok(e)
            }
            Some(b'<') => {
                let st = self.i + 1;
                let end = self.text[st..]
                    .find('>')
                    .ok_or_else(|| "unterminated IRI".to_string())?;
                self.i = st + end + 1;
                Ok(PathExpr::iri(&self.text[st..st + end]))
            }
            Some(_) => {
                let st = self.i;
                while self.i < self.s.len() {
                    let c = self.s[self.i];
                    if c.is_ascii_alphanumeric() || c == b':' || c == b'_' || c == b'-' || c >= 0x80
                    {
                        self.i += 1;
                    } else {
                        break;
                    }
                }
                let curie = &self.text[st..self.i];
                let (p, local) = curie
                    .split_once(':')
                    .ok_or_else(|| format!("expected an IRI or CURIE at {st}"))?;
                let ns = PATH_PREFIXES
                    .iter()
                    .find(|(k, _)| *k == p)
                    .ok_or_else(|| format!("unknown prefix {p:?}"))?
                    .1;
                Ok(PathExpr::iri(format!("{ns}{local}")))
            }
            None => Err("unexpected end of path".to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agg::{Agg, AggFunc, Key};
    use crate::expr::{CmpOp, Expr};
    use crate::op::TriplePattern;
    use crate::path::{PathExpr as P, PathMode};
    use crate::view::View;

    fn v(l: &str) -> String {
        format!("urn:tiramemsu:v:{l}")
    }

    #[test]
    fn sexpr_snapshot_corpus() {
        let t = |s: &str, p: &str, o: &str| {
            Op::Triple(TriplePattern::new(s, v(p).as_str(), o, View::now()))
        };
        let corpus = [
            Op::join(vec![t("?a", "worksAt", "?c"), t("?c", "name", "?n")]),
            Op::left_join(
                t("?p", "name", "?n"),
                Op::Triple(
                    TriplePattern::new("?p", v("age").as_str(), "?a", View::as_of_tx(150))
                        .with_eid("?r"),
                ),
                Some(Expr::gt(Expr::var("a"), Expr::val(Value::Int(30)))),
            ),
            Op::union(vec![t("?x", "p", "?y"), t("?x", "q", "$param")])
                .filter(Expr::Or(vec![
                    Expr::Bound("y".into()),
                    Expr::not_exists(t("?x", "r", "?z")),
                ]))
                .aggregate(
                    &["x"],
                    vec![
                        Agg::count_star("n"),
                        Agg::new("c", AggFunc::Collect, Expr::var("y")).distinct(),
                        Agg::new(
                            "g",
                            AggFunc::GroupConcat {
                                sep: ", ".to_string(),
                            },
                            Expr::var("y"),
                        ),
                    ],
                )
                .order_limit(vec![Key::desc(Expr::var("n"))], Some(2), Some(10))
                .project_distinct(&["x", "n"]),
            Op::Path(crate::op::PathPattern {
                start: "?r".into(),
                end: "?x".into(),
                path: P::Seq(vec![
                    P::iri(v("supportedBy")),
                    P::iri(crate::vocab::SYS_SUBJECT).star(),
                ]),
                mode: PathMode::Trail,
                max_hops: Some(15),
                bind_path: Some("p".into()),
                view: View::history().valid_at(5),
                graph: crate::op::GraphSel::Any,
            }),
            Op::unit().extend(
                "x",
                Expr::cmp(CmpOp::Le, Expr::val(Value::Int(1)), Expr::Param("k".into())),
            ),
        ];
        let text: Vec<String> = corpus.iter().map(sexpr).collect();
        insta::assert_snapshot!(text.join("\n\n"));
    }

    #[test]
    fn path_text_round_trips() {
        let sub = || P::iri(crate::vocab::SYS_SUBJECT);
        let samples = vec![
            P::iri(v("knows")),
            P::Seq(vec![P::iri(v("supportedBy")), sub()]),
            P::Seq(vec![P::iri(v("supportedBy")), sub().star()]),
            P::Alt(vec![P::iri(v("a")), P::iri(v("b")).inverse()]),
            P::Seq(vec![
                P::Alt(vec![P::iri(v("a")), P::iri(v("b"))]),
                P::iri(v("c")),
            ])
            .plus(),
            P::Inverse(Box::new(P::Seq(vec![P::iri(v("a")), P::iri(v("b"))]))),
            P::Repeat {
                inner: Box::new(P::iri(crate::vocab::SYS_ANY_RELATIONSHIP)),
                min: 1,
                max: Some(3),
            },
            P::Repeat {
                inner: Box::new(P::iri("http://ex/p")),
                min: 2,
                max: None,
            },
            P::Repeat {
                inner: Box::new(P::iri(v("x"))),
                min: 4,
                max: Some(4),
            },
            P::ZeroOrOne(Box::new(P::iri(v("x")).inverse())),
        ];
        let texts: Vec<String> = samples.iter().map(path_text).collect();
        assert_eq!(texts[1], "v:supportedBy/sys:subject");
        assert_eq!(texts[2], "v:supportedBy/sys:subject*");
        assert_eq!(texts[3], "v:a|^v:b");
        assert_eq!(texts[4], "((v:a|v:b)/v:c)+");
        assert_eq!(texts[5], "^(v:a/v:b)");
        assert_eq!(texts[6], "sys:anyRelationship{1,3}");
        assert_eq!(texts[7], "<http://ex/p>{2,}");
        assert_eq!(texts[8], "v:x{4}");
        assert_eq!(texts[9], "(^v:x)?");
        for (p, t) in samples.iter().zip(&texts) {
            assert_eq!(&parse_path(t).unwrap(), p, "{t}");
        }
        assert!(parse_path("foo:bar").is_err());
        assert!(parse_path("(v:a").is_err());
    }
}
