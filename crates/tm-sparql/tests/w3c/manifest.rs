//! A minimal reader of W3C test manifests (Turtle, through `oxttl`).
#![allow(dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use oxttl::TurtleParser;
use spargebra::term::{NamedOrBlankNode, Term};

/// The default graph of a manifest, indexed by subject.
pub struct Graph {
    by_subject: HashMap<String, Vec<(String, Term)>>,
}

fn subject_key(s: &NamedOrBlankNode) -> String {
    match s {
        NamedOrBlankNode::NamedNode(n) => n.as_str().to_string(),
        NamedOrBlankNode::BlankNode(b) => format!("_:{}", b.as_str()),
    }
}

fn term_key(t: &Term) -> Option<String> {
    match t {
        Term::NamedNode(n) => Some(n.as_str().to_string()),
        Term::BlankNode(b) => Some(format!("_:{}", b.as_str())),
        _ => None,
    }
}

impl Graph {
    pub fn parse(text: &str, base: &str) -> Graph {
        let mut by_subject: HashMap<String, Vec<(String, Term)>> = HashMap::new();
        let parser = TurtleParser::new().with_base_iri(base).expect("base");
        for t in parser.for_reader(text.as_bytes()) {
            let t = t.expect("manifest triple");
            by_subject
                .entry(subject_key(&t.subject))
                .or_default()
                .push((t.predicate.as_str().to_string(), t.object));
        }
        Graph { by_subject }
    }

    pub fn subjects(&self) -> Vec<String> {
        self.by_subject.keys().cloned().collect()
    }

    pub fn objects(&self, s: &str, p: &str) -> Vec<&Term> {
        self.by_subject
            .get(s)
            .map(|v| v.iter().filter(|(q, _)| q == p).map(|(_, o)| o).collect())
            .unwrap_or_default()
    }

    pub fn object(&self, s: &str, p: &str) -> Option<&Term> {
        self.objects(s, p).into_iter().next()
    }

    pub fn iri(&self, s: &str, p: &str) -> Option<String> {
        self.object(s, p).and_then(term_key)
    }

    pub fn text(&self, s: &str, p: &str) -> Option<String> {
        match self.object(s, p)? {
            Term::Literal(l) => Some(l.value().to_string()),
            _ => None,
        }
    }

    /// The members of the RDF list starting at `head`.
    pub fn list(&self, head: &str) -> Vec<String> {
        const FIRST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#first";
        const REST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#rest";
        const NIL: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#nil";
        let mut out = Vec::new();
        let mut cur = head.to_string();
        while cur != NIL {
            let Some(first) = self.iri(&cur, FIRST) else {
                break;
            };
            out.push(first);
            let Some(rest) = self.iri(&cur, REST) else {
                break;
            };
            cur = rest;
        }
        out
    }
}

pub const MF: &str = "http://www.w3.org/2001/sw/DataAccess/tests/test-manifest#";
pub const QT: &str = "http://www.w3.org/2001/sw/DataAccess/tests/test-query#";
pub const UT: &str = "http://www.w3.org/2009/sparql/tests/test-update#";
pub const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
pub const DAWGT_APPROVAL: &str = "http://www.w3.org/2001/sw/DataAccess/tests/test-dawg#approval";

/// One test of a manifest.
#[derive(Debug, Clone)]
pub struct Test {
    pub iri: String,
    /// The local name of the `rdf:type` (`QueryEvaluationTest`, ...).
    pub kind: String,
    pub name: String,
    /// `mf:action` when it is a single file (syntax tests).
    pub action_file: Option<PathBuf>,
    pub query: Option<PathBuf>,
    pub request: Option<PathBuf>,
    pub data: Vec<PathBuf>,
    pub graph_data: Vec<PathBuf>,
    pub result: Option<PathBuf>,
    pub result_data: Vec<PathBuf>,
    pub result_graph_data: Vec<PathBuf>,
    pub approval: Option<String>,
}

fn path_of(iri: &str) -> PathBuf {
    PathBuf::from(iri.strip_prefix("file://").unwrap_or(iri))
}

/// Reads every test of the manifest at `path`.
pub fn read_manifest(path: &Path) -> Vec<Test> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
    let base = format!("file://{}", path.display());
    let g = Graph::parse(&text, &base);
    let mut tests = Vec::new();
    // the manifest node is the one with mf:entries
    let manifests: Vec<String> = g
        .by_subject
        .keys()
        .filter(|s| g.object(s, &format!("{MF}entries")).is_some())
        .cloned()
        .collect();
    for m in manifests {
        let head = g.iri(&m, &format!("{MF}entries")).unwrap();
        for t in g.list(&head) {
            let kind = g
                .iri(&t, RDF_TYPE)
                .map(|k| k.rsplit(['#', '/']).next().unwrap_or("").to_string())
                .unwrap_or_default();
            let mut test = Test {
                iri: t.clone(),
                kind,
                name: g.text(&t, &format!("{MF}name")).unwrap_or_default(),
                action_file: None,
                query: None,
                request: None,
                data: Vec::new(),
                graph_data: Vec::new(),
                result: None,
                result_data: Vec::new(),
                result_graph_data: Vec::new(),
                approval: g.iri(&t, DAWGT_APPROVAL),
            };
            if let Some(action) = g.iri(&t, &format!("{MF}action")) {
                if action.starts_with("_:") {
                    test.query = g.iri(&action, &format!("{QT}query")).map(|p| path_of(&p));
                    test.request = g.iri(&action, &format!("{UT}request")).map(|p| path_of(&p));
                    for p in g
                        .objects(&action, &format!("{QT}data"))
                        .into_iter()
                        .chain(g.objects(&action, &format!("{UT}data")))
                    {
                        if let Some(i) = term_key(p) {
                            test.data.push(path_of(&i));
                        }
                    }
                    for p in g
                        .objects(&action, &format!("{QT}graphData"))
                        .into_iter()
                        .chain(g.objects(&action, &format!("{UT}graphData")))
                    {
                        if let Some(i) = term_key(p) {
                            test.graph_data.push(path_of(&i));
                        }
                    }
                } else {
                    test.action_file = Some(path_of(&action));
                }
            }
            if let Some(result) = g.iri(&t, &format!("{MF}result")) {
                if result.starts_with("_:") {
                    for p in g.objects(&result, &format!("{UT}data")) {
                        if let Some(i) = term_key(p) {
                            test.result_data.push(path_of(&i));
                        }
                    }
                    for p in g.objects(&result, &format!("{UT}graphData")) {
                        if let Some(i) = term_key(p) {
                            test.result_graph_data.push(path_of(&i));
                        }
                    }
                } else {
                    test.result = Some(path_of(&result));
                }
            }
            tests.push(test);
        }
    }
    tests
}
