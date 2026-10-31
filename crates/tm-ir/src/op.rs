//! The logical operators (`lat.md/query#Logical IR`).

use crate::agg::{Agg, Key};
use crate::expr::Expr;
use crate::path::{PathExpr, PathMode, TemporalPath};
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
    /// Time-respecting evaluation with an optional arrival binding (`None` =
    /// ordinary). It needs a bound start: a journey is not run backwards from its
    /// end.
    pub time_respecting: Option<TemporalPath>,
    /// `max_hops` is the configured hop cap applied to an unbounded expression (the
    /// Cypher `*`), not a bound the query wrote: a search it stops is reported as
    /// [`PathCompleteness::StoppedAtCap`](crate::PathCompleteness::StoppedAtCap).
    pub hop_cap: bool,
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

/// The language of a goal call's program (`lat.md/query#Datalog Calls`).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum GoalLanguage {
    /// A Datalog program (`dl:source`), run by `tm-datalog`.
    #[default]
    Datalog,
    /// An ELPI program (`elpi:source`), run by `tm-elpi` (`lat.md/elpi#Own Calls`).
    Elpi,
}

/// Which Datalog program a goal call runs: a stored program's IRI or program text.
#[derive(Clone, Debug, PartialEq)]
pub enum DlProgramRef {
    /// The node holding `(node dl:source "<text>")`: an IRI constant or a parameter.
    Iri(TermOrVar),
    /// Program text.
    Text(String),
}

/// A Datalog goal call: one solution per goal row and maximal valid interval
/// (`lat.md/query#Datalog Calls`). Executed once per query execution by the
/// `tm_datalog` table function on SQLite and by the native runner on redb, both
/// through the facade's provider over the same engine as `View::datalog`.
///
/// The leaf takes no outer binding: its program and goal are constants, so a
/// planner never pushes a variable into it.
#[derive(Clone, Debug, PartialEq)]
pub struct DatalogPattern {
    /// The language of the program.
    pub language: GoalLanguage,
    /// The program to run.
    pub program: DlProgramRef,
    /// The goal body, such as `reports(ex:alice, X)`.
    pub goal: String,
    /// The goal variables to bind: goal variable name and the query variable it binds.
    pub outputs: Vec<(String, Var)>,
    /// Binds the start and the end of each valid interval (`xsd:dateTime`,
    /// unbound for an open end).
    pub valid: Option<(Var, Var)>,
    /// The view the program reads.
    pub view: View,
}

impl DatalogPattern {
    /// A call of `program` for `goal` under `view` binding nothing yet.
    pub fn new(program: DlProgramRef, goal: impl Into<String>, view: View) -> DatalogPattern {
        DatalogPattern {
            language: GoalLanguage::Datalog,
            program,
            goal: goal.into(),
            outputs: Vec::new(),
            valid: None,
            view,
        }
    }
}

/// The variable names of a Datalog goal body in first-occurrence order, read
/// textually so a front end needs no Datalog parser: an identifier that starts
/// upper-case or with `_` is a variable, except a bare `_`, a prefix name's
/// prefix or local part, text inside string literals and inside `<iri>` references.
///
/// ```
/// use tm_ir::goal_variables;
/// assert_eq!(goal_variables("reports(ex:Alice, X), mail(X, \"Y\", _M, _)"), ["X", "_M"]);
/// ```
pub fn goal_variables(goal: &str) -> Vec<String> {
    let b = goal.as_bytes();
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let (mut i, mut out) = (0, Vec::<String>::new());
    while i < b.len() {
        let c = b[i];
        match c {
            b'"' | b'\'' => {
                i += 1;
                while i < b.len() && b[i] != c {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
                i += 1;
            }
            b'<' => {
                // an IRI reference has no whitespace before its `>`
                match goal[i + 1..].find(|x: char| x == '>' || x.is_whitespace() || x == '<') {
                    Some(n) if b[i + 1 + n] == b'>' => i += n + 2,
                    _ => i += 1,
                }
            }
            c if word(c) => {
                let start = i;
                while i < b.len() && word(b[i]) {
                    i += 1;
                }
                if i < b.len() && b[i] == b':' {
                    // a prefix name: skip its local part
                    i += 1;
                    while i < b.len()
                        && (word(b[i])
                            || b[i] == b'-'
                            || (b[i] == b'.' && i + 1 < b.len() && word(b[i + 1])))
                    {
                        i += 1;
                    }
                    continue;
                }
                let w = &goal[start..i];
                let first = b[start];
                if (first.is_ascii_uppercase() || first == b'_')
                    && w != "_"
                    && !out.iter().any(|x| x == w)
                {
                    out.push(w.to_string());
                }
            }
            _ => i += 1,
        }
    }
    out
}

/// Vector recall: one solution per visible statement with an embedding under
/// `model`, nearest to `vector` first, in the order of the store's ranking policy
/// (`lat.md/query#Vector Recall`). Exact over the statements its view selects.
#[derive(Clone, Debug, PartialEq)]
pub struct VectorPattern {
    /// The model: an IRI constant or a parameter.
    pub model: TermOrVar,
    /// The query vector: a string constant holding a JSON array of numbers, or a
    /// parameter (a list of numbers or such a string).
    pub vector: TermOrVar,
    /// Binds the statement's eid.
    pub eid: Var,
    /// Binds the similarity (a double; larger is closer).
    pub similarity: Option<Var>,
    /// Binds the 1-based rank under the ranking policy.
    pub rank: Option<Var>,
    /// Binds the confidence layer (a double), missing when the statement has none.
    pub confidence: Option<Var>,
    /// Keeps the first `limit` hits by rank (default 10).
    pub limit: Option<u32>,
    /// The pattern's own time selection.
    pub view: View,
    /// The graphs the statement must be a member of (`Any` by default).
    pub graph: GraphSel,
    /// Only statements with one of these predicates (IRI constants or
    /// parameters); empty = any predicate.
    pub predicates: Vec<TermOrVar>,
}

impl VectorPattern {
    /// A recall of the statements nearest to `vector` under `model`, binding `eid`.
    pub fn new(
        model: impl Into<TermOrVar>,
        vector: impl Into<TermOrVar>,
        eid: &str,
        view: View,
    ) -> VectorPattern {
        VectorPattern {
            model: model.into(),
            vector: vector.into(),
            eid: Var::new(eid),
            similarity: None,
            rank: None,
            confidence: None,
            limit: None,
            view,
            graph: GraphSel::Any,
            predicates: Vec::new(),
        }
    }
}

/// Hybrid recall: text and vector recall fused by reciprocal rank, optionally
/// expanded to each hit's layers and subject facts (`lat.md/query#Hybrid Recall`).
/// One solution per result statement, in rank order.
#[derive(Clone, Debug, PartialEq)]
pub struct RecallPattern {
    /// The words of the text channel: a string constant or a parameter; `None`
    /// for no text channel.
    pub text: Option<TermOrVar>,
    /// How the words combine.
    pub mode: tm_core::TextMode,
    /// The model of the vector channel (an IRI constant or a parameter).
    pub model: Option<TermOrVar>,
    /// The query vector (a JSON-array string constant or a parameter).
    pub vector: Option<TermOrVar>,
    /// Binds the statement's eid.
    pub eid: Var,
    /// Binds the fused score (a double; 0 for an expanded statement).
    pub score: Option<Var>,
    /// Binds the 1-based rank.
    pub rank: Option<Var>,
    /// Binds the eid of the fused hit an expanded statement was reached from;
    /// missing for fused hits.
    pub via: Option<Var>,
    /// At most this many fused hits (default 10).
    pub limit: Option<u32>,
    /// Layers and subject facts added per fused hit (0 = none).
    pub expand: u32,
    /// The pattern's own time selection.
    pub view: View,
    /// The graphs the channels' statements must be members of (`Any` by default).
    pub graph: GraphSel,
}

impl RecallPattern {
    /// A recall binding `eid`, with no channel yet.
    pub fn new(eid: &str, view: View) -> RecallPattern {
        RecallPattern {
            text: None,
            mode: tm_core::TextMode::All,
            model: None,
            vector: None,
            eid: Var::new(eid),
            score: None,
            rank: None,
            via: None,
            limit: None,
            expand: 0,
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
/// Leaves are [`TriplePattern`], [`PathPattern`], [`TextPattern`], [`DatalogPattern`], [`VectorPattern`] and
/// [`Values`]; the rest are
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
    /// A Datalog goal call (leaf).
    Datalog(DatalogPattern),
    /// Vector recall (leaf).
    Vector(VectorPattern),
    /// Hybrid recall (leaf).
    Recall(RecallPattern),
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

    /// The direct children of this operator, mutably (not inside expressions).
    pub fn children_mut(&mut self) -> Vec<&mut Op> {
        match self {
            Op::Triple(_)
            | Op::Path(_)
            | Op::Text(_)
            | Op::Datalog(_)
            | Op::Vector(_)
            | Op::Recall(_)
            | Op::Values(_) => Vec::new(),
            Op::Join(j) => j.inputs.iter_mut().collect(),
            Op::Union(u) => u.inputs.iter_mut().collect(),
            Op::LeftJoin(l) => vec![&mut *l.left, &mut *l.right],
            Op::Unnest(x) => vec![&mut *x.input],
            Op::Filter(x) => vec![&mut *x.input],
            Op::Extend(x) => vec![&mut *x.input],
            Op::Aggregate(x) => vec![&mut *x.input],
            Op::Project(x) => vec![&mut *x.input],
            Op::OrderLimit(x) => vec![&mut *x.input],
            Op::RowNumber(x) => vec![&mut *x.input],
        }
    }

    /// The direct children of this operator (not inside expressions).
    pub fn children(&self) -> Vec<&Op> {
        match self {
            Op::Triple(_)
            | Op::Path(_)
            | Op::Text(_)
            | Op::Datalog(_)
            | Op::Vector(_)
            | Op::Recall(_)
            | Op::Values(_) => Vec::new(),
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

impl From<DatalogPattern> for Op {
    fn from(t: DatalogPattern) -> Op {
        Op::Datalog(t)
    }
}

impl From<VectorPattern> for Op {
    fn from(t: VectorPattern) -> Op {
        Op::Vector(t)
    }
}

impl From<RecallPattern> for Op {
    fn from(t: RecallPattern) -> Op {
        Op::Recall(t)
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
