//! Materialised relations: one column per variable with its static domain, and
//! rows of SQL values.

use tm_core::ObjectId;
use tm_exec::plan::analyze::Dom;
use tm_ir::Var;

use crate::value::V;

/// The static facts of one column, as `tm_exec::sqlgen::Col` has them.
#[derive(Clone, Debug, PartialEq)]
pub struct Col {
    /// The value domain.
    pub dom: Dom,
    /// May be missing.
    pub mm: bool,
    /// Always missing (the SQL `NULL` column of an absent variable).
    pub null: bool,
}

impl Col {
    /// A term column.
    pub fn term(mm: bool) -> Col {
        Col {
            dom: Dom::Term,
            mm,
            null: false,
        }
    }

    /// An always-missing column.
    pub fn null() -> Col {
        Col {
            dom: Dom::Term,
            mm: true,
            null: true,
        }
    }
}

/// A relationship pattern under relationship isomorphism: its eid column must
/// differ from every other eid of the same match group.
#[derive(Clone, Debug, PartialEq)]
pub struct Iso {
    /// Match group.
    pub group: u32,
    /// The column holding the eid.
    pub col: usize,
    /// Its constant predicate.
    pub pred: Option<ObjectId>,
    /// On the optional side of a left join.
    pub optional: bool,
}

/// A relation.
#[derive(Clone, Debug, Default)]
pub struct Rel {
    /// The variables, one per column.
    pub vars: Vec<Var>,
    /// The column facts.
    pub cols: Vec<Col>,
    /// The rows.
    pub rows: Vec<Vec<V>>,
    /// Relationship patterns (isomorphism), by column.
    pub iso: Vec<Iso>,
}

impl Rel {
    /// The unit relation: one row, no column.
    pub fn unit() -> Rel {
        Rel {
            rows: vec![Vec::new()],
            ..Rel::default()
        }
    }

    /// The column of `v`.
    pub fn idx(&self, v: &Var) -> Option<usize> {
        self.vars.iter().position(|x| x == v)
    }

    /// The column facts of `v`.
    pub fn col(&self, v: &Var) -> Option<&Col> {
        self.idx(v).map(|i| &self.cols[i])
    }

    /// Adds a column filled with `fill` in every row; returns its index.
    pub fn push_col(&mut self, v: &Var, c: Col, fill: V) -> usize {
        self.vars.push(v.clone());
        self.cols.push(c);
        for r in &mut self.rows {
            r.push(fill.clone());
        }
        self.vars.len() - 1
    }

    /// The value of `v` in row `r` (NULL when the relation has no such column).
    pub fn get<'r>(&self, row: &'r [V], v: &Var) -> Option<&'r V> {
        self.idx(v).map(|i| &row[i])
    }
}
