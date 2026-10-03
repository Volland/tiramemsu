//! Planning: parameters, views, constants, normalisation, analysis and routing.
//!
//! The planner turns a bound [`tm_ir::IrQuery`] into a [`Node`] tree whose
//! constants are encoded, whose views are resolved and in which patterns that
//! cannot match have become [`Node::Empty`].

pub mod analyze;
pub mod bind;
pub mod encode;
pub mod normalize;
pub mod resolve;
pub mod route;

use tm_core::{ObjectId, Value};
use tm_ir::{AggFunc, ArithOp, CmpOp, Func, LookupMode, Var};

use crate::result::RouteNote;
use crate::scan::ResolvedView;
use crate::virtual_pred::VirtualPred;

/// A pattern position after encoding.
#[derive(Clone, Debug, PartialEq)]
pub enum PTerm {
    /// A variable.
    Var(Var),
    /// An encoded constant.
    Id(ObjectId),
}

impl PTerm {
    /// The variable, if this is one.
    pub fn var(&self) -> Option<&Var> {
        match self {
            PTerm::Var(v) => Some(v),
            PTerm::Id(_) => None,
        }
    }
}

/// A stored-triple pattern.
#[derive(Clone, Debug, PartialEq)]
pub struct PTriple {
    /// Subject.
    pub s: PTerm,
    /// Predicate.
    pub p: PTerm,
    /// Object.
    pub o: PTerm,
    /// Eid variable.
    pub eid: Option<Var>,
    /// Resolved view.
    pub view: ResolvedView,
    /// Match group (only under relationship isomorphism).
    pub iso_group: Option<u32>,
    /// Add the canonical-eid predicate (set semantics, design D10 / D27).
    pub canonical: bool,
}

/// The object of a virtual-predicate pattern.
#[derive(Clone, Debug, PartialEq)]
pub enum PObj {
    /// A variable bound to the computed object.
    Var(Var),
    /// A constant, as the column value it compares with.
    Column(i64),
}

/// A virtual-predicate pattern.
#[derive(Clone, Debug, PartialEq)]
pub struct PVirtual {
    /// The statement: an eid variable or a statement id.
    pub subject: PTerm,
    /// The predicate.
    pub pred: VirtualPred,
    /// The object.
    pub object: PObj,
    /// Resolved view of the statement.
    pub view: ResolvedView,
}

/// A pattern that also matches volatile values (`{Now, Unfiltered}` only).
#[derive(Clone, Debug, PartialEq)]
pub struct PVolatile {
    /// Subject.
    pub s: PTerm,
    /// The key / predicate.
    pub p: ObjectId,
    /// Object.
    pub o: PTerm,
}

/// A routed path pattern: `tm_path(arg, path, mode, max_hops, view)` binds `other`
/// from its `"end"` column.
#[derive(Clone, Debug, PartialEq)]
pub struct PPath {
    /// The endpoint the call starts from (the start, or the end when inverted).
    pub arg: PTerm,
    /// The other endpoint.
    pub other: PTerm,
    /// Path text (already inverted when routed from the end).
    pub text: String,
    /// Mode text.
    pub mode: &'static str,
    /// Hop cap.
    pub max_hops: Option<u32>,
    /// Path variable.
    pub bind_path: Option<Var>,
    /// View text.
    pub view_text: String,
    /// Forward or inverted.
    pub note: RouteNote,
    /// The graph selection (the `graphs` argument of the call).
    pub graphs: PGraphs,
    /// The resolved view (for the graph enumeration of [`PGraphs::Var`]).
    pub view: ResolvedView,
    /// The id of `sys:inGraph`; `None` when no membership was ever written.
    pub in_graph: Option<ObjectId>,
}

/// A text recall: `tm_text(query, mode, view, graphs, limit)` binds the eid and
/// the optional score, rank and confidence columns.
#[derive(Clone, Debug, PartialEq)]
pub struct PText {
    /// The words.
    pub query: String,
    /// `all`, `any` or `phrase`.
    pub mode: &'static str,
    /// View text (as for `tm_path`).
    pub view_text: String,
    /// The graph filter: `None` = any statement.
    pub graphs: Option<Vec<ObjectId>>,
    /// Hit limit.
    pub limit: Option<u32>,
    /// Eid variable.
    pub eid: Var,
    /// Score variable.
    pub score: Option<Var>,
    /// Rank variable.
    pub rank: Option<Var>,
    /// Confidence variable.
    pub confidence: Option<Var>,
}

/// The graph selection of a routed path.
#[derive(Clone, Debug, PartialEq)]
pub enum PGraphs {
    /// No graph filter: the call gets no `graphs` argument.
    Any,
    /// Every traversed statement is in one of these graphs. Graphs missing from the
    /// dictionary are dropped, so the list may be empty (only zero-hop rows).
    Ids(Vec<ObjectId>),
    /// The path lies in the graph bound to the variable: a column of the join when
    /// a pattern of it binds the variable, otherwise each graph of the view in turn.
    Var(Var),
}

/// A Values cell after encoding.
#[derive(Clone, Debug, PartialEq)]
pub enum Cell {
    /// A term (possibly a plan-local id for a value missing from the dictionary).
    Id(ObjectId),
    /// A computed integer (the count of an aggregate over an empty input).
    Int(i64),
    /// An empty list (`collect` over an empty input).
    EmptyList,
}

/// Inline rows after encoding.
#[derive(Clone, Debug, PartialEq)]
pub struct PValues {
    /// The variables.
    pub vars: Vec<Var>,
    /// The rows (`None` = UNDEF).
    pub rows: Vec<Vec<Option<Cell>>>,
}

/// A constant of an expression.
#[derive(Clone, Debug, PartialEq)]
pub struct PConst {
    /// The canonical value.
    pub value: Value,
    /// Its ObjectId: stored, inline, or plan-local; `None` for a plain string,
    /// IRI or number that is not in the dictionary (compared by value only).
    pub id: Option<ObjectId>,
}

/// A correlated lookup.
#[derive(Clone, Debug, PartialEq)]
pub struct PLookup {
    /// Subject expression.
    pub subject: Box<PExpr>,
    /// Encoded predicate.
    pub pred: ObjectId,
    /// Resolved view.
    pub view: ResolvedView,
    /// Multiplicity.
    pub multi: LookupMode,
    /// Also read volatile values (only honoured under `{Now, Unfiltered}`).
    pub volatile: bool,
}

/// A planned expression.
#[derive(Clone, Debug, PartialEq)]
pub enum PExpr {
    /// A variable.
    Var(Var),
    /// A constant.
    Const(PConst),
    /// A constant truth value (folded existence tests).
    Bool(bool),
    /// A missing value.
    Null,
    /// Comparison.
    Cmp(CmpOp, Box<PExpr>, Box<PExpr>),
    /// Term identity.
    SameTerm(Box<PExpr>, Box<PExpr>),
    /// Conjunction.
    And(Vec<PExpr>),
    /// Disjunction.
    Or(Vec<PExpr>),
    /// Negation.
    Not(Box<PExpr>),
    /// Bound test.
    Bound(Var),
    /// Membership.
    In(Box<PExpr>, Vec<PExpr>, bool),
    /// Arithmetic.
    Arith(ArithOp, Box<PExpr>, Box<PExpr>),
    /// Unary minus.
    Neg(Box<PExpr>),
    /// First non-missing.
    Coalesce(Vec<PExpr>),
    /// Conditional.
    If(Box<PExpr>, Box<PExpr>, Box<PExpr>),
    /// Built-in function.
    Func(Func, Vec<PExpr>),
    /// Existence test.
    Exists(Box<Node>, bool),
    /// Lookup.
    Lookup(PLookup),
    /// List literal.
    List(Vec<PExpr>),
}

/// A planned aggregate.
#[derive(Clone, Debug, PartialEq)]
pub struct PAgg {
    /// Output variable.
    pub var: Var,
    /// Function.
    pub func: AggFunc,
    /// Argument.
    pub arg: Option<PExpr>,
    /// Distinct.
    pub distinct: bool,
}

/// A planned sort key.
#[derive(Clone, Debug, PartialEq)]
pub struct PKey {
    /// Expression.
    pub expr: PExpr,
    /// Descending.
    pub desc: bool,
}

/// The planned operator tree.
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    /// No rows; exposes these variables.
    Empty(Vec<Var>),
    /// Stored triples.
    Triple(PTriple),
    /// A virtual predicate.
    Virtual(PVirtual),
    /// Stored triples plus volatile values.
    Volatile(PVolatile),
    /// A path pattern (native region).
    Path(PPath),
    /// A text recall (`tm_text`).
    Text(PText),
    /// Inline rows.
    Values(PValues),
    /// List unnesting.
    Unnest(Box<Node>, PExpr, Var),
    /// Natural join with null-safe variables.
    Join(Vec<Node>, Vec<Var>),
    /// Optional join.
    LeftJoin(Box<Node>, Box<Node>, Option<PExpr>),
    /// Filter.
    Filter(Box<Node>, PExpr),
    /// Bag union.
    Union(Vec<Node>),
    /// Computed variable.
    Extend(Box<Node>, Var, PExpr),
    /// Grouping.
    Aggregate(Box<Node>, Vec<Var>, Vec<PAgg>),
    /// Projection.
    Project(Box<Node>, Vec<Var>, bool),
    /// Order, skip, limit.
    OrderLimit(Box<Node>, Vec<PKey>, Option<u64>, Option<u64>),
    /// Row numbering.
    RowNumber(Box<Node>, Vec<Var>, Vec<PKey>, Var),
    /// The input plus always-missing variables (an empty optional side or union
    /// branch).
    PadMissing(Box<Node>, Vec<Var>),
}
