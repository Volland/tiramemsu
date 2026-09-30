//! Result parsing and comparison for the W3C runner: rows and graphs are compared
//! up to blank node isomorphism, after `urn:tiramemsu:bnode:` IRIs are mapped back
//! to blank nodes.
#![allow(dead_code)]

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use oxttl::{NTriplesParser, TurtleParser};
use sparesults::{QueryResultsFormat, QueryResultsParser, ReaderQueryResultsParserOutput};
use spargebra::term::{Term, Triple};
use tm_sparql::results::{RdfTerm, RdfTriple};

/// A term as text: `<iri>`, `"lex"^^<dt>`, `"lex"@tag`, `"lex"`, `_:label`,
/// `<<( s p o )>>`.
pub fn term_text(t: &Term) -> String {
    match t {
        Term::NamedNode(n) => iri_text(n.as_str()),
        Term::BlankNode(b) => format!("_:{}", b.as_str()),
        Term::Literal(l) => match l.language() {
            Some(lang) => format!("{:?}@{}", l.value(), lang.to_ascii_lowercase()),
            None if l.datatype().as_str() == "http://www.w3.org/2001/XMLSchema#string" => {
                format!("{:?}", l.value())
            }
            None => format!("{:?}^^<{}>", l.value(), l.datatype().as_str()),
        },
        Term::Triple(t) => triple_text(t),
    }
}

fn iri_text(iri: &str) -> String {
    match iri.strip_prefix("urn:tiramemsu:bnode:") {
        Some(n) => format!("_:sk{n}"),
        None => format!("<{iri}>"),
    }
}

fn triple_text(t: &Triple) -> String {
    format!(
        "<<( {} {} {} )>>",
        term_text(&Term::from(t.subject.clone())),
        iri_text(t.predicate.as_str()),
        term_text(&t.object)
    )
}

/// An engine term as text (same format as [`term_text`]).
pub fn rdf_text(t: &RdfTerm) -> String {
    match t {
        RdfTerm::Iri(i) => iri_text(i),
        RdfTerm::Blank(b) => format!("_:{b}"),
        RdfTerm::Literal {
            lex,
            datatype,
            lang,
        } => match (lang, datatype) {
            (Some(l), _) => format!("{lex:?}@{l}"),
            (None, Some(d)) if d != "http://www.w3.org/2001/XMLSchema#string" => {
                format!("{lex:?}^^<{d}>")
            }
            _ => format!("{lex:?}"),
        },
        RdfTerm::Triple(t) => rdf_triple_text(t),
    }
}

fn rdf_triple_text(t: &RdfTriple) -> String {
    format!(
        "<<( {} {} {} )>>",
        rdf_text(&t.s),
        rdf_text(&t.p),
        rdf_text(&t.o)
    )
}

pub type Row = BTreeMap<String, String>;

/// A parsed expected result.
#[derive(Debug, Clone)]
pub enum Expected {
    Boolean(bool),
    Solutions { vars: Vec<String>, rows: Vec<Row> },
    Graph(Vec<[String; 3]>),
}

/// Reads an expected result file by extension.
pub fn read_expected(path: &Path) -> Result<Expected, String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let bytes = std::fs::read(path).map_err(|e| format!("{path:?}: {e}"))?;
    match ext {
        "srx" | "srj" => {
            let fmt = if ext == "srx" {
                QueryResultsFormat::Xml
            } else {
                QueryResultsFormat::Json
            };
            match QueryResultsParser::from_format(fmt)
                .for_reader(bytes.as_slice())
                .map_err(|e| e.to_string())?
            {
                ReaderQueryResultsParserOutput::Boolean(b) => Ok(Expected::Boolean(b)),
                ReaderQueryResultsParserOutput::Solutions(s) => {
                    let vars = s
                        .variables()
                        .iter()
                        .map(|v| v.as_str().to_string())
                        .collect();
                    let mut rows = Vec::new();
                    for r in s {
                        let r = r.map_err(|e| e.to_string())?;
                        rows.push(
                            r.iter()
                                .map(|(v, t)| (v.as_str().to_string(), term_text(t)))
                                .collect(),
                        );
                    }
                    Ok(Expected::Solutions { vars, rows })
                }
            }
        }
        "ttl" | "nt" => match read_result_set(path)? {
            Some(rs) => Ok(rs),
            None => Ok(Expected::Graph(read_graph(path)?)),
        },
        other => Err(format!("unsupported result format .{other}")),
    }
}

/// The triples of a Turtle or N-Triples file as text tuples (RDF/XML is not read:
/// `oxrdfxml` does not build together with oxrdf's `rdf-12` feature).
pub fn read_graph(path: &Path) -> Result<Vec<[String; 3]>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{path:?}: {e}"))?;
    let base = format!("file://{}", path.display());
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let mut out = Vec::new();
    let mut push = |t: &Triple| {
        out.push([
            term_text(&Term::from(t.subject.clone())),
            iri_text(t.predicate.as_str()),
            term_text(&t.object),
        ]);
    };
    match ext {
        "ttl" => {
            for t in TurtleParser::new()
                .with_base_iri(base)
                .map_err(|e| e.to_string())?
                .for_reader(bytes.as_slice())
            {
                push(&t.map_err(|e| e.to_string())?);
            }
        }
        "nt" => {
            for t in NTriplesParser::new().for_reader(bytes.as_slice()) {
                push(&t.map_err(|e| e.to_string())?);
            }
        }
        other => return Err(format!("unsupported graph format .{other}")),
    }
    Ok(out)
}

fn is_bnode(s: &str) -> bool {
    s.starts_with("_:")
}

/// Extends `map` (actual label → expected label) so that `a` equals `b`, or `false`.
fn unify(
    a: &str,
    b: &str,
    map: &mut HashMap<String, String>,
    rev: &mut HashMap<String, String>,
) -> bool {
    // terms are compared token-wise: blank node labels inside a term (triple terms)
    let (ta, tb) = (tokens(a), tokens(b));
    if ta.len() != tb.len() {
        return false;
    }
    for (x, y) in ta.iter().zip(&tb) {
        if is_bnode(x) && is_bnode(y) {
            match (map.get(x), rev.get(y)) {
                (Some(m), _) if m != y => return false,
                (_, Some(r)) if r != x => return false,
                _ => {
                    map.insert(x.clone(), y.clone());
                    rev.insert(y.clone(), x.clone());
                }
            }
        } else if x != y {
            return false;
        }
    }
    true
}

/// Splits a term text into blank-node tokens and other text chunks.
fn tokens(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let b = s.as_bytes();
    let mut i = 0;
    let mut in_str = false;
    while i < b.len() {
        let c = b[i] as char;
        if in_str {
            cur.push(c);
            if c == '\\' && i + 1 < b.len() {
                cur.push(b[i + 1] as char);
                i += 1;
            } else if c == '"' {
                in_str = false;
            }
        } else if c == '"' {
            in_str = true;
            cur.push(c);
        } else if c == '_'
            && b.get(i + 1) == Some(&b':')
            && (i == 0 || !b[i - 1].is_ascii_alphanumeric())
        {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            let mut j = i + 2;
            while j < b.len() && ((b[j] as char).is_alphanumeric() || b[j] == b'_' || b[j] == b'-')
            {
                j += 1;
            }
            out.push(s[i..j].to_string());
            i = j;
            continue;
        } else {
            cur.push(c);
        }
        i += 1;
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn row_eq(
    a: &[(&String, &String)],
    b: &[(&String, &String)],
    map: &mut HashMap<String, String>,
    rev: &mut HashMap<String, String>,
) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|((va, ta), (vb, tb))| va == vb && unify(ta, tb, map, rev))
}

fn match_rows(
    a: &[Row],
    b: &[Row],
    used: &mut Vec<bool>,
    i: usize,
    map: &mut HashMap<String, String>,
    rev: &mut HashMap<String, String>,
    ordered: bool,
) -> bool {
    if i == a.len() {
        return true;
    }
    let ra: Vec<_> = a[i].iter().collect();
    let candidates: Vec<usize> = if ordered {
        vec![i]
    } else {
        (0..b.len()).collect()
    };
    for j in candidates {
        if used[j] {
            continue;
        }
        let rb: Vec<_> = b[j].iter().collect();
        let (mut m2, mut r2) = (map.clone(), rev.clone());
        if row_eq(&ra, &rb, &mut m2, &mut r2) {
            used[j] = true;
            if match_rows(a, b, used, i + 1, &mut m2, &mut r2, ordered) {
                *map = m2;
                *rev = r2;
                return true;
            }
            used[j] = false;
        }
    }
    false
}

/// Solutions are equal up to blank node isomorphism (and order, unless `ordered`).
pub fn rows_equal(actual: &[Row], expected: &[Row], ordered: bool) -> bool {
    if actual.len() != expected.len() {
        return false;
    }
    let mut used = vec![false; expected.len()];
    match_rows(
        actual,
        expected,
        &mut used,
        0,
        &mut HashMap::new(),
        &mut HashMap::new(),
        ordered,
    )
}

fn match_triples(
    a: &[[String; 3]],
    b: &[[String; 3]],
    used: &mut Vec<bool>,
    i: usize,
    map: &mut HashMap<String, String>,
    rev: &mut HashMap<String, String>,
) -> bool {
    if i == a.len() {
        return true;
    }
    for j in 0..b.len() {
        if used[j] {
            continue;
        }
        let (mut m2, mut r2) = (map.clone(), rev.clone());
        if (0..3).all(|k| unify(&a[i][k], &b[j][k], &mut m2, &mut r2)) {
            used[j] = true;
            if match_triples(a, b, used, i + 1, &mut m2, &mut r2) {
                *map = m2;
                *rev = r2;
                return true;
            }
            used[j] = false;
        }
    }
    false
}

/// Graphs are equal up to blank node isomorphism (as sets).
pub fn graphs_equal(actual: &[[String; 3]], expected: &[[String; 3]]) -> bool {
    let dedup = |g: &[[String; 3]]| {
        let mut v = g.to_vec();
        v.sort();
        v.dedup();
        v
    };
    let (a, b) = (dedup(actual), dedup(expected));
    if a.len() != b.len() {
        return false;
    }
    let mut used = vec![false; b.len()];
    match_triples(
        &a,
        &b,
        &mut used,
        0,
        &mut HashMap::new(),
        &mut HashMap::new(),
    )
}

const RS: &str = "http://www.w3.org/2001/sw/DataAccess/tests/result-set#";
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

/// A DAWG result set in Turtle (`rs:ResultSet`), the SPARQL 1.0 test format.
fn read_result_set(path: &Path) -> Result<Option<Expected>, String> {
    use crate::manifest::Graph;
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let g = Graph::parse(&text, &format!("file://{}", path.display()));
    let Some(set) = g
        .subjects()
        .into_iter()
        .find(|s| g.iri(s, RDF_TYPE).as_deref() == Some(&format!("{RS}ResultSet")))
    else {
        return Ok(None);
    };
    if let Some(b) = g.text(&set, &format!("{RS}boolean")) {
        return Ok(Some(Expected::Boolean(b == "true" || b == "1")));
    }
    let vars: Vec<String> = g
        .objects(&set, &format!("{RS}resultVariable"))
        .into_iter()
        .filter_map(|t| match t {
            Term::Literal(l) => Some(l.value().to_string()),
            _ => None,
        })
        .collect();
    let mut sols: Vec<(i64, Row)> = Vec::new();
    for (i, s) in g
        .objects(&set, &format!("{RS}solution"))
        .into_iter()
        .enumerate()
    {
        let Term::BlankNode(b) = s else { continue };
        let key = format!("_:{}", b.as_str());
        let index = g
            .text(&key, &format!("{RS}index"))
            .and_then(|x| x.parse().ok())
            .unwrap_or(i as i64);
        let mut row = Row::new();
        for bind in g.objects(&key, &format!("{RS}binding")) {
            let Term::BlankNode(bb) = bind else { continue };
            let bk = format!("_:{}", bb.as_str());
            if let (Some(v), Some(val)) = (
                g.text(&bk, &format!("{RS}variable")),
                g.object(&bk, &format!("{RS}value")),
            ) {
                row.insert(v, term_text(val));
            }
        }
        sols.push((index, row));
    }
    sols.sort_by_key(|(i, _)| *i);
    Ok(Some(Expected::Solutions {
        vars,
        rows: sols.into_iter().map(|(_, r)| r).collect(),
    }))
}
