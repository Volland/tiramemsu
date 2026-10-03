//! The logical operators (`lat.md/query#Logical IR`).

use crate::agg::{Agg, Key};
use crate::expr::Expr;
use crate::path::{PathExpr, PathMode};
use crate::term::TermOrVar;
use crate::var::Var;
use crate::view::View;

/// Which named graphs a triple pattern's statement must be a member of
/// (`lat.md/data-model#Named Graphs`). Membership is read in the pattern's own view.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum GraphSel {
    /// No graph condition: the default graph is the union of every statement.
    #[default]
    Any,
    /// Member of at least one listed graph (constants or parameters). The statement
    /// matches once however many of the listed graphs contain it.
    Set(Vec<TermOrVar>),
    /// Binds the variable to each graph the statement is a member of: one solution
    /// per membership.
    Var(Var),
}

impl GraphSel {
    /// True for [`GraphSel::Any`].
    pub fn is_any(&self) -> bool {
        matches!(self, GraphSel::Any)
    }
}

/// A view-scoped triple pattern: one statement per solution.
///
/// Its own [`View`] decides which statements are visible, so two patterns of
/// one query can look at different points in time. Set `eid` to bind the
/// statement id (needed to reach layers or to count parallel statements).
#[derive(Clone, Debug, PartialEq)]
pub struct TriplePattern {
    /// Subject position.
    pub s: TermOrVar,
    /// Predicate position.
    pub p: TermOrVar,
    /// Object position.
    pub o: TermOrVar,
    /// Binds the statement's eid.
    pub eid: Option<Var>,
    /// The pattern's own time selection.
    pub view: View,
    /// Relationship pattern of MATCH group `n` (relationship isomorphism).
    pub iso_group: Option<u32>,
    /// Also match volatile values (under `{Now, Unfiltered}` with a constant predicate).
    pub include_volatile: bool,
    /// The graph membership the statement must have (`Any` by default).
    pub graph: GraphSel,
}

impl TriplePattern {
    /// A pattern under `view` with no eid, no match group and no volatile values.
    pub fn new(
        s: impl Into<TermOrVar>,
        p: impl Into<TermOrVar>,
        o: impl Into<TermOrVar>,
        view: View,
    ) -> TriplePattern {
        TriplePattern {
            s: s.into(),
            p: p.into(),
            o: o.into(),
            eid: None,
            view,
            iso_group: None,
            include_volatile: false,
            graph: GraphSel::Any,
        }
    }

    /// Restricts the statement to a graph selection.
    pub fn in_graph(mut self, graph: GraphSel) -> TriplePattern {
        self.graph = graph;
        self
    }

    /// Binds the eid to `var`.
    pub fn with_eid(mut self, var: &str) -> TriplePattern {
        self.eid = Some(Var::new(var));
        self
    }

    /// Marks the pattern as a relationship pattern of match group `g`.
    pub fn in_group(mut self, g: u32) -> TriplePattern {
        self.iso_group = Some(g);
        self
    }

    /// Opts in to volatile values.
    pub fn volatile(mut self) -> TriplePattern {
        self.include_volatile = true;
        self
    }
}

/// A path between two endpoints, evaluated by the native path operator.
///
/// Like [`TriplePattern`] it carries its own [`View`]. A recursive path needs a
/// bound endpoint when it is executed by `tm-exec`.
#[derive(Clone, Debug, PartialEq)]
pub struct PathPattern {
    /// Start endpoint.
    pub start: TermOrVar,
    /// End endpoint.
    pub end: TermOrVar,
    /// The path expression.
    pub path: PathExpr,
    /// The enumeration mode.
    pub mode: PathMode,
    /// Explicit hop cap passed to `tm_path` (`None` = the operator's default).
    pub max_hops: Option<u32>,
    /// Binds the path value.
    pub bind_path: Option<Var>,
    /// The pattern's own time selection.
    pub view: View,
    /// The graphs every traversed statement must be a member of (`Any` by default).
    /// `Set` restricts the path to statements in at least one listed graph; `Var`
    /// binds the graph the whole path lies in (one graph per solution). Membership is
    /// read in the pattern's own view.
    pub graph: GraphSel,
}

impl PathPattern {
    /// Restricts the path to a graph selection.
    pub fn in_graph(mut self, graph: GraphSel) -> PathPattern {
        self.graph = graph;
        self
    }
}

/// Text recall: one solution per visible statement whose string object matches
/// `query`, in the order of the store's ranking policy (`lat.md/query#Text
/// Recall`). Executed by the `tm_text` table function, which runs the same recall
/// as the Rust `View::text_search`.
///
/// Like [`TriplePattern`] it carries its own [`View`]; `graph` may be `Any` or a
/// `Set` (a graph variable is invalid).
#[derive(Clone, Debug, PartialEq)]
pub struct TextPattern {
    /// The words: a string constant or a parameter.
    pub query: TermOrVar,
    /// How the words combine.
    pub mode: tm_core::TextMode,
    /// Binds the matching statement's eid.
    pub eid: Var,
    /// Binds the lexical score (a double; larger is better).
    pub score: Option<Var>,
    /// Binds the 1-based rank under the ranking policy.
    pub rank: Option<Var>,
    /// Binds the confidence layer (a double), missing when the statement has none.
    pub confidence: Option<Var>,
    /// Keeps the first `limit` hits by rank.
    pub limit: Option<u32>,
    /// The pattern's own time selection.
    pub view: View,
    /// The graphs the statement must be a member of (`Any` by default).
    pub graph: GraphSel,
}

impl TextPattern {
    /// An all-words recall of `query` binding `eid`, with no other output.
    pub fn new(query: impl Into<TermOrVar>, eid: &str, view: View) -> TextPattern {
        TextPattern {
            query: query.into(),
            mode: tm_core::TextMode::All,
            eid: Var::new(eid),
            score: None,
            rank: None,
            confidence: None,
            limit: None,
            view,
            graph: GraphSel::Any,
        }
    }
}

/// Inline bindings; a `None` cell is UNDEF.
#[derive(Clone, Debug, PartialEq)]
pub struct Values {
    /// The variables, one per column.
    pub vars: Vec<Var>,
    /// The rows; each must have exactly `vars.len()` cells.
    pub rows: Vec<Vec<Option<TermOrVar>>>,
}

/// One row per element of a list (`UNWIND`).
#[derive(Clone, Debug, PartialEq)]
pub struct Unnest {
    /// The input rows.
    pub input: Box<Op>,
    /// The list expression, evaluated per input row.
    pub list: Expr,
    /// The element variable.
    pub var: Var,
}

/// The natural join of its inputs; `[]` is the unit relation.
#[derive(Clone, Debug, PartialEq)]
pub struct Join {
    /// The joined inputs.
    pub inputs: Vec<Op>,
    /// Shared variables on which two missing values are equal.
    pub null_safe: Vec<Var>,
}

/// Keeps every left row, extended by the compatible right rows satisfying `cond`.
#[derive(Clone, Debug, PartialEq)]
pub struct LeftJoin {
    /// The kept side.
    pub left: Box<Op>,
    /// The optional side.
    pub right: Box<Op>,
    /// Condition evaluated as part of the join.
    pub cond: Option<Expr>,
}

/// Keeps rows whose condition is true.
#[derive(Clone, Debug, PartialEq)]
pub struct Filter {
    /// The input.
    pub input: Box<Op>,
    /// The condition.
    pub cond: Expr,
}

/// The bag union of its inputs.
#[derive(Clone, Debug, PartialEq)]
pub struct Union {
    /// The branches.
    pub inputs: Vec<Op>,
}

/// Adds `var := expr`.
#[derive(Clone, Debug, PartialEq)]
pub struct Extend {
    /// The input.
    pub input: Box<Op>,
    /// The new variable (must not be bound by the input).
    pub var: Var,
    /// Its expression.
    pub expr: Expr,
}

/// Groups by `group` and computes `aggs`.
#[derive(Clone, Debug, PartialEq)]
pub struct Aggregate {
    /// The input.
    pub input: Box<Op>,
    /// Grouping variables.
    pub group: Vec<Var>,
    /// Aggregates.
    pub aggs: Vec<Agg>,
}

/// Restricts and orders the columns.
#[derive(Clone, Debug, PartialEq)]
pub struct Project {
    /// The input.
    pub input: Box<Op>,
    /// The output variables in order.
    pub vars: Vec<Var>,
    /// Remove duplicate rows.
    pub distinct: bool,
}

/// Sorts, then skips and limits.
#[derive(Clone, Debug, PartialEq)]
pub struct OrderLimit {
    /// The input.
    pub input: Box<Op>,
    /// Sort keys, by decoded value.
    pub keys: Vec<Key>,
    /// Rows to skip: a non-negative integer constant or a parameter.
    pub skip: Option<TermOrVar>,
    /// Row limit: a non-negative integer constant or a parameter.
    pub limit: Option<TermOrVar>,
}

/// Numbers rows within partitions (`ROW_NUMBER() OVER (…)`), starting at 1.
#[derive(Clone, Debug, PartialEq)]
pub struct RowNumber {
    /// The input.
    pub input: Box<Op>,
    /// Partition variables.
    pub partition: Vec<Var>,
    /// Order within a partition.
    pub order: Vec<Key>,
    /// The row-number variable.
    pub var: Var,
}

/// A logical operator: one node of the query tree.
///
/// Leaves are [`TriplePattern`], [`PathPattern`], [`TextPattern`] and [`Values`]; the rest are
/// relational operators over child operators. Build trees with the constructor
/// methods on `Op` (`Op::join`, [`Op::filter`], `Op::project`, ...) or with
/// [`IrBuilder`](crate::builder::IrBuilder), then check them with
/// [`validate`](crate::validate::validate).
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// A triple pattern.
    Triple(TriplePattern),
    /// A path pattern.
    Path(PathPattern),
    /// A text recall.
    Text(TextPattern),
    /// Inline rows.
    Values(Values),
    /// List unnesting.
    Unnest(Unnest),
    /// Natural join.
    Join(Join),
    /// Optional join.
    LeftJoin(LeftJoin),
    /// Filter.
    Filter(Filter),
    /// Bag union.
    Union(Union),
    /// Computed variable.
    Extend(Extend),
    /// Grouping and aggregation.
    Aggregate(Aggregate),
    /// Projection.
    Project(Project),
    /// Order, skip and limit.
    OrderLimit(OrderLimit),
    /// Row numbering.
    RowNumber(RowNumber),
}

impl Op {
    /// A join of `inputs`.
    pub fn join(inputs: Vec<Op>) -> Op {
        Op::Join(Join {
            inputs,
            null_safe: Vec::new(),
        })
    }

    /// The unit relation (`Join[]`).
    pub fn unit() -> Op {
        Op::join(Vec::new())
    }

    /// `LeftJoin(left, right, cond)`.
    pub fn left_join(left: Op, right: Op, cond: Option<Expr>) -> Op {
        Op::LeftJoin(LeftJoin {
            left: Box::new(left),
            right: Box::new(right),
            cond,
        })
    }

    /// `Filter(cond, self)`.
    pub fn filter(self, cond: Expr) -> Op {
        Op::Filter(Filter {
            input: Box::new(self),
            cond,
        })
    }

    /// `Union(inputs)`.
    pub fn union(inputs: Vec<Op>) -> Op {
        Op::Union(Union { inputs })
    }

    /// `Extend(var := expr, self)`.
    pub fn extend(self, var: &str, expr: Expr) -> Op {
        Op::Extend(Extend {
            input: Box::new(self),
            var: Var::new(var),
            expr,
        })
    }

    /// `Project(vars, self)`.
    pub fn project(self, vars: &[&str]) -> Op {
        Op::Project(Project {
            input: Box::new(self),
            vars: vars.iter().map(Var::new).collect(),
            distinct: false,
        })
    }

    /// `Project{distinct}(vars, self)`.
    pub fn project_distinct(self, vars: &[&str]) -> Op {
        Op::Project(Project {
            input: Box::new(self),
            vars: vars.iter().map(Var::new).collect(),
            distinct: true,
        })
    }

    /// `Aggregate(group, aggs, self)`.
    pub fn aggregate(self, group: &[&str], aggs: Vec<Agg>) -> Op {
        Op::Aggregate(Aggregate {
            input: Box::new(self),
            group: group.iter().map(Var::new).collect(),
            aggs,
        })
    }

    /// `OrderLimit(keys, skip, limit, self)` with constant skip and limit.
    pub fn order_limit(self, keys: Vec<Key>, skip: Option<i64>, limit: Option<i64>) -> Op {
        let c = |n: i64| TermOrVar::Const(tm_core::Value::Int(n));
        Op::OrderLimit(OrderLimit {
            input: Box::new(self),
            keys,
            skip: skip.map(c),
            limit: limit.map(c),
        })
    }

    /// The direct children of this operator (not inside expressions).
    pub fn children(&self) -> Vec<&Op> {
        match self {
            Op::Triple(_) | Op::Path(_) | Op::Text(_) | Op::Values(_) => Vec::new(),
            Op::Join(j) => j.inputs.iter().collect(),
            Op::Union(u) => u.inputs.iter().collect(),
            Op::LeftJoin(l) => vec![&l.left, &l.right],
            Op::Unnest(x) => vec![&x.input],
            Op::Filter(x) => vec![&x.input],
            Op::Extend(x) => vec![&x.input],
            Op::Aggregate(x) => vec![&x.input],
            Op::Project(x) => vec![&x.input],
            Op::OrderLimit(x) => vec![&x.input],
            Op::RowNumber(x) => vec![&x.input],
        }
    }
}

impl From<TriplePattern> for Op {
    fn from(t: TriplePattern) -> Op {
        Op::Triple(t)
    }
}

impl From<TextPattern> for Op {
    fn from(t: TextPattern) -> Op {
        Op::Text(t)
    }
}

impl From<PathPattern> for Op {
    fn from(p: PathPattern) -> Op {
        Op::Path(p)
    }
}

/// A query: an operator tree plus its semantic flags.
///
/// This is the unit a front end hands to the executor. Prefer the
/// [`IrQuery::sparql`] and [`IrQuery::cypher`] presets unless you need a custom
/// flag combination.
///
/// # Example
///
/// ```
/// use tm_ir::builder::IrBuilder;
/// use tm_ir::{GraphSet, IrQuery};
///
/// let b = IrBuilder::cypher();
/// let q = IrQuery::cypher(b.triple("?a", "v:knows", "?b"));
/// assert_eq!(q.semantics.graph_set, GraphSet::BagOfEids);
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct IrQuery {
    /// The root operator.
    pub root: Op,
    /// The semantic flags.
    pub semantics: crate::semantics::Semantics,
}

impl IrQuery {
    /// A query with the given flags.
    pub fn new(root: Op, semantics: crate::semantics::Semantics) -> IrQuery {
        IrQuery { root, semantics }
    }

    /// A query with the SPARQL preset.
    pub fn sparql(root: Op) -> IrQuery {
        IrQuery::new(root, crate::semantics::Semantics::sparql())
    }

    /// A query with the Cypher preset.
    pub fn cypher(root: Op) -> IrQuery {
        IrQuery::new(root, crate::semantics::Semantics::cypher())
    }
}
