//! Expressions, evaluated per row with the semantics of the SQL the route of
//! `tm_exec::sqlgen::expr` writes: three-valued logic on NULL, value equality and
//! class-checked ordering, datetimes by instant, the same helper functions.
//! Each function here mirrors the `Gen` method of the same name.

use std::cmp::Ordering;

use tm_core::{Result, Tag, Value};
use tm_exec::plan::analyze::{Dom, VClass};
use tm_exec::plan::{PConst, PExpr, PLookup};
use tm_exec::scan::ResolvedTx;
use tm_exec::udf::{kind_of_value, value_key, K_NUM};
use tm_ir::{ArithOp, CmpOp, Func, LookupMode};
use tm_store::Pattern;

use crate::exec::Exec;
use crate::funcs::call;
use crate::rel::Rel;
use crate::value::{self as sv, truth, V};

/// Dictionary tags (whose values need a term lookup).
const DICT_TAGS: [i64; 6] = [0, 10, 11, 12, 13, 14];

/// A compiled scalar value: its SQL value, static domain, nullability, and the
/// constant it came from (constants are folded the way the SQL route binds them).
#[derive(Clone, Debug)]
pub struct Val {
    /// The value.
    pub v: V,
    /// Static domain.
    pub dom: Dom,
    /// May be NULL.
    pub mm: bool,
    /// The constant, if this is one.
    pub konst: Option<PConst>,
    /// The always-NULL column of an absent variable.
    pub null: bool,
}

impl Val {
    fn of(v: V, dom: Dom, mm: bool) -> Val {
        Val {
            v,
            dom,
            mm,
            konst: None,
            null: false,
        }
    }

    fn boolean(v: V, mm: bool) -> Val {
        Val::of(v, Dom::Computed(VClass::Bool), mm)
    }
}

fn tag(x: i64) -> i64 {
    x & 15
}

/// The static class of a constant.
pub fn class_of(v: &Value) -> VClass {
    match v {
        Value::Int(_) => VClass::Int,
        Value::Double(_) | Value::Decimal(_) => VClass::Double,
        Value::Bool(_) => VClass::Bool,
        Value::Iri(_) => VClass::Iri,
        _ => VClass::Str,
    }
}

/// The native SQLite value of a constant, if it has a computed class.
pub fn native(v: &Value) -> Option<V> {
    Some(match v {
        Value::Int(i) => V::Integer(*i),
        Value::Double(x) => V::Real(*x),
        Value::Decimal(s) => V::Real(s.parse().ok()?),
        Value::Bool(b) => V::Integer(*b as i64),
        Value::Str(s) | Value::Iri(s) => V::Text(s.clone()),
        _ => return None,
    })
}

fn compatible(c: VClass, v: &Value) -> bool {
    let vc = class_of(v);
    match c {
        VClass::Int | VClass::Double => vc.numeric(),
        VClass::Dynamic => vc.numeric() || vc == VClass::Str,
        other => other == vc && native(v).is_some(),
    }
}

fn flip(op: CmpOp) -> CmpOp {
    match op {
        CmpOp::Lt => CmpOp::Gt,
        CmpOp::Le => CmpOp::Ge,
        CmpOp::Gt => CmpOp::Lt,
        CmpOp::Ge => CmpOp::Le,
        other => other,
    }
}

/// `a <op> b` for an ordering operator, three-valued.
fn ord(op: CmpOp, a: &V, b: &V) -> V {
    truth(sv::compare(a, b).map(|o| match op {
        CmpOp::Lt => o == Ordering::Less,
        CmpOp::Le => o != Ordering::Greater,
        CmpOp::Gt => o == Ordering::Greater,
        CmpOp::Ge => o != Ordering::Less,
        CmpOp::Eq => o == Ordering::Equal,
        CmpOp::Ne => o != Ordering::Equal,
    }))
}

fn not(v: &V) -> V {
    truth(sv::cond(v).map(|b| !b))
}

fn and(a: &V, b: &V) -> V {
    match (sv::cond(a), sv::cond(b)) {
        (Some(false), _) | (_, Some(false)) => V::Integer(0),
        (Some(true), Some(true)) => V::Integer(1),
        _ => V::Null,
    }
}

fn or(a: &V, b: &V) -> V {
    match (sv::cond(a), sv::cond(b)) {
        (Some(true), _) | (_, Some(true)) => V::Integer(1),
        (Some(false), Some(false)) => V::Integer(0),
        _ => V::Null,
    }
}

/// `CASE WHEN x IS NULL THEN NULL ELSE 0 END`.
fn null_else_false(x: &V) -> V {
    if x.is_null() {
        V::Null
    } else {
        V::Integer(0)
    }
}

impl Exec<'_> {
    /// The SQL value a compiled value stands for (constants as the SQL route
    /// binds them).
    pub fn val_sql(&self, v: &Val) -> V {
        let Some(k) = &v.konst else {
            return v.v.clone();
        };
        match (&v.dom, k.id) {
            (Dom::Term, Some(id)) => V::Integer(id.raw()),
            _ => native(&k.value).unwrap_or(V::Null),
        }
    }

    fn dict_num(&self, x: i64) -> Result<V> {
        Ok(self
            .terms
            .row(x >> 4)?
            .and_then(|t| t.num)
            .map_or(V::Null, V::Real))
    }

    fn dict_lex(&self, x: i64) -> Result<V> {
        Ok(self.terms.row(x >> 4)?.map_or(V::Null, |t| V::Text(t.lex)))
    }

    /// The numeric value of `x` (NULL for non-numbers).
    pub fn num_of(&self, dom: &Dom, x: &V) -> Result<V> {
        Ok(match dom {
            Dom::Term => match x {
                V::Integer(i) if tag(*i) == 5 => V::Integer(i >> 4),
                V::Integer(i) if tag(*i) == 13 || tag(*i) == 14 => self.dict_num(*i)?,
                _ => V::Null,
            },
            Dom::Computed(VClass::Bool | VClass::Str | VClass::Iri | VClass::Lit) => V::Null,
            Dom::Computed(VClass::Dynamic) => match x {
                V::Integer(_) | V::Real(_) => x.clone(),
                _ => V::Null,
            },
            _ => x.clone(),
        })
    }

    /// The string (lexical) value of `x`.
    pub fn str_of(&self, dom: &Dom, x: &V) -> Result<V> {
        Ok(match dom {
            Dom::Term => {
                let lex = match x {
                    V::Integer(i) if DICT_TAGS.contains(&tag(*i)) => self.dict_lex(*i)?,
                    _ => V::Null,
                };
                call("tm_str", &[x.clone(), lex])?
            }
            Dom::Computed(VClass::Str | VClass::Iri) => x.clone(),
            Dom::Computed(VClass::Lit) => call("tm_lit_lex", std::slice::from_ref(x))?,
            Dom::Computed(VClass::Bool) => match sv::cond(x) {
                None => V::Null,
                Some(true) => V::Text("true".into()),
                Some(false) => V::Text("false".into()),
            },
            _ => sv::to_text(x),
        })
    }

    /// The memcmp-ordered value key of `x`.
    pub fn key_of(&self, dom: &Dom, x: &V) -> Result<V> {
        Ok(match dom {
            Dom::Term => match x {
                V::Integer(i) if DICT_TAGS.contains(&tag(*i)) => match self.terms.row(i >> 4)? {
                    Some(t) => call(
                        "tm_sortkey",
                        &[
                            x.clone(),
                            V::Text(t.lex),
                            t.num.map_or(V::Null, V::Real),
                            t.lang.map_or(V::Null, V::Text),
                        ],
                    )?,
                    None => V::Null,
                },
                _ => call("tm_sortkey", &[x.clone(), V::Null, V::Null, V::Null])?,
            },
            Dom::Computed(c) => call("tm_vkey", &[x.clone(), V::Integer(c.code())])?,
            _ => x.clone(),
        })
    }

    fn kind_of(&self, dom: &Dom, x: &V) -> Result<V> {
        Ok(match dom {
            Dom::Term => call("tm_kind", std::slice::from_ref(x))?,
            Dom::Computed(c) => call("tm_vkind", &[x.clone(), V::Integer(c.code())])?,
            _ => V::Null,
        })
    }

    fn num_val(&self, v: &Val) -> Result<V> {
        if let Some(k) = &v.konst {
            return Ok(match native(&k.value) {
                Some(n @ (V::Integer(_) | V::Real(_))) if class_of(&k.value).numeric() => n,
                _ => V::Null,
            });
        }
        self.num_of(&v.dom, &v.v)
    }

    fn str_val(&self, v: &Val) -> Result<V> {
        if let Some(k) = &v.konst {
            return Ok(V::Text(k.value.lexical()));
        }
        self.str_of(&v.dom, &v.v)
    }

    fn common_dom(&self, vals: &[Val]) -> Result<Dom> {
        // the SQL route leaves out the literal NULL of an absent variable
        let doms: Vec<&Dom> = vals.iter().filter(|v| !v.null).map(|v| &v.dom).collect();
        if doms.windows(2).any(|w| w[0] != w[1]) {
            return Err(tm_exec::error::unsupported(
                "an expression mixing stored terms and computed values".to_string(),
            ));
        }
        Ok(doms.first().map_or(Dom::Term, |d| (*d).clone()))
    }

    /// Evaluates a scalar expression on `row` of `rel`.
    pub fn value(&self, e: &PExpr, rel: &Rel, row: &[V]) -> Result<Val> {
        Ok(match e {
            PExpr::Var(v) => match rel.idx(v) {
                Some(i) => {
                    let c = &rel.cols[i];
                    Val {
                        v: row[i].clone(),
                        dom: c.dom.clone(),
                        mm: c.mm,
                        konst: None,
                        null: c.null,
                    }
                }
                None => Val {
                    v: V::Null,
                    dom: Dom::Term,
                    mm: true,
                    konst: None,
                    null: true,
                },
            },
            PExpr::Const(k) => {
                let dom = if k.id.is_some() {
                    Dom::Term
                } else {
                    Dom::Computed(class_of(&k.value))
                };
                let mut v = Val {
                    v: V::Null,
                    dom,
                    mm: false,
                    konst: Some(k.clone()),
                    null: false,
                };
                v.v = self.val_sql(&v);
                v
            }
            PExpr::Bool(b) => Val::boolean(V::Integer(*b as i64), false),
            PExpr::Null => Val {
                v: V::Null,
                dom: Dom::Term,
                mm: true,
                konst: None,
                null: true,
            },
            PExpr::Bound(_) | PExpr::Exists(..) => Val::boolean(self.cond(e, rel, row)?, false),
            PExpr::Cmp(..)
            | PExpr::SameTerm(..)
            | PExpr::And(_)
            | PExpr::Or(_)
            | PExpr::Not(_)
            | PExpr::In(..) => Val::boolean(self.cond(e, rel, row)?, true),
            PExpr::Arith(op, a, b) => {
                let a = self.value(a, rel, row)?;
                let b = self.value(b, rel, row)?;
                let na = self.num_val(&a)?;
                let nb = self.num_val(&b)?;
                let cls = |v: &Val| match (&v.dom, &v.konst) {
                    (_, Some(k)) => class_of(&k.value),
                    (Dom::Computed(c), None) => *c,
                    _ => VClass::Dynamic,
                };
                match op {
                    ArithOp::Div => Val::of(
                        sv::div(&sv::to_real(&na), &nb),
                        Dom::Computed(VClass::Double),
                        true,
                    ),
                    _ => {
                        let c = match (cls(&a), cls(&b)) {
                            (VClass::Int, VClass::Int) => VClass::Int,
                            (VClass::Double, _) | (_, VClass::Double) => VClass::Double,
                            _ => VClass::Dynamic,
                        };
                        let sym = match op {
                            ArithOp::Add => '+',
                            ArithOp::Sub => '-',
                            _ => '*',
                        };
                        Val::of(sv::arith(sym, &na, &nb), Dom::Computed(c), true)
                    }
                }
            }
            PExpr::Neg(a) => {
                let a = self.value(a, rel, row)?;
                let n = self.num_val(&a)?;
                let c = match &a.dom {
                    Dom::Computed(c) if c.numeric() => *c,
                    _ => VClass::Dynamic,
                };
                Val::of(sv::neg(&n), Dom::Computed(c), true)
            }
            PExpr::Coalesce(xs) => {
                let vals: Vec<Val> = xs
                    .iter()
                    .map(|x| self.value(x, rel, row))
                    .collect::<Result<_>>()?;
                let dom = self.common_dom(&vals)?;
                let v = vals
                    .iter()
                    .map(|v| self.val_sql(v))
                    .find(|v| !v.is_null())
                    .unwrap_or(V::Null);
                Val::of(v, dom, vals.iter().all(|v| v.mm))
            }
            PExpr::If(c, a, b) => {
                let c = self.cond(c, rel, row)?;
                let a = self.value(a, rel, row)?;
                let b = self.value(b, rel, row)?;
                let dom = self.common_dom(&[a.clone(), b.clone()])?;
                let v = match sv::cond(&c) {
                    Some(true) => self.val_sql(&a),
                    Some(false) => self.val_sql(&b),
                    None => V::Null,
                };
                Val::of(v, dom, true)
            }
            PExpr::Func(f, args) => self.func(*f, args, rel, row)?,
            PExpr::Lookup(l) => self.lookup(l, rel, row)?,
            PExpr::List(xs) => {
                let vals: Vec<Val> = xs
                    .iter()
                    .map(|x| self.value(x, rel, row))
                    .collect::<Result<_>>()?;
                let elem = if vals.is_empty() {
                    Dom::Term
                } else {
                    self.common_dom(&vals)?
                };
                let parts: Vec<V> = vals.iter().map(|v| self.val_sql(v)).collect();
                Val::of(sv::json_array(&parts), Dom::List(Box::new(elem)), false)
            }
        })
    }

    fn func(&self, f: Func, args: &[PExpr], rel: &Rel, row: &[V]) -> Result<Val> {
        let vals: Vec<Val> = args
            .iter()
            .map(|a| self.value(a, rel, row))
            .collect::<Result<_>>()?;
        let arity = match f {
            Func::Contains
            | Func::StrStarts
            | Func::StrEnds
            | Func::LangMatches
            | Func::StrDt
            | Func::StrLang
            | Func::StrBefore
            | Func::StrAfter => 2..=2,
            Func::Regex | Func::Substr => 2..=3,
            Func::Replace => 3..=4,
            Func::Concat => 0..=usize::MAX,
            _ => 1..=1,
        };
        if !arity.contains(&vals.len()) {
            return Err(tm_exec::error::invalid(format!(
                "{} takes {:?} arguments, got {}",
                f.name(),
                arity,
                vals.len()
            )));
        }
        let str_ = |v: V| Val::of(v, Dom::Computed(VClass::Str), true);
        Ok(match f {
            Func::Str => str_(self.str_val(&vals[0])?),
            Func::Lang => {
                let v = &vals[0];
                let x = self.val_sql(v);
                match v.dom {
                    Dom::Term => {
                        let l = match &x {
                            V::Integer(i) if tag(*i) == 11 => self
                                .terms
                                .row(i >> 4)?
                                .and_then(|t| t.lang)
                                .map_or(V::Null, V::Text),
                            _ => V::Null,
                        };
                        str_(call("tm_lang", &[x.clone(), l])?)
                    }
                    Dom::Computed(VClass::Lit) => str_(call("tm_lit_lang", &[x])?),
                    _ => str_(if x.is_null() {
                        V::Null
                    } else {
                        V::Text(String::new())
                    }),
                }
            }
            Func::Datatype => {
                let v = &vals[0];
                let x = self.val_sql(v);
                let out = match v.dom {
                    Dom::Term => {
                        let dt = match &x {
                            V::Integer(i) if [12, 13, 14].contains(&tag(*i)) => {
                                match self.terms.stored(i >> 4)? {
                                    Some(t) => match t.dt {
                                        Some(d) => self
                                            .terms
                                            .stored(d >> 4)?
                                            .map_or(V::Null, |t| V::Text(t.lex)),
                                        None => V::Null,
                                    },
                                    None => V::Null,
                                }
                            }
                            _ => V::Null,
                        };
                        call("tm_datatype", &[x.clone(), dt])?
                    }
                    Dom::Computed(VClass::Lit) => call("tm_lit_dt", &[x])?,
                    _ => match x {
                        V::Integer(_) => V::Text(tm_core::vocab::XSD_INTEGER.into()),
                        V::Real(_) => V::Text(tm_core::vocab::XSD_DOUBLE.into()),
                        V::Text(_) => V::Text(tm_core::vocab::XSD_STRING.into()),
                        _ => V::Null,
                    },
                };
                Val::of(out, Dom::Computed(VClass::Iri), true)
            }
            Func::IsIri | Func::IsLiteral | Func::IsNumeric => {
                let v = &vals[0];
                let x = self.val_sql(v);
                let tagged = |set: &[i64]| match &x {
                    V::Null => V::Null,
                    V::Integer(i) => truth(Some(set.contains(&tag(*i)))),
                    _ => V::Integer(0),
                };
                let out = match (&v.dom, f) {
                    (Dom::Term, Func::IsIri) => tagged(&[0, 1, 3, 4]),
                    (Dom::Term, Func::IsLiteral) => tagged(&[5, 6, 7, 8, 9, 10, 11, 12, 13, 14]),
                    (Dom::Term, _) => tagged(&[5, 13, 14]),
                    (Dom::Computed(VClass::Iri), Func::IsIri) => {
                        if x.is_null() {
                            V::Null
                        } else {
                            V::Integer(1)
                        }
                    }
                    (Dom::Computed(VClass::Iri), _) | (_, Func::IsIri) => null_else_false(&x),
                    (_, Func::IsLiteral) => {
                        if x.is_null() {
                            V::Null
                        } else {
                            V::Integer(1)
                        }
                    }
                    _ => truth(Some(matches!(x, V::Integer(_) | V::Real(_)))),
                };
                Val::boolean(out, true)
            }
            Func::StrLen => Val::of(
                sv::length(&self.str_val(&vals[0])?),
                Dom::Computed(VClass::Int),
                true,
            ),
            Func::UCase | Func::LCase => {
                let s = self.str_val(&vals[0])?;
                let n = if f == Func::UCase {
                    "tm_ucase"
                } else {
                    "tm_lcase"
                };
                str_(call(n, &[s])?)
            }
            Func::Contains => {
                let a = self.str_val(&vals[0])?;
                let b = self.str_val(&vals[1])?;
                Val::boolean(ord(CmpOp::Gt, &sv::instr(&a, &b), &V::Integer(0)), true)
            }
            Func::StrStarts => {
                let a = self.str_val(&vals[0])?;
                let b = self.str_val(&vals[1])?;
                let out = match sv::length(&b) {
                    V::Integer(n) => sv::eq(&sv::substr(&a, 1, Some(n)), &b),
                    _ => V::Null,
                };
                Val::boolean(if a.is_null() { V::Null } else { out }, true)
            }
            Func::StrEnds => {
                let a = self.str_val(&vals[0])?;
                let b = self.str_val(&vals[1])?;
                let out = if a.is_null() || b.is_null() {
                    V::Null
                } else {
                    match sv::length(&b) {
                        V::Integer(0) => V::Integer(1),
                        V::Integer(n) => sv::eq(&sv::substr(&a, -n, None), &b),
                        _ => V::Null,
                    }
                };
                Val::boolean(out, true)
            }
            Func::Regex => {
                let a = self.str_val(&vals[0])?;
                let p = self.str_val(&vals[1])?;
                let fl = match vals.get(2) {
                    Some(v) => self.str_val(v)?,
                    None => V::Text(String::new()),
                };
                Val::boolean(call("tm_regex", &[a, p, fl])?, true)
            }
            other => self.sparql_func(other, &vals)?,
        })
    }

    fn sparql_func(&self, f: Func, vals: &[Val]) -> Result<Val> {
        use tm_exec::udf_fn::cast;
        let with = |v: V, c: VClass| Val::of(v, Dom::Computed(c), true);
        let arg_class = |v: &Val| match (&v.dom, &v.konst) {
            (_, Some(k)) => match class_of(&k.value) {
                c if c.numeric() => c,
                _ => VClass::Dynamic,
            },
            (Dom::Computed(c), None) if c.numeric() => *c,
            _ => VClass::Dynamic,
        };
        let s = |i: usize| self.str_val(&vals[i]);
        Ok(match f {
            Func::IsBlank => {
                let x = self.val_sql(&vals[0]);
                let out = match (&vals[0].dom, &x) {
                    (_, V::Null) => V::Null,
                    (Dom::Term, V::Integer(i)) => truth(Some(tag(*i) == 2)),
                    _ => V::Integer(0),
                };
                Val::boolean(out, true)
            }
            Func::LangMatches => Val::boolean(call("tm_langmatches", &[s(0)?, s(1)?])?, true),
            Func::Iri => with(s(0)?, VClass::Iri),
            Func::StrDt => with(call("tm_lit", &[s(0)?, s(1)?, V::Integer(0)])?, VClass::Lit),
            Func::StrLang => with(call("tm_lit", &[s(0)?, s(1)?, V::Integer(1)])?, VClass::Lit),
            Func::Substr => {
                let start = self.num_val(&vals[1])?;
                let len = match vals.get(2) {
                    Some(v) => self.num_val(v)?,
                    None => V::Null,
                };
                with(call("tm_substr", &[s(0)?, start, len])?, VClass::Str)
            }
            Func::StrBefore | Func::StrAfter => {
                let (a, b) = (s(0)?, s(1)?);
                let out = if a.is_null() || b.is_null() {
                    V::Null
                } else if b == V::Text(String::new()) {
                    if f == Func::StrBefore {
                        V::Text(String::new())
                    } else {
                        a.clone()
                    }
                } else {
                    match sv::instr(&a, &b) {
                        V::Integer(i) if i > 0 => {
                            if f == Func::StrBefore {
                                sv::substr(&a, 1, Some(i - 1))
                            } else {
                                let lb = match sv::length(&b) {
                                    V::Integer(n) => n,
                                    _ => 0,
                                };
                                sv::substr(&a, i + lb, None)
                            }
                        }
                        _ => V::Text(String::new()),
                    }
                };
                with(out, VClass::Str)
            }
            Func::Concat => {
                let mut acc: Option<V> = None;
                for i in 0..vals.len() {
                    let x = s(i)?;
                    acc = Some(match acc {
                        None => x,
                        Some(a) => sv::concat(&a, &x),
                    });
                }
                with(acc.unwrap_or(V::Text(String::new())), VClass::Str)
            }
            Func::EncodeForUri => with(call("tm_encode_uri", &[s(0)?])?, VClass::Str),
            Func::Replace => {
                let fl = match vals.get(3) {
                    Some(_) => s(3)?,
                    None => V::Text(String::new()),
                };
                with(call("tm_replace", &[s(0)?, s(1)?, s(2)?, fl])?, VClass::Str)
            }
            Func::Abs | Func::Ceil | Func::Floor | Func::Round => {
                let c = arg_class(&vals[0]);
                let n = self.num_val(&vals[0])?;
                let out = match f {
                    Func::Abs => sv::abs(&n),
                    Func::Ceil => call("tm_ceil", &[n])?,
                    Func::Floor => call("tm_floor", &[n])?,
                    _ => call("tm_round", &[n])?,
                };
                with(out, c)
            }
            Func::Year | Func::Month | Func::Day | Func::Hours | Func::Minutes | Func::Seconds => {
                let part = match f {
                    Func::Year => 0,
                    Func::Month => 1,
                    Func::Day => 2,
                    Func::Hours => 3,
                    Func::Minutes => 4,
                    _ => 5,
                };
                let x = self.val_sql(&vals[0]);
                let out = match vals[0].dom {
                    Dom::Term => call("tm_dt", &[x, V::Integer(part)])?,
                    _ => V::Null,
                };
                with(
                    out,
                    if part == 5 {
                        VClass::Dynamic
                    } else {
                        VClass::Int
                    },
                )
            }
            Func::Tz | Func::Timezone => {
                let x = self.val_sql(&vals[0]);
                let (name, c) = if f == Func::Tz {
                    ("tm_tz", VClass::Str)
                } else {
                    ("tm_timezone", VClass::Lit)
                };
                let out = match vals[0].dom {
                    Dom::Term => call(name, &[x])?,
                    _ => V::Null,
                };
                with(out, c)
            }
            Func::CastString => with(s(0)?, VClass::Str),
            Func::CastInteger => with(
                call("tm_cast", &[s(0)?, V::Integer(cast::INTEGER)])?,
                VClass::Int,
            ),
            Func::CastDouble | Func::CastDecimal => {
                let k = if f == Func::CastDouble {
                    cast::DOUBLE
                } else {
                    cast::DECIMAL
                };
                with(call("tm_cast", &[s(0)?, V::Integer(k)])?, VClass::Double)
            }
            Func::CastBoolean => {
                Val::boolean(call("tm_cast", &[s(0)?, V::Integer(cast::BOOLEAN)])?, true)
            }
            Func::CastDate | Func::CastDateTime => {
                let k = if f == Func::CastDate {
                    cast::DATE
                } else {
                    cast::DATETIME
                };
                Val::of(call("tm_cast", &[s(0)?, V::Integer(k)])?, Dom::Term, true)
            }
            _ => {
                return Err(tm_exec::error::invalid(format!(
                    "{} is not a SPARQL built-in",
                    f.name()
                )))
            }
        })
    }

    fn lookup(&self, l: &PLookup, rel: &Rel, row: &[V]) -> Result<Val> {
        let sval = self.value(&l.subject, rel, row)?;
        let s = self.val_sql(&sval);
        let view = crate::exec::store_view(&l.view);
        let rows = match &s {
            V::Integer(s) => {
                view.scan_eid(self.store, Pattern::new(Some(*s), Some(l.pred.raw()), None))?
            }
            _ => Vec::new(),
        };
        let (mut out, dom) = match l.multi {
            LookupMode::Single => (rows.first().map_or(V::Null, |r| V::Integer(r.o)), Dom::Term),
            LookupMode::ListIfMany => {
                // distinct objects in order of their first eid
                let mut seen: Vec<i64> = Vec::new();
                for r in &rows {
                    if !seen.contains(&r.o) {
                        seen.push(r.o);
                    }
                }
                let v = match seen.len() {
                    0 => V::Null,
                    1 => V::Integer(seen[0]),
                    _ => sv::json_array(&seen.iter().map(|o| V::Integer(*o)).collect::<Vec<_>>()),
                };
                (v, Dom::TermOrList)
            }
        };
        let volatile = l.volatile
            && l.view.tx == ResolvedTx::Now
            && l.view.valid == tm_core::ValidSel::Unfiltered;
        if volatile && out.is_null() {
            if let V::Integer(s) = &s {
                if let Some(v) = self.store.volatile(*s, l.pred.raw())? {
                    out = V::Integer(v.value);
                }
            }
        }
        Ok(Val::of(out, dom, true))
    }

    /// Evaluates a condition with three-valued logic.
    pub fn cond(&self, e: &PExpr, rel: &Rel, row: &[V]) -> Result<V> {
        Ok(match e {
            PExpr::Cmp(op, a, b) => self.compare(*op, a, b, rel, row)?,
            PExpr::SameTerm(a, b) => {
                let (a, b) = (self.value(a, rel, row)?, self.value(b, rel, row)?);
                let (a, b) = if a.konst.is_some() { (b, a) } else { (a, b) };
                let x = self.val_sql(&a);
                match &b.konst {
                    Some(k) => match (k.id, &a.dom) {
                        (Some(id), Dom::Term) => sv::eq(&x, &V::Integer(id.raw())),
                        (_, Dom::Computed(c)) if compatible(*c, &k.value) => {
                            sv::eq(&x, &self.val_sql(&b))
                        }
                        _ => null_else_false(&x),
                    },
                    None => {
                        let y = self.val_sql(&b);
                        if a.dom == b.dom {
                            sv::eq(&x, &y)
                        } else if x.is_null() || y.is_null() {
                            V::Null
                        } else {
                            V::Integer(0)
                        }
                    }
                }
            }
            PExpr::And(xs) => {
                let mut acc = V::Integer(1);
                for x in xs {
                    acc = and(&acc, &self.cond(x, rel, row)?);
                    if acc == V::Integer(0) {
                        break;
                    }
                }
                acc
            }
            PExpr::Or(xs) => {
                let mut acc = V::Integer(0);
                for x in xs {
                    acc = or(&acc, &self.cond(x, rel, row)?);
                    if acc == V::Integer(1) {
                        break;
                    }
                }
                acc
            }
            PExpr::Not(x) => not(&self.cond(x, rel, row)?),
            PExpr::Bound(v) => match rel.idx(v) {
                Some(i) if !rel.cols[i].null => truth(Some(!row[i].is_null())),
                _ => V::Integer(0),
            },
            PExpr::In(a, xs, neg) => {
                if xs.is_empty() {
                    return Ok(V::Integer(*neg as i64));
                }
                let mut any = V::Integer(0);
                for x in xs {
                    any = or(&any, &self.compare(CmpOp::Eq, a, x, rel, row)?);
                }
                if *neg {
                    not(&any)
                } else {
                    any
                }
            }
            PExpr::Exists(n, neg) => self.exists(n, *neg, rel, row)?,
            PExpr::Bool(b) => V::Integer(*b as i64),
            PExpr::Null => V::Null,
            other => {
                let v = self.value(other, rel, row)?;
                self.ebv(&v)?
            }
        })
    }

    /// The effective boolean value of a value.
    fn ebv(&self, v: &Val) -> Result<V> {
        if let Some(k) = &v.konst {
            return Ok(match &k.value {
                Value::Bool(b) => V::Integer(*b as i64),
                Value::Int(i) => V::Integer((*i != 0) as i64),
                Value::Str(s) => V::Integer((!s.is_empty()) as i64),
                _ => V::Null,
            });
        }
        let x = self.val_sql(v);
        Ok(match &v.dom {
            Dom::Term => match &x {
                V::Integer(i) => match tag(*i) {
                    6 => V::Integer(((i >> 4) == 1) as i64),
                    5 => V::Integer(((i >> 4) != 0) as i64),
                    9 => V::Integer((((i >> 4) & 15) != 0) as i64),
                    10 | 11 => V::Integer(1),
                    13 | 14 => match self.num_of(&v.dom, &x)? {
                        V::Null => V::Null,
                        n => truth(sv::compare(&n, &V::Integer(0)).map(|o| o != Ordering::Equal)),
                    },
                    _ => V::Null,
                },
                _ => V::Null,
            },
            Dom::Computed(VClass::Bool) => x,
            Dom::Computed(VClass::Int | VClass::Double) => {
                truth(sv::compare(&x, &V::Integer(0)).map(|o| o != Ordering::Equal))
            }
            Dom::Computed(VClass::Str | VClass::Iri) => {
                truth(sv::compare(&sv::length(&x), &V::Integer(0)).map(|o| o == Ordering::Greater))
            }
            _ => match &x {
                V::Text(_) => truth(
                    sv::compare(&sv::length(&x), &V::Integer(0)).map(|o| o == Ordering::Greater),
                ),
                _ => truth(sv::compare(&x, &V::Integer(0)).map(|o| o != Ordering::Equal)),
            },
        })
    }

    /// Value equality of two values of possibly different domains.
    pub fn value_eq(&self, a: &Val, x: &V, b: &Val, y: &V) -> Result<V> {
        Ok(match (&a.dom, &b.dom) {
            (Dom::Term, Dom::Term) => {
                let (kx, ky) = (
                    call("tm_kind", std::slice::from_ref(x))?,
                    call("tm_kind", std::slice::from_ref(y))?,
                );
                if kx == V::Integer(K_NUM) && ky == V::Integer(K_NUM) {
                    sv::eq(&self.num_of(&a.dom, x)?, &self.num_of(&b.dom, y)?)
                } else {
                    match (x, y) {
                        (V::Integer(i), V::Integer(j)) if tag(*i) == 7 && tag(*j) == 7 => {
                            V::Integer(((i >> 15) == (j >> 15)) as i64)
                        }
                        _ => sv::eq(x, y),
                    }
                }
            }
            (Dom::Computed(_), Dom::Term) => return self.value_eq(b, y, a, x),
            (Dom::Term, Dom::Computed(c)) => {
                if x.is_null() || y.is_null() {
                    return Ok(V::Null);
                }
                let kind = call("tm_kind", std::slice::from_ref(x))?;
                match c {
                    VClass::Int | VClass::Double => {
                        if kind == V::Integer(K_NUM) {
                            sv::eq(&self.num_of(&a.dom, x)?, y)
                        } else {
                            V::Integer(0)
                        }
                    }
                    VClass::Str => {
                        if kind == V::Integer(9) {
                            sv::eq(&self.str_of(&a.dom, x)?, y)
                        } else {
                            V::Integer(0)
                        }
                    }
                    VClass::Iri => match x {
                        V::Integer(i) if tag(*i) == 0 => sv::eq(&self.str_of(&a.dom, x)?, y),
                        _ => V::Integer(0),
                    },
                    VClass::Bool => match x {
                        V::Integer(i) if tag(*i) == 6 => sv::eq(&V::Integer(i >> 4), y),
                        _ => V::Integer(0),
                    },
                    VClass::Lit => match x {
                        V::Integer(i) if tag(*i) == 11 || tag(*i) == 12 => sv::eq(
                            &self.str_of(&a.dom, x)?,
                            &call("tm_lit_lex", std::slice::from_ref(y))?,
                        ),
                        _ => V::Integer(0),
                    },
                    VClass::Dynamic => {
                        let vk = call("tm_vkind", &[y.clone(), V::Integer(0)])?;
                        if sv::cond(&sv::eq(&kind, &vk)) == Some(true) {
                            sv::eq(
                                &self.key_of(&a.dom, x)?,
                                &call("tm_vkey", &[y.clone(), V::Integer(0)])?,
                            )
                        } else {
                            V::Integer(0)
                        }
                    }
                }
            }
            _ => sv::eq(x, y),
        })
    }

    fn compare(&self, op: CmpOp, a: &PExpr, b: &PExpr, rel: &Rel, row: &[V]) -> Result<V> {
        let a = self.value(a, rel, row)?;
        let b = self.value(b, rel, row)?;
        let (op, a, b) = if a.konst.is_some() && b.konst.is_none() {
            (flip(op), b, a)
        } else {
            (op, a, b)
        };
        if let Some(k) = b.konst.clone() {
            let x = self.val_sql(&a);
            let a = Val {
                konst: None,
                v: x.clone(),
                ..a
            };
            return match op {
                CmpOp::Eq => self.eq_const(&a, &x, &k),
                CmpOp::Ne => Ok(not(&self.eq_const(&a, &x, &k)?)),
                _ => self.ord_const(op, &a, &x, &k),
            };
        }
        let x = self.val_sql(&a);
        let y = self.val_sql(&b);
        Ok(match op {
            CmpOp::Eq => self.value_eq(&a, &x, &b, &y)?,
            CmpOp::Ne => not(&self.value_eq(&a, &x, &b, &y)?),
            _ => match (&a.dom, &b.dom) {
                (Dom::Computed(c), Dom::Computed(d))
                    if (c.numeric() && d.numeric()) || (c == d && *c == VClass::Str) =>
                {
                    ord(op, &x, &y)
                }
                _ => {
                    let (ka, kb) = (self.kind_of(&a.dom, &x)?, self.kind_of(&b.dom, &y)?);
                    if sv::cond(&sv::eq(&ka, &kb)) == Some(true) {
                        ord(op, &self.key_of(&a.dom, &x)?, &self.key_of(&b.dom, &y)?)
                    } else {
                        V::Null
                    }
                }
            },
        })
    }

    fn synthetic_id(&self, v: &Value) -> Option<i64> {
        self.synthetic
            .iter()
            .find(|(_, x)| *x == v)
            .map(|(k, _)| *k)
    }

    fn eq_const(&self, a: &Val, x: &V, k: &PConst) -> Result<V> {
        match &a.dom {
            Dom::Term => {
                if kind_of_value(&k.value) == K_NUM {
                    let Some(n) = native(&k.value) else {
                        return Ok(null_else_false(x));
                    };
                    if x.is_null() {
                        return Ok(V::Null);
                    }
                    return Ok(
                        if call("tm_kind", std::slice::from_ref(x))? == V::Integer(K_NUM) {
                            sv::eq(&self.num_of(&a.dom, x)?, &n)
                        } else {
                            V::Integer(0)
                        },
                    );
                }
                if let Value::DateTime { ms, .. } = k.value {
                    return Ok(match x {
                        V::Null => V::Null,
                        V::Integer(i) if tag(*i) == 7 => V::Integer(((i >> 15) == ms) as i64),
                        _ => V::Integer(0),
                    });
                }
                Ok(
                    match k
                        .id
                        .map(|i| i.raw())
                        .or_else(|| self.synthetic_id(&k.value))
                    {
                        Some(id) => sv::eq(x, &V::Integer(id)),
                        None => null_else_false(x),
                    },
                )
            }
            Dom::Computed(c) if compatible(*c, &k.value) => {
                Ok(sv::eq(x, &native(&k.value).expect("compatible")))
            }
            _ => Ok(null_else_false(x)),
        }
    }

    fn ord_const(&self, op: CmpOp, a: &Val, x: &V, k: &PConst) -> Result<V> {
        let kind = kind_of_value(&k.value);
        Ok(match &a.dom {
            Dom::Term => {
                if let Value::DateTime { ms, .. } = k.value {
                    return Ok(match x {
                        V::Integer(i) if tag(*i) == 7 => {
                            ord(op, &V::Integer(i >> 15), &V::Integer(ms))
                        }
                        _ => V::Null,
                    });
                }
                if call("tm_kind", std::slice::from_ref(x))? == V::Integer(kind) {
                    ord(op, &self.key_of(&a.dom, x)?, &V::Blob(value_key(&k.value)))
                } else {
                    V::Null
                }
            }
            Dom::Computed(c) => {
                let direct = match c {
                    VClass::Int | VClass::Double => class_of(&k.value).numeric(),
                    VClass::Str => matches!(k.value, Value::Str(_)),
                    _ => false,
                };
                if direct {
                    ord(op, x, &native(&k.value).expect("native"))
                } else {
                    let code = V::Integer(c.code());
                    if call("tm_vkind", &[x.clone(), code.clone()])? == V::Integer(kind) {
                        ord(
                            op,
                            &call("tm_vkey", &[x.clone(), code])?,
                            &V::Blob(value_key(&k.value)),
                        )
                    } else {
                        V::Null
                    }
                }
            }
            _ => V::Null,
        })
    }

    /// The truth of a join condition on one shared variable (`Gen::join_eq`).
    pub fn join_eq(
        &self,
        a: &crate::rel::Col,
        x: &V,
        b: &crate::rel::Col,
        y: &V,
        null_safe: bool,
    ) -> Result<bool> {
        let va = Val {
            v: x.clone(),
            dom: a.dom.clone(),
            mm: a.mm,
            konst: None,
            null: a.null,
        };
        let vb = Val {
            v: y.clone(),
            dom: b.dom.clone(),
            mm: b.mm,
            konst: None,
            null: b.null,
        };
        let base = || -> Result<V> {
            Ok(if a.dom == b.dom {
                sv::eq(x, y)
            } else {
                self.value_eq(&va, x, &vb, y)?
            })
        };
        if null_safe {
            return Ok(if a.dom == b.dom {
                sv::is(x, y)
            } else {
                (x.is_null() && y.is_null()) || sv::cond(&base()?) == Some(true)
            });
        }
        if (a.mm || b.mm) && self.sem.missing == tm_ir::Missing::Unbound {
            if (a.mm && x.is_null()) || (b.mm && y.is_null()) {
                return Ok(true);
            }
            return Ok(sv::cond(&base()?) == Some(true));
        }
        Ok(sv::cond(&base()?) == Some(true))
    }

    /// A statement's virtual-predicate object as the SQL route computes it.
    pub fn virtual_object(
        &self,
        pred: tm_exec::virtual_pred::VirtualPred,
        r: &tm_store::StmtRow,
    ) -> Result<V> {
        use tm_exec::virtual_pred::{VirtualPred as P, DATETIME_Z_LOW};
        Ok(match pred {
            P::Subject => V::Integer(r.s),
            P::Predicate => V::Integer(r.p),
            P::Object => V::Integer(r.o),
            P::TxAdded => V::Integer((r.t_add << 4) | 4),
            P::TxRetracted => r.t_ret.map_or(V::Null, |t| V::Integer((t << 4) | 4)),
            P::AddedAt => match self.store.tx_instant(r.t_add)? {
                Some(i) => V::Integer((i << 15) | DATETIME_Z_LOW),
                None => V::Null,
            },
            P::RetractedAt => match r.t_ret {
                Some(t) => match self.store.tx_instant(t)? {
                    Some(i) => V::Integer((i << 15) | DATETIME_Z_LOW),
                    None => V::Null,
                },
                None => V::Null,
            },
            P::ValidFrom => r
                .v_from
                .map_or(V::Null, |v| V::Integer((v << 15) | DATETIME_Z_LOW)),
            P::ValidTo => r
                .v_to
                .map_or(V::Null, |v| V::Integer((v << 15) | DATETIME_Z_LOW)),
            P::RetractKind => r.ret_kind.map_or(V::Null, |k| V::Integer((k << 4) | 5)),
        })
    }
}

/// True when an id is a statement eid.
pub fn is_stmt(id: i64) -> bool {
    id & 15 == Tag::Stmt as i64
}

/// The raw column a virtual predicate compares a constant with.
pub fn virtual_column(
    pred: tm_exec::virtual_pred::VirtualPred,
    r: &tm_store::StmtRow,
) -> Option<i64> {
    use tm_exec::virtual_pred::VirtualPred as P;
    match pred {
        P::Subject => Some(r.s),
        P::Predicate => Some(r.p),
        P::Object => Some(r.o),
        P::TxAdded | P::AddedAt => Some(r.t_add),
        P::TxRetracted | P::RetractedAt => r.t_ret,
        P::ValidFrom => r.v_from,
        P::ValidTo => r.v_to,
        P::RetractKind => r.ret_kind,
    }
}
