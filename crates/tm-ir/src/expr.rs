//! The expression language of Filter, Extend, LeftJoin conditions and keys.

use tm_core::Value;

use crate::op::Op;
use crate::term::TermOrVar;
use crate::var::Var;
use crate::view::View;

/// A comparison operator.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum CmpOp {
    /// `=` (value equality).
    Eq,
    /// `!=`.
    Ne,
    /// `<`.
    Lt,
    /// `<=`.
    Le,
    /// `>`.
    Gt,
    /// `>=`.
    Ge,
}

impl CmpOp {
    /// The SQL / surface symbol.
    pub fn symbol(self) -> &'static str {
        match self {
            CmpOp::Eq => "=",
            CmpOp::Ne => "!=",
            CmpOp::Lt => "<",
            CmpOp::Le => "<=",
            CmpOp::Gt => ">",
            CmpOp::Ge => ">=",
        }
    }
}

/// An arithmetic operator.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum ArithOp {
    /// `+`.
    Add,
    /// `-`.
    Sub,
    /// `*`.
    Mul,
    /// `/` (always a real division).
    Div,
}

impl ArithOp {
    /// The SQL symbol.
    pub fn symbol(self) -> &'static str {
        match self {
            ArithOp::Add => "+",
            ArithOp::Sub => "-",
            ArithOp::Mul => "*",
            ArithOp::Div => "/",
        }
    }
}

/// Built-in scalar functions.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Func {
    /// `STR(x)`: the lexical form (or IRI text).
    Str,
    /// `LANG(x)`: the language tag, `""` for other literals.
    Lang,
    /// `DATATYPE(x)`: the datatype IRI of a literal.
    Datatype,
    /// `isIRI(x)`.
    IsIri,
    /// `isLiteral(x)`.
    IsLiteral,
    /// `isNumeric(x)`.
    IsNumeric,
    /// `STRLEN(x)` in characters.
    StrLen,
    /// `UCASE(x)`.
    UCase,
    /// `LCASE(x)`.
    LCase,
    /// `CONTAINS(x, y)`.
    Contains,
    /// `STRSTARTS(x, y)`.
    StrStarts,
    /// `STRENDS(x, y)`.
    StrEnds,
    /// `REGEX(x, pattern [, flags])`.
    Regex,
}

impl Func {
    /// The surface name.
    pub fn name(self) -> &'static str {
        match self {
            Func::Str => "STR",
            Func::Lang => "LANG",
            Func::Datatype => "DATATYPE",
            Func::IsIri => "isIRI",
            Func::IsLiteral => "isLiteral",
            Func::IsNumeric => "isNumeric",
            Func::StrLen => "STRLEN",
            Func::UCase => "UCASE",
            Func::LCase => "LCASE",
            Func::Contains => "CONTAINS",
            Func::StrStarts => "STRSTARTS",
            Func::StrEnds => "STRENDS",
            Func::Regex => "REGEX",
        }
    }
}

/// How a [`Lookup`] reports several distinct values.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum LookupMode {
    /// The value of the smallest eid.
    #[default]
    Single,
    /// A list of the distinct values in eid order (a single value stays scalar).
    ListIfMany,
}

/// A correlated per-row lookup of the objects of `pred` for `subject` (Cypher `x.k`).
/// It never multiplies rows: zero matches give a missing value.
#[derive(Clone, Debug, PartialEq)]
pub struct Lookup {
    /// The subject expression.
    pub subject: Box<Expr>,
    /// The predicate: a constant IRI, an id or a parameter.
    pub pred: TermOrVar,
    /// The view the lookup reads.
    pub view: View,
    /// Multiplicity mode.
    pub multi: LookupMode,
    /// Also read the volatile value of `(subject, pred)` under `{Now, Unfiltered}`.
    pub include_volatile: bool,
}

/// A scalar expression.
#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    /// A variable.
    Var(Var),
    /// A constant.
    Const(Value),
    /// A named parameter.
    Param(String),
    /// A value comparison.
    Cmp(CmpOp, Box<Expr>, Box<Expr>),
    /// Term identity (`sameTerm`).
    SameTerm(Box<Expr>, Box<Expr>),
    /// Conjunction.
    And(Vec<Expr>),
    /// Disjunction.
    Or(Vec<Expr>),
    /// Negation.
    Not(Box<Expr>),
    /// `BOUND(?v)` / `?v IS NOT NULL`: never unknown.
    Bound(Var),
    /// `x [NOT] IN (list)`; the flag is `negated`.
    In(Box<Expr>, Vec<Expr>, bool),
    /// Arithmetic.
    Arith(ArithOp, Box<Expr>, Box<Expr>),
    /// Unary minus.
    Neg(Box<Expr>),
    /// The first bound argument.
    Coalesce(Vec<Expr>),
    /// `IF(cond, then, else)`.
    If(Box<Expr>, Box<Expr>, Box<Expr>),
    /// A built-in function call.
    Func(Func, Vec<Expr>),
    /// `[NOT] EXISTS { op }`; the flag is `negated`. Correlated on shared variables.
    Exists(Box<Op>, bool),
    /// A correlated property lookup.
    Lookup(Lookup),
    /// A list literal.
    List(Vec<Expr>),
}

impl Expr {
    /// A variable expression.
    pub fn var(name: &str) -> Expr {
        Expr::Var(Var::new(name))
    }

    /// A constant expression.
    pub fn val(v: impl Into<Value>) -> Expr {
        Expr::Const(v.into())
    }

    /// A comparison.
    pub fn cmp(op: CmpOp, a: Expr, b: Expr) -> Expr {
        Expr::Cmp(op, Box::new(a), Box::new(b))
    }

    /// `a = b`.
    pub fn eq(a: Expr, b: Expr) -> Expr {
        Expr::cmp(CmpOp::Eq, a, b)
    }

    /// `a != b`.
    pub fn ne(a: Expr, b: Expr) -> Expr {
        Expr::cmp(CmpOp::Ne, a, b)
    }

    /// `a < b`.
    pub fn lt(a: Expr, b: Expr) -> Expr {
        Expr::cmp(CmpOp::Lt, a, b)
    }

    /// `a > b`.
    pub fn gt(a: Expr, b: Expr) -> Expr {
        Expr::cmp(CmpOp::Gt, a, b)
    }

    /// `a >= b`.
    pub fn ge(a: Expr, b: Expr) -> Expr {
        Expr::cmp(CmpOp::Ge, a, b)
    }

    /// `NOT e`.
    #[allow(clippy::should_implement_trait)]
    pub fn not(e: Expr) -> Expr {
        Expr::Not(Box::new(e))
    }

    /// `EXISTS { op }`.
    pub fn exists(op: Op) -> Expr {
        Expr::Exists(Box::new(op), false)
    }

    /// `NOT EXISTS { op }`.
    pub fn not_exists(op: Op) -> Expr {
        Expr::Exists(Box::new(op), true)
    }

    /// Every variable the expression reads directly (not inside `Exists`).
    pub fn vars(&self, out: &mut Vec<Var>) {
        match self {
            Expr::Var(v) | Expr::Bound(v) => {
                if !out.contains(v) {
                    out.push(v.clone())
                }
            }
            Expr::Const(_) | Expr::Param(_) | Expr::Exists(..) => {}
            Expr::Cmp(_, a, b) | Expr::SameTerm(a, b) | Expr::Arith(_, a, b) => {
                a.vars(out);
                b.vars(out);
            }
            Expr::And(xs) | Expr::Or(xs) | Expr::Coalesce(xs) | Expr::List(xs) => {
                xs.iter().for_each(|x| x.vars(out))
            }
            Expr::Func(_, xs) => xs.iter().for_each(|x| x.vars(out)),
            Expr::Not(a) | Expr::Neg(a) => a.vars(out),
            Expr::In(a, xs, _) => {
                a.vars(out);
                xs.iter().for_each(|x| x.vars(out));
            }
            Expr::If(a, b, c) => {
                a.vars(out);
                b.vars(out);
                c.vars(out);
            }
            Expr::Lookup(l) => l.subject.vars(out),
        }
    }
}
