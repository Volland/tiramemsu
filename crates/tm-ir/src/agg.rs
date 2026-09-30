//! Aggregates and sort keys.

use crate::expr::Expr;
use crate::var::Var;

/// An aggregate function.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum AggFunc {
    /// `count(*)` (no argument) or `count(expr)`.
    Count,
    /// Numeric sum.
    Sum,
    /// Numeric average (a double).
    Avg,
    /// Smallest value in value order.
    Min,
    /// Largest value in value order.
    Max,
    /// An arbitrary value.
    Sample,
    /// String concatenation with a separator.
    GroupConcat {
        /// The separator.
        sep: String,
    },
    /// A list of the bound values.
    Collect,
}

impl AggFunc {
    /// The surface name.
    pub fn name(&self) -> &'static str {
        match self {
            AggFunc::Count => "count",
            AggFunc::Sum => "sum",
            AggFunc::Avg => "avg",
            AggFunc::Min => "min",
            AggFunc::Max => "max",
            AggFunc::Sample => "sample",
            AggFunc::GroupConcat { .. } => "group_concat",
            AggFunc::Collect => "collect",
        }
    }
}

/// One aggregate output `var := func([distinct] arg)`.
#[derive(Clone, Debug, PartialEq)]
pub struct Agg {
    /// The output variable.
    pub var: Var,
    /// The function.
    pub func: AggFunc,
    /// The argument; `None` only for `count(*)`.
    pub arg: Option<Expr>,
    /// Aggregate distinct values only.
    pub distinct: bool,
}

impl Agg {
    /// `var := count(*)`.
    pub fn count_star(var: &str) -> Agg {
        Agg {
            var: Var::new(var),
            func: AggFunc::Count,
            arg: None,
            distinct: false,
        }
    }

    /// `var := func(arg)`.
    pub fn new(var: &str, func: AggFunc, arg: Expr) -> Agg {
        Agg {
            var: Var::new(var),
            func,
            arg: Some(arg),
            distinct: false,
        }
    }

    /// The same aggregate over distinct values.
    pub fn distinct(self) -> Agg {
        Agg {
            distinct: true,
            ..self
        }
    }
}

/// A sort key of OrderLimit or RowNumber.
#[derive(Clone, Debug, PartialEq)]
pub struct Key {
    /// The expression sorted by decoded value.
    pub expr: Expr,
    /// Descending order.
    pub descending: bool,
}

impl Key {
    /// Ascending by `expr`.
    pub fn asc(expr: Expr) -> Key {
        Key {
            expr,
            descending: false,
        }
    }

    /// Descending by `expr`.
    pub fn desc(expr: Expr) -> Key {
        Key {
            expr,
            descending: true,
        }
    }
}
