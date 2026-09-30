//! Expressions and functions → IR `Expr` (design D5b).

use spargebra::algebra::{Expression, Function};
use tm_core::Result;
use tm_ir::{ArithOp, CmpOp, Expr, Func};

use super::Lowerer;
use crate::dataset::ViewScope;
use crate::error::unsupported;
use crate::terms;

fn bx(e: Expr) -> Box<Expr> {
    Box::new(e)
}

/// Flattens nested applications of the same associative operator.
fn push_flat(op_and: bool, e: Expr, out: &mut Vec<Expr>) {
    match e {
        Expr::And(xs) if op_and => out.extend(xs),
        Expr::Or(xs) if !op_and => out.extend(xs),
        other => out.push(other),
    }
}

impl Lowerer<'_> {
    /// Lowers an expression read in the time scope `sc` (for `EXISTS`).
    pub fn expr(&mut self, e: &Expression, sc: ViewScope) -> Result<Expr> {
        let bin = |l: &Expression, r: &Expression, this: &mut Self| -> Result<(Expr, Expr)> {
            Ok((this.expr(l, sc)?, this.expr(r, sc)?))
        };
        Ok(match e {
            Expression::NamedNode(n) => Expr::Const(terms::named_node(n)),
            Expression::Literal(l) => Expr::Const(terms::literal(l)?),
            Expression::Variable(v) => Expr::Var(self.var(v.as_str())),
            Expression::Or(l, r) => {
                let (a, b) = bin(l, r, self)?;
                let mut xs = Vec::new();
                push_flat(false, a, &mut xs);
                push_flat(false, b, &mut xs);
                Expr::Or(xs)
            }
            Expression::And(l, r) => {
                let (a, b) = bin(l, r, self)?;
                let mut xs = Vec::new();
                push_flat(true, a, &mut xs);
                push_flat(true, b, &mut xs);
                Expr::And(xs)
            }
            Expression::Equal(l, r) => {
                let (a, b) = bin(l, r, self)?;
                Expr::cmp(CmpOp::Eq, a, b)
            }
            Expression::SameTerm(l, r) => {
                let (a, b) = bin(l, r, self)?;
                Expr::SameTerm(bx(a), bx(b))
            }
            Expression::Greater(l, r) => {
                let (a, b) = bin(l, r, self)?;
                Expr::cmp(CmpOp::Gt, a, b)
            }
            Expression::GreaterOrEqual(l, r) => {
                let (a, b) = bin(l, r, self)?;
                Expr::cmp(CmpOp::Ge, a, b)
            }
            Expression::Less(l, r) => {
                let (a, b) = bin(l, r, self)?;
                Expr::cmp(CmpOp::Lt, a, b)
            }
            Expression::LessOrEqual(l, r) => {
                let (a, b) = bin(l, r, self)?;
                Expr::cmp(CmpOp::Le, a, b)
            }
            Expression::In(x, list) => {
                let x = self.expr(x, sc)?;
                let xs: Result<Vec<Expr>> = list.iter().map(|i| self.expr(i, sc)).collect();
                Expr::In(bx(x), xs?, false)
            }
            Expression::Add(l, r) => {
                let (a, b) = bin(l, r, self)?;
                self.arith(ArithOp::Add, a, b)
            }
            Expression::Subtract(l, r) => {
                let (a, b) = bin(l, r, self)?;
                self.arith(ArithOp::Sub, a, b)
            }
            Expression::Multiply(l, r) => {
                let (a, b) = bin(l, r, self)?;
                self.arith(ArithOp::Mul, a, b)
            }
            Expression::Divide(l, r) => {
                let (a, b) = bin(l, r, self)?;
                self.arith(ArithOp::Div, a, b)
            }
            Expression::UnaryPlus(x) => self.expr(x, sc)?,
            Expression::UnaryMinus(x) => Expr::Neg(bx(self.expr(x, sc)?)),
            Expression::Not(x) => match &**x {
                Expression::Exists(p) => {
                    let op = self.pattern(p, sc)?;
                    Expr::not_exists(op)
                }
                Expression::In(a, list) => {
                    let a = self.expr(a, sc)?;
                    let xs: Result<Vec<Expr>> = list.iter().map(|i| self.expr(i, sc)).collect();
                    Expr::In(bx(a), xs?, true)
                }
                other => Expr::not(self.expr(other, sc)?),
            },
            Expression::Exists(p) => {
                let op = self.pattern(p, sc)?;
                Expr::exists(op)
            }
            Expression::Bound(v) => Expr::Bound(self.var(v.as_str())),
            Expression::If(c, a, b) => {
                let c = self.expr(c, sc)?;
                let a = self.expr(a, sc)?;
                let b = self.expr(b, sc)?;
                Expr::If(bx(c), bx(a), bx(b))
            }
            Expression::Coalesce(xs) => {
                let xs: Result<Vec<Expr>> = xs.iter().map(|i| self.expr(i, sc)).collect();
                let mut xs = xs?;
                match xs.len() {
                    // COALESCE() is an error: an internal variable that is never bound
                    0 => Expr::Var(self.vars.fresh("u")),
                    // COALESCE(x) is x (the SQL COALESCE needs two arguments)
                    1 => xs.remove(0),
                    _ => Expr::Coalesce(xs),
                }
            }
            Expression::FunctionCall(f, args) => {
                let args: Result<Vec<Expr>> = args.iter().map(|i| self.expr(i, sc)).collect();
                self.function(f, args?)?
            }
        })
    }

    /// `a op b`, re-associating right-nested chains to the left when the text has
    /// no parenthesised operand of that class.
    ///
    /// spargebra 0.4.7 parses `a - b - c` (and `a / b / c`, `a + b - c`, ...) as
    /// `a - (b - c)`: right associative, which is wrong for `-` and `/`. The
    /// algebra cannot tell that from a written `a - (b - c)`, so the chain is
    /// rebuilt left-associatively only when the query text has no operator of the
    /// same class followed by `(` ([`crate::parse::assoc_hints`]).
    fn arith(&self, op: ArithOp, a: Expr, b: Expr) -> Expr {
        let additive = matches!(op, ArithOp::Add | ArithOp::Sub);
        let allowed = if additive {
            self.assoc.additive
        } else {
            self.assoc.multiplicative
        };
        let same_class = |o: ArithOp| matches!(o, ArithOp::Add | ArithOp::Sub) == additive;
        if allowed {
            if let Expr::Arith(inner_op, x, y) = b {
                if same_class(inner_op) {
                    // a op (x inner y)  =>  (a op x) inner y
                    let left = self.arith(op, a, *x);
                    return self.arith(inner_op, left, *y);
                }
                return Expr::Arith(op, bx(a), bx(Expr::Arith(inner_op, x, y)));
            }
        }
        Expr::Arith(op, bx(a), bx(b))
    }

    /// A function call over lowered arguments (the whitelist of the `sparql-query`
    /// "Built-in functions" requirement).
    fn function(&mut self, f: &Function, args: Vec<Expr>) -> Result<Expr> {
        let plain = |func: Func, args: Vec<Expr>| Expr::Func(func, args);
        Ok(match f {
            Function::Str => plain(Func::Str, args),
            Function::Lang => plain(Func::Lang, args),
            Function::LangMatches => plain(Func::LangMatches, args),
            Function::Datatype => plain(Func::Datatype, args),
            Function::Iri => plain(Func::Iri, args),
            Function::StrDt => plain(Func::StrDt, args),
            Function::StrLang => plain(Func::StrLang, args),
            Function::StrLen => plain(Func::StrLen, args),
            Function::SubStr => plain(Func::Substr, args),
            Function::UCase => plain(Func::UCase, args),
            Function::LCase => plain(Func::LCase, args),
            Function::StrStarts => plain(Func::StrStarts, args),
            Function::StrEnds => plain(Func::StrEnds, args),
            Function::Contains => plain(Func::Contains, args),
            Function::StrBefore => plain(Func::StrBefore, args),
            Function::StrAfter => plain(Func::StrAfter, args),
            Function::Concat => plain(Func::Concat, args),
            Function::EncodeForUri => plain(Func::EncodeForUri, args),
            Function::Regex => plain(Func::Regex, args),
            Function::Replace => plain(Func::Replace, args),
            Function::Abs => plain(Func::Abs, args),
            Function::Ceil => plain(Func::Ceil, args),
            Function::Floor => plain(Func::Floor, args),
            Function::Round => plain(Func::Round, args),
            Function::Year => plain(Func::Year, args),
            Function::Month => plain(Func::Month, args),
            Function::Day => plain(Func::Day, args),
            Function::Hours => plain(Func::Hours, args),
            Function::Minutes => plain(Func::Minutes, args),
            Function::Seconds => plain(Func::Seconds, args),
            Function::Timezone => plain(Func::Timezone, args),
            Function::Tz => plain(Func::Tz, args),
            Function::Now => Expr::Const(self.now()),
            Function::IsIri => plain(Func::IsIri, args),
            Function::IsBlank => plain(Func::IsBlank, args),
            Function::IsLiteral => plain(Func::IsLiteral, args),
            Function::IsNumeric => plain(Func::IsNumeric, args),
            Function::Custom(n) => match cast_of(n.as_str()) {
                Some(func) => plain(func, args),
                None => return Err(unsupported(n.as_str())),
            },
            Function::BNode => return Err(unsupported("BNODE")),
            Function::Rand => return Err(unsupported("RAND")),
            Function::Uuid => return Err(unsupported("UUID")),
            Function::StrUuid => return Err(unsupported("STRUUID")),
            Function::Md5 => return Err(unsupported("MD5")),
            Function::Sha1 => return Err(unsupported("SHA1")),
            Function::Sha256 => return Err(unsupported("SHA256")),
            Function::Sha384 => return Err(unsupported("SHA384")),
            Function::Sha512 => return Err(unsupported("SHA512")),
            Function::Triple => return Err(unsupported("TRIPLE")),
            Function::Subject => return Err(unsupported("SUBJECT")),
            Function::Predicate => return Err(unsupported("PREDICATE")),
            Function::Object => return Err(unsupported("OBJECT")),
            Function::IsTriple => return Err(unsupported("isTRIPLE")),
            Function::LangDir => return Err(unsupported("LANGDIR")),
            Function::HasLang => return Err(unsupported("hasLANG")),
            Function::HasLangDir => return Err(unsupported("hasLANGDIR")),
            Function::StrLangDir => return Err(unsupported("STRLANGDIR")),
        })
    }
}

/// The IR cast of an XSD constructor function IRI.
fn cast_of(iri: &str) -> Option<Func> {
    use tm_core::vocab::*;
    Some(match iri {
        XSD_STRING => Func::CastString,
        XSD_INTEGER => Func::CastInteger,
        XSD_DECIMAL => Func::CastDecimal,
        XSD_DOUBLE => Func::CastDouble,
        XSD_BOOLEAN => Func::CastBoolean,
        XSD_DATE => Func::CastDate,
        XSD_DATETIME => Func::CastDateTime,
        _ => return None,
    })
}
