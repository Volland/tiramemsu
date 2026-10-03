//! The Cypher subset AST that semantic analysis and lowering work on. Every node
//! that an error can point at carries a [`Span`] into the original query text.

use crate::span::Span;

/// A name with its span. `escaped` names came from backticks.
#[derive(Clone, Debug, PartialEq)]
pub struct Name {
    /// The text without backticks.
    pub text: String,
    /// True when written in backticks.
    pub escaped: bool,
    /// Where it was written.
    pub span: Span,
}

/// One time argument of `USE`.
#[derive(Clone, Debug, PartialEq)]
pub enum TimeArg {
    /// An integer literal.
    Int(i64),
    /// A parameter `$name`.
    Param(String),
    /// `datetime('…')` with its argument text.
    DateTime(String),
    /// `date('…')` with its argument text.
    Date(String),
}

/// The transaction-time part of a `USE` clause.
#[derive(Clone, Debug, PartialEq)]
pub enum TxClause {
    /// `AS OF arg`.
    AsOf(TimeArg, Span),
    /// `HISTORY`.
    History,
}

/// A parsed `USE` extension clause.
#[derive(Clone, Debug, PartialEq)]
pub struct TimeSel {
    /// The transaction selector, if any.
    pub tx: Option<TxClause>,
    /// `VALID AT arg`, if any.
    pub valid: Option<(TimeArg, Span)>,
    /// The whole clause.
    pub span: Span,
}

/// The match mode written after `MATCH`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MatchModeExt {
    /// No keyword, or `DIFFERENT RELATIONSHIPS`.
    Default,
    /// `REPEATABLE ELEMENTS`.
    Repeatable,
}

/// `TIME RESPECTING [AFTER t] [ARRIVAL AS name]` after `MATCH`: the
/// variable-length relationships of the clause become time-respecting journeys
/// (`lat.md/query#Temporal Path Syntax#Cypher Temporal Paths`).
#[derive(Clone, Debug, PartialEq)]
pub struct TemporalMatch {
    /// `AFTER t`: the start instant (integer epoch ms, `$param`, `datetime('…')`
    /// or `date('…')`); `None` is −∞.
    pub after: Option<(TimeArg, Span)>,
    /// `ARRIVAL AS name`: binds the arrival (Integer epoch ms, `null` for −∞).
    pub arrival: Option<Name>,
    /// The extension's text span.
    pub span: Span,
}

/// A whole query: `UNION` branches of single queries.
#[derive(Clone, Debug, PartialEq)]
pub struct Query {
    /// The branches, at least one.
    pub parts: Vec<SingleQuery>,
    /// `unions[i]` combines `parts[i]` and `parts[i+1]`; true = `UNION ALL`.
    pub unions: Vec<bool>,
    /// The whole text.
    pub span: Span,
}

/// A query without `UNION`.
#[derive(Clone, Debug, PartialEq)]
pub struct SingleQuery {
    /// The scope's `USE` clause.
    pub time: Option<TimeSel>,
    /// The clauses in order.
    pub clauses: Vec<Clause>,
    /// The whole text.
    pub span: Span,
}

/// A path pattern part `p = (a)-[r]->(b)`.
#[derive(Clone, Debug, PartialEq)]
pub struct PatternPart {
    /// The path variable.
    pub binding: Option<Name>,
    /// The chain: nodes and relationships alternate, starting and ending with a node.
    pub nodes: Vec<NodePat>,
    /// `rels[i]` connects `nodes[i]` and `nodes[i+1]`.
    pub rels: Vec<RelPat>,
    /// `shortestPath(...)` (`Some(false)`) or `allShortestPaths(...)` (`Some(true)`).
    pub shortest: Option<bool>,
    /// The whole text.
    pub span: Span,
}

/// A node pattern.
#[derive(Clone, Debug, PartialEq)]
pub struct NodePat {
    /// The variable.
    pub var: Option<Name>,
    /// Required labels (conjunction); each item is a disjunction of names.
    pub labels: Vec<Vec<Name>>,
    /// The inline property map.
    pub props: Option<Expr>,
    /// Text span.
    pub span: Span,
}

/// Relationship direction as written.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Dir {
    /// `-[]->`.
    Right,
    /// `<-[]-`.
    Left,
    /// `-[]-`.
    Either,
}

/// A relationship pattern.
#[derive(Clone, Debug, PartialEq)]
pub struct RelPat {
    /// The variable.
    pub var: Option<Name>,
    /// The type alternatives.
    pub types: Vec<Name>,
    /// Direction.
    pub dir: Dir,
    /// The inline property map.
    pub props: Option<Expr>,
    /// The `*m..n` quantifier of a variable-length relationship.
    pub var_len: Option<VarLen>,
    /// Text span.
    pub span: Span,
}

/// The bounds of a variable-length relationship (`*`, `*n`, `*m..n`, `*m..`, `*..n`).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct VarLen {
    /// Minimum number of hops.
    pub min: u32,
    /// Maximum number of hops; `None` for unbounded.
    pub max: Option<u32>,
    /// Text span of the quantifier.
    pub span: Span,
}

/// A pattern: comma-separated parts.
pub type Pattern = Vec<PatternPart>;

/// One projection item.
#[derive(Clone, Debug, PartialEq)]
pub struct ProjItem {
    /// The expression.
    pub expr: Expr,
    /// The alias.
    pub alias: Option<Name>,
    /// The expression text as written (the default column name).
    pub text: String,
    /// Text span.
    pub span: Span,
}

/// A sort item.
#[derive(Clone, Debug, PartialEq)]
pub struct SortItem {
    /// Expression.
    pub expr: Expr,
    /// Descending.
    pub desc: bool,
}

/// `WITH`/`RETURN` body.
#[derive(Clone, Debug, PartialEq)]
pub struct Projection {
    /// `DISTINCT`.
    pub distinct: bool,
    /// `*` present.
    pub star: bool,
    /// Items.
    pub items: Vec<ProjItem>,
    /// `ORDER BY`.
    pub order: Vec<SortItem>,
    /// `SKIP`.
    pub skip: Option<Expr>,
    /// `LIMIT`.
    pub limit: Option<Expr>,
    /// Text span.
    pub span: Span,
}

/// A `SET` item.
#[derive(Clone, Debug, PartialEq)]
pub enum SetItem {
    /// `x.k = v`.
    Prop {
        /// The target expression (`Expr::Property`).
        target: Expr,
        /// The value.
        value: Expr,
        /// Span.
        span: Span,
    },
    /// `x = m`.
    Replace {
        /// Variable.
        target: Name,
        /// The map.
        value: Expr,
        /// Span.
        span: Span,
    },
    /// `x += m`.
    Merge {
        /// Variable.
        target: Name,
        /// The map.
        value: Expr,
        /// Span.
        span: Span,
    },
    /// `n:L1:L2`.
    Labels {
        /// Variable.
        target: Name,
        /// The labels.
        labels: Vec<Name>,
        /// Span.
        span: Span,
    },
}

/// A `REMOVE` item.
#[derive(Clone, Debug, PartialEq)]
pub enum RemoveItem {
    /// `x.k`.
    Prop(Expr),
    /// `n:L`.
    Labels {
        /// Variable.
        target: Name,
        /// Labels.
        labels: Vec<Name>,
    },
}

/// A clause.
#[derive(Clone, Debug, PartialEq)]
pub enum Clause {
    /// `[OPTIONAL] MATCH`.
    Match {
        /// OPTIONAL.
        optional: bool,
        /// Match mode.
        mode: MatchModeExt,
        /// `TIME RESPECTING …`.
        temporal: Option<TemporalMatch>,
        /// The pattern.
        pattern: Pattern,
        /// `WHERE`.
        where_: Option<Expr>,
        /// Span.
        span: Span,
    },
    /// `UNWIND e AS x`.
    Unwind {
        /// The list.
        expr: Expr,
        /// The variable.
        var: Name,
    },
    /// `CREATE`.
    Create {
        /// The pattern.
        pattern: Pattern,
        /// Span.
        span: Span,
    },
    /// `MERGE`.
    Merge {
        /// The pattern part.
        part: PatternPart,
        /// `ON CREATE SET` items.
        on_create: Vec<SetItem>,
        /// `ON MATCH SET` items.
        on_match: Vec<SetItem>,
        /// Span.
        span: Span,
    },
    /// `SET`.
    Set(Vec<SetItem>),
    /// `REMOVE`.
    Remove(Vec<RemoveItem>),
    /// `[DETACH] DELETE`.
    Delete {
        /// DETACH.
        detach: bool,
        /// Targets.
        exprs: Vec<Expr>,
        /// Span.
        span: Span,
    },
    /// `WITH`.
    With {
        /// Body.
        proj: Projection,
        /// `WHERE`.
        where_: Option<Expr>,
    },
    /// `RETURN`.
    Return(Projection),
    /// `CALL { … }`.
    Subquery {
        /// The body.
        body: Box<Query>,
        /// Imported variables when the body starts with `WITH v1, v2` (or `CALL (v1) {}`).
        imports: Vec<Name>,
        /// Span.
        span: Span,
    },
    /// `CALL proc(args) [YIELD …]`.
    Procedure {
        /// The dotted name.
        name: String,
        /// The arguments.
        args: Vec<Expr>,
        /// `YIELD` items: (column, alias).
        yields: Vec<(Name, Option<Name>)>,
        /// Span.
        span: Span,
    },
}

/// A literal.
#[derive(Clone, Debug, PartialEq)]
pub enum Lit {
    /// `null`.
    Null,
    /// Boolean.
    Bool(bool),
    /// Integer.
    Int(i64),
    /// Float.
    Float(f64),
    /// String.
    Str(String),
}

/// Unary operators.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum UnOp {
    /// `+x`.
    Plus,
    /// `-x`.
    Neg,
    /// `NOT x`.
    Not,
}

/// Binary operators.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BinOp {
    /// OR.
    Or,
    /// XOR.
    Xor,
    /// AND.
    And,
    /// `=`.
    Eq,
    /// `<>`.
    Ne,
    /// `<`.
    Lt,
    /// `<=`.
    Le,
    /// `>`.
    Gt,
    /// `>=`.
    Ge,
    /// `=~`.
    Regex,
    /// `IN`.
    In,
    /// `STARTS WITH`.
    StartsWith,
    /// `ENDS WITH`.
    EndsWith,
    /// `CONTAINS`.
    Contains,
    /// `+`.
    Add,
    /// `-`.
    Sub,
    /// `*`.
    Mul,
    /// `/`.
    Div,
    /// `%`.
    Mod,
    /// `^`.
    Pow,
}

/// A map projection item.
#[derive(Clone, Debug, PartialEq)]
pub enum MapProjItem {
    /// `.key`.
    Prop(Name),
    /// `.*`.
    All,
    /// `var`.
    Var(Name),
    /// `key: expr`.
    Entry(Name, Expr),
}

/// An expression with its span.
#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    /// The node.
    pub kind: ExprKind,
    /// Where it was written.
    pub span: Span,
}

/// Expression forms.
#[derive(Clone, Debug, PartialEq)]
pub enum ExprKind {
    /// Literal.
    Lit(Lit),
    /// Variable.
    Var(String),
    /// Parameter.
    Param(String),
    /// `count(*)`.
    CountStar,
    /// List literal.
    List(Vec<Expr>),
    /// Map literal.
    Map(Vec<(Name, Expr)>),
    /// `base {…}`.
    MapProj(Box<Expr>, Vec<MapProjItem>),
    /// Unary.
    Unary(UnOp, Box<Expr>),
    /// Binary.
    Binary(BinOp, Box<Expr>, Box<Expr>),
    /// `IS [NOT] NULL`; the flag is `negated`.
    IsNull(Box<Expr>, bool),
    /// Property access.
    Prop(Box<Expr>, Name),
    /// `e[i]`.
    Index(Box<Expr>, Box<Expr>),
    /// `e[a..b]`.
    Slice(Box<Expr>, Option<Box<Expr>>, Option<Box<Expr>>),
    /// A function call.
    Call {
        /// Dotted name as written.
        name: String,
        /// `DISTINCT`.
        distinct: bool,
        /// Arguments.
        args: Vec<Expr>,
    },
    /// `CASE`.
    Case {
        /// Operand of the simple form.
        operand: Option<Box<Expr>>,
        /// `(when, then)` alternatives.
        alts: Vec<(Expr, Expr)>,
        /// `ELSE`.
        els: Option<Box<Expr>>,
    },
    /// `[x IN list WHERE p | e]`.
    ListComp {
        /// Variable.
        var: Name,
        /// The list.
        list: Box<Expr>,
        /// Predicate.
        pred: Option<Box<Expr>>,
        /// Projection.
        proj: Option<Box<Expr>>,
    },
    /// `all/any/none/single(x IN l WHERE p)`.
    Quantifier {
        /// One of `all`, `any`, `none`, `single`.
        kind: String,
        /// Variable.
        var: Name,
        /// List.
        list: Box<Expr>,
        /// Predicate.
        pred: Box<Expr>,
    },
    /// `reduce(acc = init, x IN l | e)`.
    Reduce {
        /// Accumulator.
        acc: Name,
        /// Initial.
        init: Box<Expr>,
        /// Variable.
        var: Name,
        /// List.
        list: Box<Expr>,
        /// Body.
        body: Box<Expr>,
    },
    /// `EXISTS { … }` (`Query`) or a bare pattern predicate.
    Exists(Box<ExistsBody>),
    /// `n:Label` / `n:A:B` predicate.
    HasLabels(Box<Expr>, Vec<Name>),
}

/// The body of an existential subquery or pattern predicate.
#[derive(Clone, Debug, PartialEq)]
pub enum ExistsBody {
    /// `EXISTS { pattern [WHERE] }` or a pattern predicate.
    Pattern(Pattern, Option<Expr>),
    /// `EXISTS { MATCH … }`.
    Query(Query),
}
