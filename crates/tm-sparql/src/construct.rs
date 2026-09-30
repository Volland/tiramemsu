//! `CONSTRUCT` template instantiation (queries and, later, update templates).
//!
//! A template triple is skipped for a solution when a variable is unbound or a
//! term is invalid in its position. Blank nodes are fresh per solution, and the
//! result is a set of triples in first-occurrence order.

use std::collections::{HashMap, HashSet};

use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern};
use tm_core::Result;

use crate::results::term::{render, RdfTerm, RdfTriple};
use crate::results::Solutions;
use crate::terms;

struct Ctx<'a> {
    sol: &'a Solutions,
    row: usize,
    blanks: HashMap<String, String>,
}

impl Ctx<'_> {
    fn var(&self, name: &str) -> Option<RdfTerm> {
        self.sol.get(self.row, name).map(render)
    }

    fn blank(&mut self, label: &str) -> RdfTerm {
        let row = self.row;
        let id = self
            .blanks
            .entry(label.to_string())
            .or_insert_with(|| format!("b{row}_{label}"));
        RdfTerm::Blank(id.clone())
    }

    fn term(&mut self, t: &TermPattern) -> Result<Option<RdfTerm>> {
        Ok(match t {
            TermPattern::NamedNode(n) => Some(node(n)),
            TermPattern::Literal(l) => Some(render(&terms::literal(l)?)),
            TermPattern::Variable(v) => self.var(v.as_str()),
            TermPattern::BlankNode(b) => Some(self.blank(b.as_str())),
            TermPattern::Triple(tp) => self.triple(tp)?.map(|t| RdfTerm::Triple(Box::new(t))),
        })
    }

    fn predicate(&mut self, p: &NamedNodePattern) -> Option<RdfTerm> {
        match p {
            NamedNodePattern::NamedNode(n) => Some(node(n)),
            NamedNodePattern::Variable(v) => self.var(v.as_str()).filter(RdfTerm::is_iri),
        }
    }

    fn triple(&mut self, t: &TriplePattern) -> Result<Option<RdfTriple>> {
        let s = self.term(&t.subject)?;
        let p = self.predicate(&t.predicate);
        let o = self.term(&t.object)?;
        Ok(match (s, p, o) {
            (Some(s), Some(p), Some(o))
                if matches!(s, RdfTerm::Iri(_) | RdfTerm::Blank(_)) && p.is_iri() =>
            {
                Some(RdfTriple { s, p, o })
            }
            _ => None,
        })
    }
}

fn node(n: &spargebra::term::NamedNode) -> RdfTerm {
    render(&terms::named_node(n))
}

/// Instantiates `template` once per solution.
pub fn instantiate(template: &[TriplePattern], sol: &Solutions) -> Result<Vec<RdfTriple>> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for row in 0..sol.rows.len() {
        let mut ctx = Ctx {
            sol,
            row,
            blanks: HashMap::new(),
        };
        for t in template {
            if let Some(tr) = ctx.triple(t)? {
                if seen.insert(tr.clone()) {
                    out.push(tr);
                }
            }
        }
    }
    Ok(out)
}
