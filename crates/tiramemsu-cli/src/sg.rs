//! `tm export --format sg-nt12`: memory as a statement graph in RDF 1.2, the format
//! LeanRDF's `leanrdf-statements` defines and proves exact (its `toRDF`, with
//! `decode_toRDF` and `toRDF_decode`). Every statement, live or not, becomes a reifier
//! `stmt:e rdf:reifies <<( s p o )>>` with its clocks, and its base triple `s p o` is
//! asserted when it is live in the chosen view. The output is canonical RDF 1.2
//! N-Triples in LeanRDF's triple order, so `leanrdf-sg export` of the decoded records
//! reproduces it byte for byte.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::io::Write;

use tiramemsu::rdf::{render, RdfTerm};
use tiramemsu::vocab;
use tiramemsu::*;

use crate::io;

/// The transaction part of an sg-nt12 view.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SgTx {
    /// Not retracted.
    Now,
    /// Added at or before `t` and not retracted at or before `t`.
    AsOf(u64),
    /// Every statement.
    History,
}

/// Which statements the export asserts: a transaction selector and an optional
/// valid-time point (half-open intervals, epoch ms).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct SgView {
    pub tx: SgTx,
    pub valid_at: Option<i64>,
}

/// What an sg-nt12 export wrote.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SgReport {
    /// Statements written as reifiers.
    pub records: u64,
    /// Of those, the ones live in the view (their base triple is asserted).
    pub live: u64,
    /// Statements added and retracted in one transaction: never visible, left out.
    pub never_visible: u64,
}

/// An RDF term, ordered like LeanRDF's `Term` (`iri`, `bnode`, `lit`, `triple`, then
/// their fields), so that sorting gives LeanRDF's canonical triple order. Variant and
/// field order matter: do not reorder.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Term {
    Iri(String),
    Blank(String),
    Lit(Lit),
    Triple(Box<Triple>),
}

/// A literal, ordered like LeanRDF's `Literal` (`typed lex dt`, then `lang lex tag`).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Lit {
    Typed(String, String),
    Lang(String, String),
}

/// A triple, ordered like LeanRDF's `Triple` (`s`, `p`, `o`).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Triple(Term, Term, Term);

fn iri(s: &str) -> Term {
    Term::Iri(s.to_string())
}

fn integer(n: impl ToString) -> Term {
    Term::Lit(Lit::Typed(n.to_string(), vocab::XSD_INTEGER.to_string()))
}

fn term(t: RdfTerm) -> Term {
    match t {
        RdfTerm::Iri(s) => Term::Iri(s),
        RdfTerm::Blank(b) => Term::Blank(b),
        RdfTerm::Literal {
            lex, lang: Some(l), ..
        } => Term::Lit(Lit::Lang(lex, l)),
        RdfTerm::Literal { lex, datatype, .. } => Term::Lit(Lit::Typed(
            lex,
            datatype.unwrap_or_else(|| vocab::XSD_STRING.to_string()),
        )),
        RdfTerm::Triple(t) => {
            let t = *t;
            Term::Triple(Box::new(Triple(term(t.s), term(t.p), term(t.o))))
        }
    }
}

/// True for the characters an IRI cannot hold in N-Triples without `UCHAR`, which the
/// canonical form does not use.
fn unwritable(c: char) -> bool {
    matches!(
        c,
        '<' | '>' | '"' | '{' | '}' | '|' | '^' | '`' | '\\' | ' '
    ) || (c as u32) <= 0x20
}

/// A string literal in canonical form (RDF 1.2 N-Triples §3): `ECHAR` for `"`, `\`,
/// BACKSPACE, TAB, LF, FORM FEED and CR, `\uXXXX` for the other characters below
/// U+0020 and for U+007F, U+FFFE and U+FFFF. The same rule as LeanRDF's serializer.
fn quote(lex: &str, out: &mut String) {
    out.push('"');
    for c in lex.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\u{c}' => out.push_str("\\f"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 || matches!(c as u32, 0x7f | 0xfffe | 0xffff) => {
                out.push_str(&format!("\\u{:04X}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn write_iri(s: &str, out: &mut String) -> Result<()> {
    if s.chars().any(unwritable) {
        return Err(Error::unsupported(format!(
            "IRI <{s}> cannot be written in canonical N-Triples"
        )));
    }
    out.push('<');
    out.push_str(s);
    out.push('>');
    Ok(())
}

fn write_term(t: &Term, out: &mut String) -> Result<()> {
    match t {
        Term::Iri(s) => write_iri(s, out)?,
        Term::Blank(b) => {
            out.push_str("_:");
            out.push_str(b);
        }
        Term::Lit(Lit::Typed(lex, dt)) => {
            quote(lex, out);
            if dt != vocab::XSD_STRING {
                out.push_str("^^");
                write_iri(dt, out)?;
            }
        }
        Term::Lit(Lit::Lang(lex, tag)) => {
            quote(lex, out);
            out.push('@');
            out.push_str(tag);
        }
        Term::Triple(t) => {
            out.push_str("<<( ");
            write_triple(t, out)?;
            out.push_str(" )>>");
        }
    }
    Ok(())
}

fn write_triple(t: &Triple, out: &mut String) -> Result<()> {
    write_term(&t.0, out)?;
    out.push(' ');
    write_term(&t.1, out)?;
    out.push(' ');
    write_term(&t.2, out)
}

/// The reserved names the export gives meaning to, which statement content must not use.
fn reserved(v: &Value) -> bool {
    matches!(v, Value::Iri(s) if s.starts_with(vocab::SKOLEM_STMT))
}

// @tg: implements:: [[openspec:tm-cli#Statement graph export]] {since: "2026-10-31"}
/// Writes `db` as a statement graph in RDF 1.2 (see the module docs). Statements with a
/// `sys:` predicate are engine bookkeeping and are left out, as in the plain export;
/// a statement that refers to one then refers to a statement outside the export.
pub fn export_sg(db: &Db, sel: SgView, out: &mut dyn Write) -> Result<SgReport> {
    let all = db.history();
    let view = match sel.tx {
        SgTx::Now => db.now(),
        SgTx::AsOf(t) => db.as_of(TimeRef::Tx(t)),
        SgTx::History => db.history(),
    };
    let view = match sel.valid_at {
        Some(ms) => view.valid_at(ms),
        None => view,
    };
    let live: HashSet<Eid> = view
        .triples(None, None, None)?
        .into_iter()
        .map(|t| t.eid)
        .collect();

    let mut values: HashMap<ObjectId, Value> = HashMap::new();
    let mut value = |id: ObjectId| -> Result<Value> {
        if let Some(v) = values.get(&id) {
            return Ok(v.clone());
        }
        let v = all.decode(id)?;
        values.insert(id, v.clone());
        Ok(v)
    };

    let tm = |name: &str| iri(&format!("{}{name}", vocab::TM));
    let mut graph: BTreeSet<Triple> = BTreeSet::new();
    let mut report = SgReport::default();
    for t in all.triples(None, None, None)? {
        let (s, p, o) = (value(t.s)?, value(t.p)?, value(t.o)?);
        if matches!(&p, Value::Iri(i) if i.starts_with(vocab::SYS)) {
            continue;
        }
        if t.t_ret == Some(t.t_add) {
            report.never_visible += 1;
            continue;
        }
        let stmt = Value::Stmt(t.eid).lexical();
        for v in [&s, &p, &o] {
            if reserved(v) {
                return Err(Error::unsupported(format!(
                    "statement {}: {} is reserved by the statement-graph export",
                    t.eid.n(),
                    v.lexical()
                )));
            }
        }
        if matches!(&p, Value::Iri(i) if i == vocab::RDF_REIFIES) {
            return Err(Error::unsupported(format!(
                "statement {}: rdf:reifies is reserved by the statement-graph export",
                t.eid.n()
            )));
        }
        let base = Triple(term(render(&s)), term(render(&p)), term(render(&o)));
        let r = iri(&stmt);
        graph.insert(Triple(
            r.clone(),
            iri(vocab::RDF_REIFIES),
            Term::Triple(Box::new(base.clone())),
        ));
        graph.insert(Triple(r.clone(), tm("txAdded"), integer(t.t_add.0)));
        if let Some(ret) = t.t_ret {
            graph.insert(Triple(r.clone(), tm("txRetracted"), integer(ret.0)));
        }
        if let Some(ms) = t.v_from {
            graph.insert(Triple(r.clone(), iri(vocab::TM_VALID_FROM_MS), integer(ms)));
        }
        if let Some(ms) = t.v_to {
            graph.insert(Triple(r, iri(vocab::TM_VALID_TO_MS), integer(ms)));
        }
        report.records += 1;
        if live.contains(&t.eid) {
            report.live += 1;
            graph.insert(base);
        }
    }

    let mut line = String::new();
    for t in &graph {
        line.clear();
        write_triple(t, &mut line)?;
        line.push_str(" .\n");
        out.write_all(line.as_bytes()).map_err(io)?;
    }
    Ok(report)
}
