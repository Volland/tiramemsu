//! Results: solutions, RDF terms, SPARQL JSON and N-Triples.

pub mod json;
pub mod nt;
pub mod term;

use tm_core::Value;

pub use term::{RdfTerm, RdfTriple};

/// A table of solutions: variable names in projection order and rows of optional
/// values (`None` is unbound).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Solutions {
    /// The variable names, without `?`.
    pub vars: Vec<String>,
    /// The rows.
    pub rows: Vec<Vec<Option<Value>>>,
}

impl Solutions {
    /// The index of the column `var`.
    pub fn col(&self, var: &str) -> Option<usize> {
        self.vars.iter().position(|v| v == var)
    }

    /// The value in row `row`, column `var`.
    pub fn get(&self, row: usize, var: &str) -> Option<&Value> {
        self.rows.get(row)?.get(self.col(var)?)?.as_ref()
    }
}
