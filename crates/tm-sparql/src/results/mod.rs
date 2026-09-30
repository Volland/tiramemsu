//! Results: solutions, RDF terms, SPARQL JSON and N-Triples.

pub mod json;
pub mod nt;
pub mod term;

use tm_core::{Eid, Value};

pub use term::{RdfTerm, RdfTriple};

/// A table of solutions: variable names in projection order and rows of optional
/// values (`None` is unbound).
///
/// This is what a `SELECT` returns. Cells are stored terms ([`tm_core::Value`]);
/// use [`json::write_select`] for
/// the SPARQL 1.1 JSON form, or [`term::render`] to
/// get an [`RdfTerm`] with skolem IRIs for nodes, statements and transactions.
///
/// # Example
///
/// ```
/// use tm_core::{Eid, Value};
/// use tm_sparql::results::Solutions;
///
/// let s = Solutions {
///     vars: vec!["who".into(), "age".into()],
///     rows: vec![vec![Some(Value::iri("urn:tiramemsu:v:alice")), None]],
///     provenance: None,
/// };
/// assert_eq!(s.col("age"), Some(1));
/// assert!(s.get(0, "who").is_some());
/// assert!(s.get(0, "age").is_none()); // unbound
/// assert_eq!(s.provenance(0), None); // not requested
/// ```
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Solutions {
    /// The variable names, without `?`.
    pub vars: Vec<String>,
    /// The rows.
    pub rows: Vec<Vec<Option<Value>>>,
    /// Per-row provenance, parallel to `rows`: the eids of the stored statements
    /// that matched to produce each row, ascending and without duplicates. `None`
    /// unless the query ran with provenance (`query-provenance`).
    pub provenance: Option<Vec<Vec<Eid>>>,
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

    /// The eids of the statements that produced row `row`, ascending: `None` when
    /// the query ran without provenance or the row does not exist.
    ///
    /// ```
    /// use tm_core::{Eid, Value};
    /// use tm_sparql::results::Solutions;
    ///
    /// let s = Solutions {
    ///     vars: vec!["c".into()],
    ///     rows: vec![vec![Some(Value::iri("urn:tiramemsu:v:paris"))]],
    ///     provenance: Some(vec![vec![Eid::new(1), Eid::new(2)]]),
    /// };
    /// assert_eq!(s.provenance(0), Some(&[Eid::new(1), Eid::new(2)][..]));
    /// assert_eq!(s.provenance(1), None);
    /// ```
    pub fn provenance(&self, row: usize) -> Option<&[Eid]> {
        self.provenance.as_ref()?.get(row).map(Vec::as_slice)
    }
}
