//! RDF files in and out: `tm import` (N-Triples, N-Quads, Turtle, TriG through
//! `oxttl`, in chunked bulk-import transactions) and `tm export` (the live
//! statements as N-Quads or N-Triples).

use std::collections::HashMap;
use std::io::{BufReader, Write};
use std::path::Path;

use clap::ValueEnum;
use oxrdf::{GraphName, Quad, Term};
use oxttl::{NQuadsParser, NTriplesParser, TriGParser, TurtleParser};
use tiramemsu::rdf::{render, write_term, RdfTerm};
use tiramemsu::*;

use crate::io;

/// Input formats of `tm import`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum RdfFormat {
    Ntriples,
    Nquads,
    Turtle,
    Trig,
}

impl RdfFormat {
    /// The format named by a file extension.
    pub fn from_path(p: &Path) -> Result<RdfFormat> {
        match p
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("nt") => Ok(RdfFormat::Ntriples),
            Some("nq") => Ok(RdfFormat::Nquads),
            Some("ttl") => Ok(RdfFormat::Turtle),
            Some("trig") => Ok(RdfFormat::Trig),
            _ => Err(Error::unsupported(format!(
                "{}: unknown RDF extension (use --format)",
                p.display()
            ))),
        }
    }
}

/// Output formats of `tm export`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum ExportFormat {
    Nquads,
    Ntriples,
}

/// What an import did.
#[derive(Clone, Debug, Default)]
pub struct Loaded {
    pub statements: u64,
    pub new: u64,
    pub transactions: u64,
}

impl Loaded {
    pub fn add(&mut self, o: &Loaded) {
        self.statements += o.statements;
        self.new += o.new;
        self.transactions += o.transactions;
    }
}

fn parse_error(path: &Path, e: impl std::fmt::Display) -> Error {
    Error::custom(format!("RDF syntax in {}: {e}", path.display()))
}

/// Loads `path` into `db` in transactions of `chunk` statements.
pub fn import(db: &Db, path: &Path, format: RdfFormat, chunk: usize) -> Result<Loaded> {
    let file = std::fs::File::open(path).map_err(io)?;
    let r = BufReader::new(file);
    let quads: Box<dyn Iterator<Item = Result<Quad>>> = match format {
        RdfFormat::Ntriples => Box::new(NTriplesParser::new().for_reader(r).map(move |t| {
            t.map(|t| t.in_graph(GraphName::DefaultGraph))
                .map_err(|e| parse_error(path, e))
        })),
        RdfFormat::Turtle => Box::new(TurtleParser::new().for_reader(r).map(move |t| {
            t.map(|t| t.in_graph(GraphName::DefaultGraph))
                .map_err(|e| parse_error(path, e))
        })),
        RdfFormat::Nquads => Box::new(
            NQuadsParser::new()
                .for_reader(r)
                .map(move |q| q.map_err(|e| parse_error(path, e))),
        ),
        RdfFormat::Trig => Box::new(
            TriGParser::new()
                .for_reader(r)
                .map(move |q| q.map_err(|e| parse_error(path, e))),
        ),
    };
    let mut session = db.bulk_import()?;
    // blank node labels are scoped to the file: one fresh node per label
    let mut blanks: HashMap<String, ObjectId> = HashMap::new();
    let mut loaded = Loaded::default();
    let mut quads = quads.peekable();
    while quads.peek().is_some() {
        let batch: Vec<Quad> = quads.by_ref().take(chunk).collect::<Result<_>>()?;
        let report = session.chunk(|tx| {
            for q in &batch {
                let s = node(tx, &mut blanks, &Term::from(q.subject.clone()))?;
                let p = Value::iri(q.predicate.as_str());
                let o = node(tx, &mut blanks, &q.object)?;
                let eid = tx.assert(s, p, o, Valid::ALWAYS)?.eid();
                match &q.graph_name {
                    GraphName::DefaultGraph => {}
                    GraphName::NamedNode(g) => {
                        tx.add_to_graph(eid, Value::iri(g.as_str()), AssertOpts::default())?;
                    }
                    GraphName::BlankNode(b) => {
                        let g = blank(tx, &mut blanks, b.as_str())?;
                        tx.add_to_graph(eid, g, AssertOpts::default())?;
                    }
                }
            }
            Ok(())
        })?;
        loaded.statements += batch.len() as u64;
        loaded.new += report.asserted.len() as u64;
        loaded.transactions += 1;
    }
    session.finish();
    Ok(loaded)
}

fn blank(tx: &mut Tx<'_>, blanks: &mut HashMap<String, ObjectId>, label: &str) -> Result<ObjectId> {
    if let Some(id) = blanks.get(label) {
        return Ok(*id);
    }
    let id = tx.new_bnode()?;
    blanks.insert(label.to_string(), id);
    Ok(id)
}

/// A term as a stored object: IRIs and literals by value, blank nodes by label.
fn node(tx: &mut Tx<'_>, blanks: &mut HashMap<String, ObjectId>, t: &Term) -> Result<Obj> {
    Ok(match t {
        Term::NamedNode(n) => Value::iri(n.as_str()).into_obj(),
        Term::BlankNode(b) => blank(tx, blanks, b.as_str())?.into_obj(),
        Term::Literal(l) => {
            let lang = l.language();
            let dt = l.datatype().as_str();
            let dt =
                (lang.is_none() && dt != "http://www.w3.org/2001/XMLSchema#string").then_some(dt);
            Value::literal(l.value(), dt, lang).into_obj()
        }
        Term::Triple(_) => return Err(Error::unsupported("triple terms in `tm import`")),
    })
}

/// The predicates the engine keeps for itself (transaction metadata, graph
/// declarations): not data, never exported.
const SYSTEM: &str = "urn:tiramemsu:sys:";

/// Writes the live statements of `db`: every statement once in the default
/// graph and, for N-Quads, once more in each named graph it belongs to.
pub fn export(db: &Db, format: ExportFormat, out: &mut dyn Write) -> Result<()> {
    let view = db.now();
    let mut terms: HashMap<ObjectId, String> = HashMap::new();
    let mut text = |id: ObjectId| -> Result<String> {
        if let Some(t) = terms.get(&id) {
            return Ok(t.clone());
        }
        let t = write_term(&render(&view.decode(id)?));
        terms.insert(id, t.clone());
        Ok(t)
    };
    let triples = view.triples(None, None, None)?;
    let mut by_eid = HashMap::with_capacity(triples.len());
    for t in &triples {
        let p = text(t.p)?;
        if p.starts_with(&format!("<{SYSTEM}")) {
            continue;
        }
        let line = format!("{} {p} {}", text(t.s)?, text(t.o)?);
        writeln!(out, "{line} .").map_err(io)?;
        by_eid.insert(t.eid, line);
    }
    if format == ExportFormat::Nquads {
        for g in view.graphs()? {
            let gname = text(g)?;
            if !matches!(
                render(&view.decode(g)?),
                RdfTerm::Iri(_) | RdfTerm::Blank(_)
            ) {
                continue;
            }
            for eid in view.graph_members(g)? {
                if let Some(line) = by_eid.get(&eid) {
                    writeln!(out, "{line} {gname} .").map_err(io)?;
                }
            }
        }
    }
    Ok(())
}
