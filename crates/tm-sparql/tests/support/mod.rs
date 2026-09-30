//! Shared support of the golden and W3C runners: fresh databases, the Turtle
//! loader (through `INSERT DATA`), and result normalisation.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use oxttl::TurtleParser;
use sparesults::{QueryResultsFormat, QueryResultsParser, ReaderQueryResultsParserOutput};
use spargebra::term::{Term, Triple};
use tiramemsu::{Db, ManualClock, OpenOptions, SparqlResult};
use tm_sparql::results::{nt, RdfTerm, RdfTriple};

/// A fresh temporary database with a manual clock.
pub struct TestDb {
    pub dir: tempfile::TempDir,
    pub db: Db,
    pub sql_runs: Arc<AtomicU64>,
}

impl TestDb {
    pub fn new() -> TestDb {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Db::open(
            dir.path().join("g.db"),
            OpenOptions {
                clock: Arc::new(ManualClock::new(1_000_000)),
                ..OpenOptions::default()
            },
        )
        .expect("open");
        let sql_runs = Arc::new(AtomicU64::new(0));
        let c = sql_runs.clone();
        db.set_query_hook(Some(Arc::new(move || {
            c.fetch_add(1, Ordering::SeqCst);
        })));
        TestDb { dir, db, sql_runs }
    }

    pub fn runs(&self) -> u64 {
        self.sql_runs.load(Ordering::SeqCst)
    }

    pub fn last_t(&self) -> i64 {
        self.db
            .read_sql("SELECT value FROM meta WHERE key = 'last_t'")
            .unwrap()[0][0]
            .as_i64()
            .unwrap()
    }
}

/// The engine triple of an oxrdf triple.
pub fn rdf_triple(t: &Triple) -> RdfTriple {
    RdfTriple {
        s: rdf_term(&Term::from(t.subject.clone())),
        p: RdfTerm::Iri(t.predicate.as_str().to_string()),
        o: rdf_term(&t.object),
    }
}

fn rdf_term(t: &Term) -> RdfTerm {
    match t {
        Term::NamedNode(n) => RdfTerm::Iri(n.as_str().to_string()),
        Term::BlankNode(b) => RdfTerm::Blank(b.as_str().to_string()),
        Term::Literal(l) => {
            let lang = l.language().map(str::to_string);
            let dt = l.datatype().as_str().to_string();
            RdfTerm::Literal {
                lex: l.value().to_string(),
                datatype: (lang.is_none() && dt != "http://www.w3.org/2001/XMLSchema#string")
                    .then_some(dt),
                lang,
            }
        }
        Term::Triple(t) => RdfTerm::Triple(Box::new(rdf_triple(t))),
    }
}

/// Loads triples into the default graph through one `INSERT DATA`.
pub fn load_triples(db: &Db, triples: &[RdfTriple]) -> Result<usize, String> {
    if triples.is_empty() {
        return Ok(0);
    }
    let text = format!("INSERT DATA {{\n{}}}", nt::write(triples));
    db.now().sparql(&text).map_err(|e| format!("{e}"))?;
    Ok(triples.len())
}

/// Loads Turtle text into the default graph.
pub fn load_turtle(db: &Db, ttl: &str, base: Option<&str>) -> Result<usize, String> {
    let mut p = TurtleParser::new();
    if let Some(b) = base {
        p = p.with_base_iri(b).map_err(|e| e.to_string())?;
    }
    let mut triples = Vec::new();
    for t in p.for_reader(ttl.as_bytes()) {
        triples.push(rdf_triple(&t.map_err(|e| e.to_string())?));
    }
    load_triples(db, &triples)
}

/// One result term as text: `<iri>`, `"lex"^^<dt>`, `"lex"@tag`, `"lex"`, `_:b`.
pub fn term_text(t: &Term) -> String {
    match t {
        Term::NamedNode(n) => format!("<{}>", n.as_str()),
        Term::BlankNode(b) => format!("_:{}", b.as_str()),
        Term::Literal(l) => match l.language() {
            Some(lang) => format!("{:?}@{lang}", l.value()),
            None if l.datatype().as_str() == "http://www.w3.org/2001/XMLSchema#string" => {
                format!("{:?}", l.value())
            }
            None => format!("{:?}^^<{}>", l.value(), l.datatype().as_str()),
        },
        Term::Triple(_) => "<<triple>>".to_string(),
    }
}

/// A parsed SPARQL JSON document: the variables and, per row, `var → term text`.
#[derive(Debug, PartialEq, Clone)]
pub enum Srj {
    Solutions {
        vars: Vec<String>,
        rows: Vec<BTreeMap<String, String>>,
    },
    Boolean(bool),
}

pub fn parse_srj(doc: &str) -> Srj {
    let parser = QueryResultsParser::from_format(QueryResultsFormat::Json);
    match parser
        .for_reader(doc.as_bytes())
        .expect("valid SPARQL JSON")
    {
        ReaderQueryResultsParserOutput::Boolean(b) => Srj::Boolean(b),
        ReaderQueryResultsParserOutput::Solutions(sols) => {
            let vars = sols
                .variables()
                .iter()
                .map(|v| v.as_str().to_string())
                .collect();
            let rows = sols
                .map(|r| {
                    let r = r.expect("row");
                    r.iter()
                        .map(|(v, t)| (v.as_str().to_string(), term_text(t)))
                        .collect()
                })
                .collect();
            Srj::Solutions { vars, rows }
        }
    }
}

/// The JSON text of a query result.
pub fn to_srj(r: &SparqlResult) -> Option<String> {
    r.write_sparql_json().ok()
}
