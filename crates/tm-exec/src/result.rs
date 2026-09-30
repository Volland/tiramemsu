//! Query results, execution statistics and explain output.

use tm_core::{SqlValue, Value};
use tm_ir::Var;

/// One decoded result cell.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum ResultValue {
    /// A term: IRI, node, statement, transaction or literal.
    Term(Value),
    /// A list (from `collect`, a multi-valued lookup, or a list expression).
    List(Vec<Option<ResultValue>>),
}

impl ResultValue {
    /// The term, if this is one.
    pub fn as_term(&self) -> Option<&Value> {
        match self {
            ResultValue::Term(v) => Some(v),
            ResultValue::List(_) => None,
        }
    }

    /// The list, if this is one.
    pub fn as_list(&self) -> Option<&[Option<ResultValue>]> {
        match self {
            ResultValue::List(l) => Some(l),
            ResultValue::Term(_) => None,
        }
    }
}

/// Counters of one execution.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct ExecStats {
    /// Whether the query's SQL statement ran (false when it short-circuited).
    pub sql_executed: bool,
    /// Term decodes served by the term cache.
    pub cache_hits: u64,
    /// Term decodes that read the dictionary.
    pub dictionary_reads: u64,
}

/// The result of a query: columns, rows of optional cells (`None` = missing), and
/// statistics.
#[derive(Clone, Debug, PartialEq)]
pub struct QueryResult {
    /// The result variables, in column order.
    pub columns: Vec<Var>,
    /// The rows; a `None` cell is a missing value.
    pub rows: Vec<Vec<Option<ResultValue>>>,
    /// Execution counters.
    pub stats: ExecStats,
}

impl QueryResult {
    /// The index of the column of `var` (`"x"` or `"?x"`).
    pub fn col(&self, var: &str) -> Option<usize> {
        let v = Var::new(var);
        self.columns.iter().position(|c| *c == v)
    }

    /// The term in row `row`, column `var`, if bound to a term.
    pub fn get(&self, row: usize, var: &str) -> Option<&Value> {
        let c = self.col(var)?;
        self.rows.get(row)?.get(c)?.as_ref()?.as_term()
    }

    /// Every cell of column `var` as a term (`None` for missing or list cells).
    pub fn column(&self, var: &str) -> Vec<Option<Value>> {
        let Some(c) = self.col(var) else {
            return Vec::new();
        };
        self.rows
            .iter()
            .map(|r| r[c].as_ref().and_then(|x| x.as_term().cloned()))
            .collect()
    }

    /// Number of rows.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// True when there are no rows.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// Where a region of the plan runs.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum RegionKind {
    /// Generated SQL (joins over `triple` aliases).
    Sql,
    /// The native path operator, as the `tm_path` table-valued function.
    NativePath,
    /// The LFTJ operator (M4; never produced in M1).
    NativeLftj,
}

/// Why a region was routed the way it was.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum RouteNote {
    /// Ordinary routing.
    #[default]
    None,
    /// A cyclic BGP routed to SQL because LFTJ is disabled.
    CyclicLftjDisabled,
    /// A cyclic BGP routed to SQL because LFTJ is enabled but no operator exists.
    LftjUnavailable,
    /// A path called from its bound start.
    PathForward,
    /// A path called from its bound end with the inverse path.
    PathInverted,
}

/// One region of an explained plan.
#[derive(Clone, Debug, PartialEq)]
pub struct RegionInfo {
    /// Where it runs.
    pub kind: RegionKind,
    /// Routing note.
    pub note: RouteNote,
    /// The SQL aliases of the region (`t0`, `t1`, … or `p0`).
    pub aliases: Vec<String>,
    /// The `EXPLAIN QUERY PLAN` rows that scan this region's aliases, in plan order.
    pub query_plan: Vec<String>,
}

/// The explained plan of a query; the query itself is never stepped.
#[derive(Clone, Debug, PartialEq)]
pub struct Explain {
    /// The regions.
    pub regions: Vec<RegionInfo>,
    /// True when the query short-circuited to a constant result (no SQL).
    pub short_circuit: bool,
    /// The SQL text (`None` when short-circuited).
    pub sql: Option<String>,
    /// The bound parameter values, in `?N` order.
    pub params: Vec<SqlValue>,
    /// The whole `EXPLAIN QUERY PLAN` (detail column), with real parameters bound.
    pub query_plan: Vec<String>,
}

impl Explain {
    /// The first SQL region, if any.
    pub fn sql_region(&self) -> Option<&RegionInfo> {
        self.regions.iter().find(|r| r.kind == RegionKind::Sql)
    }
}
