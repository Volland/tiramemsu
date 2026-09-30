//! Expressions → SQL (design D9): three-valued logic on SQL NULL, value equality
//! and class-checked ordering comparisons, datetimes by instant (`id >> 15`,
//! decision D20), constants bound as ids, native values or key BLOBs.

use tm_core::{Result, SqlValue, Value};
use tm_ir::{ArithOp, CmpOp, Func, LookupMode, Missing};

use super::{Col, Gen, Rel, Select};
use crate::error::invalid;
use crate::plan::analyze::{Dom, VClass};
use crate::plan::{Node, PConst, PExpr, PLookup};
use crate::scan::{view_predicates, ResolvedTx};
use crate::udf::{kind_of_value, value_key, K_NUM};

/// Dictionary tags (whose values need a `term` lookup).
const DICT_TAGS: &str = "(0, 10, 11, 12, 13, 14)";

/// A compiled scalar value. Constants stay unbound until used, so no parameter
/// is ever bound without appearing in the SQL text.
#[derive(Clone, Debug)]
pub struct Val {
    /// SQL text (`None` for a constant not yet bound).
    pub sql: Option<String>,
    /// Static domain.
    pub dom: Dom,
    /// May be NULL.
    pub mm: bool,
    /// The constant, if this is one.
    pub konst: Option<PConst>,
}

impl Val {
    fn sql(sql: impl Into<String>, dom: Dom, mm: bool) -> Val {
        Val {
            sql: Some(sql.into()),
            dom,
            mm,
            konst: None,
        }
    }

    fn boolean(sql: impl Into<String>, mm: bool) -> Val {
        Val::sql(sql, Dom::Computed(VClass::Bool), mm)
    }
}

fn class_of(v: &Value) -> VClass {
    match v {
        Value::Int(_) => VClass::Int,
        Value::Double(_) | Value::Decimal(_) => VClass::Double,
        Value::Bool(_) => VClass::Bool,
        Value::Iri(_) => VClass::Iri,
        _ => VClass::Str,
    }
}

/// The native SQLite value of a constant, if it has a computed class.
fn native(v: &Value) -> Option<SqlValue> {
    Some(match v {
        Value::Int(i) => SqlValue::Integer(*i),
        Value::Double(x) => SqlValue::Real(*x),
        Value::Decimal(s) => SqlValue::Real(s.parse().ok()?),
        Value::Bool(b) => SqlValue::Integer(*b as i64),
        Value::Str(s) | Value::Iri(s) => SqlValue::Text(s.clone()),
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

fn sym(op: CmpOp) -> &'static str {
    match op {
        CmpOp::Ne => "<>",
        other => other.symbol(),
    }
}

impl Gen<'_> {
    /// The SQL of a value, binding a constant (call once per use).
    pub fn val_sql(&mut self, v: &Val) -> String {
        if let Some(s) = &v.sql {
            return s.clone();
        }
        let k = v.konst.as_ref().expect("constant");
        match (&v.dom, k.id) {
            (Dom::Term, Some(id)) => self.params.id(id),
            _ => match native(&k.value) {
                Some(n) => self.params.push(n),
                None => "NULL".to_string(),
            },
        }
    }

    /// A column for an Extend.
    pub fn val_col(&mut self, v: Val) -> Col {
        Col {
            sql: self.val_sql(&v),
            dom: v.dom,
            mm: v.mm,
            eid_of: None,
        }
    }

    fn dict(&mut self, x: &str, col: &str) -> String {
        let src = self.term_source();
        let d = self.alias('d');
        format!("(SELECT {d}.{col} FROM {src} AS {d} WHERE {d}.id = ({x} >> 4))")
    }

    /// The numeric value of `x` (NULL for non-numbers).
    pub fn num_of(&mut self, v: &Val, x: &str) -> String {
        match v.dom {
            Dom::Term => {
                let n = self.dict(x, "num");
                format!(
                    "CASE WHEN ({x} & 15) = 5 THEN ({x} >> 4) WHEN ({x} & 15) IN (13, 14) THEN {n} END"
                )
            }
            Dom::Computed(VClass::Bool) => format!("CASE WHEN {x} IS NULL THEN NULL END"),
            _ => x.to_string(),
        }
    }

    /// The string (lexical) value of `x`.
    pub fn str_of(&mut self, v: &Val, x: &str) -> String {
        match v.dom {
            Dom::Term => {
                let lex = self.dict(x, "lex");
                format!("tm_str({x}, CASE WHEN ({x} & 15) IN {DICT_TAGS} THEN {lex} END)")
            }
            Dom::Computed(VClass::Str | VClass::Iri) => x.to_string(),
            Dom::Computed(VClass::Bool) => {
                format!("CASE WHEN {x} IS NULL THEN NULL WHEN {x} THEN 'true' ELSE 'false' END")
            }
            _ => format!("CAST({x} AS TEXT)"),
        }
    }

    /// The memcmp-ordered value key of `x`.
    pub fn key_of(&mut self, v: &Val, x: &str) -> String {
        match &v.dom {
            Dom::Term => {
                let src = self.term_source();
                let d = self.alias('d');
                format!(
                    "CASE WHEN ({x} & 15) IN {DICT_TAGS} THEN (SELECT tm_sortkey({x}, {d}.lex, {d}.num, {d}.lang) \
                     FROM {src} AS {d} WHERE {d}.id = ({x} >> 4)) ELSE tm_sortkey({x}, NULL, NULL, NULL) END"
                )
            }
            Dom::Computed(c) => format!("tm_vkey({x}, {})", c.code()),
            _ => x.to_string(),
        }
    }

    fn kind_of(&self, v: &Val, x: &str) -> String {
        match &v.dom {
            Dom::Term => format!("tm_kind({x})"),
            Dom::Computed(c) => format!("tm_vkind({x}, {})", c.code()),
            _ => "NULL".to_string(),
        }
    }

    /// Compiles a scalar expression against the columns of `rel`.
    pub fn value(&mut self, e: &PExpr, rel: &Rel) -> Result<Val> {
        Ok(match e {
            PExpr::Var(v) => match rel.col(v) {
                Some(c) => Val::sql(c.sql.clone(), c.dom.clone(), c.mm),
                None => Val::sql("NULL", Dom::Term, true),
            },
            PExpr::Const(k) => Val {
                sql: None,
                dom: if k.id.is_some() {
                    Dom::Term
                } else {
                    Dom::Computed(class_of(&k.value))
                },
                mm: false,
                konst: Some(k.clone()),
            },
            PExpr::Bool(b) => Val::boolean(if *b { "1" } else { "0" }, false),
            PExpr::Null => Val::sql("NULL", Dom::Term, true),
            PExpr::Bound(_) | PExpr::Exists(..) => Val::boolean(self.cond(e, rel, false)?, false),
            PExpr::Cmp(..)
            | PExpr::SameTerm(..)
            | PExpr::And(_)
            | PExpr::Or(_)
            | PExpr::Not(_)
            | PExpr::In(..) => Val::boolean(self.cond(e, rel, false)?, true),
            PExpr::Arith(op, a, b) => {
                let a = self.value(a, rel)?;
                let b = self.value(b, rel)?;
                let na = self.num_val(&a);
                let nb = self.num_val(&b);
                let cls = |v: &Val| match (&v.dom, &v.konst) {
                    (_, Some(k)) => class_of(&k.value),
                    (Dom::Computed(c), None) => *c,
                    _ => VClass::Dynamic,
                };
                match op {
                    ArithOp::Div => Val::sql(
                        format!("(CAST({na} AS REAL) / {nb})"),
                        Dom::Computed(VClass::Double),
                        true,
                    ),
                    _ => {
                        let c = match (cls(&a), cls(&b)) {
                            (VClass::Int, VClass::Int) => VClass::Int,
                            (VClass::Double, _) | (_, VClass::Double) => VClass::Double,
                            _ => VClass::Dynamic,
                        };
                        Val::sql(
                            format!("({na} {} {nb})", op.symbol()),
                            Dom::Computed(c),
                            true,
                        )
                    }
                }
            }
            PExpr::Neg(a) => {
                let a = self.value(a, rel)?;
                let n = self.num_val(&a);
                let c = match &a.dom {
                    Dom::Computed(c) if c.numeric() => *c,
                    _ => VClass::Dynamic,
                };
                Val::sql(format!("(-{n})"), Dom::Computed(c), true)
            }
            PExpr::Coalesce(xs) => {
                let vals: Vec<Val> = xs
                    .iter()
                    .map(|x| self.value(x, rel))
                    .collect::<Result<_>>()?;
                let dom = self.common_dom(&vals)?;
                let parts: Vec<String> = vals.iter().map(|v| self.val_sql(v)).collect();
                Val::sql(
                    format!("COALESCE({})", parts.join(", ")),
                    dom,
                    vals.iter().all(|v| v.mm),
                )
            }
            PExpr::If(c, a, b) => {
                let c = self.cond(c, rel, false)?;
                let a = self.value(a, rel)?;
                let b = self.value(b, rel)?;
                let dom = self.common_dom(&[a.clone(), b.clone()])?;
                let (x, y) = (self.val_sql(&a), self.val_sql(&b));
                Val::sql(
                    format!("CASE WHEN {c} THEN {x} WHEN NOT ({c}) THEN {y} END"),
                    dom,
                    true,
                )
            }
            PExpr::Func(f, args) => self.func(*f, args, rel)?,
            PExpr::Lookup(l) => self.lookup(l, rel)?,
            PExpr::List(xs) => {
                let vals: Vec<Val> = xs
                    .iter()
                    .map(|x| self.value(x, rel))
                    .collect::<Result<_>>()?;
                let elem = if vals.is_empty() {
                    Dom::Term
                } else {
                    self.common_dom(&vals)?
                };
                let parts: Vec<String> = vals.iter().map(|v| self.val_sql(v)).collect();
                Val::sql(
                    format!("json_array({})", parts.join(", ")),
                    Dom::List(Box::new(elem)),
                    false,
                )
            }
        })
    }

    fn common_dom(&self, vals: &[Val]) -> Result<Dom> {
        let doms: Vec<&Dom> = vals
            .iter()
            .filter(|v| v.sql.as_deref() != Some("NULL"))
            .map(|v| &v.dom)
            .collect();
        if doms.windows(2).any(|w| w[0] != w[1]) {
            return self.unsupported("an expression mixing stored terms and computed values");
        }
        Ok(doms.first().map_or(Dom::Term, |d| (*d).clone()))
    }

    /// The numeric SQL of a value (a constant binds its native number).
    fn num_val(&mut self, v: &Val) -> String {
        if let Some(k) = &v.konst {
            return match native(&k.value) {
                Some(n @ (SqlValue::Integer(_) | SqlValue::Real(_)))
                    if class_of(&k.value).numeric() =>
                {
                    self.params.push(n)
                }
                _ => "NULL".to_string(),
            };
        }
        let x = self.val_sql(v);
        self.num_of(v, &x)
    }

    fn str_val(&mut self, v: &Val) -> String {
        if let Some(k) = &v.konst {
            return self.params.push(SqlValue::Text(k.value.lexical()));
        }
        let x = self.val_sql(v);
        self.str_of(v, &x)
    }

    fn func(&mut self, f: Func, args: &[PExpr], rel: &Rel) -> Result<Val> {
        let vals: Vec<Val> = args
            .iter()
            .map(|a| self.value(a, rel))
            .collect::<Result<_>>()?;
        let arity = match f {
            Func::Contains | Func::StrStarts | Func::StrEnds => 2..=2,
            Func::Regex => 2..=3,
            _ => 1..=1,
        };
        if !arity.contains(&vals.len()) {
            return Err(invalid(format!(
                "{} takes {:?} arguments, got {}",
                f.name(),
                arity,
                vals.len()
            )));
        }
        let str_ = |s: String| Val::sql(s, Dom::Computed(VClass::Str), true);
        Ok(match f {
            Func::Str => str_(self.str_val(&vals[0])),
            Func::Lang => {
                let v = &vals[0];
                let x = self.val_sql(v);
                match v.dom {
                    Dom::Term => {
                        let l = self.dict(&x, "lang");
                        str_(format!(
                            "tm_lang({x}, CASE WHEN ({x} & 15) = 11 THEN {l} END)"
                        ))
                    }
                    _ => str_(format!("CASE WHEN {x} IS NULL THEN NULL ELSE '' END")),
                }
            }
            Func::Datatype => {
                let v = &vals[0];
                let x = self.val_sql(v);
                let sql = match v.dom {
                    Dom::Term => {
                        let d = self.alias('d');
                        let e = self.alias('d');
                        format!(
                            "tm_datatype({x}, CASE WHEN ({x} & 15) IN (12, 13, 14) THEN \
                             (SELECT (SELECT {e}.lex FROM term AS {e} WHERE {e}.id = ({d}.dt >> 4)) \
                             FROM term AS {d} WHERE {d}.id = ({x} >> 4)) END)"
                        )
                    }
                    _ => {
                        let i = self
                            .params
                            .push(SqlValue::Text(tm_core::vocab::XSD_INTEGER.into()));
                        let r = self
                            .params
                            .push(SqlValue::Text(tm_core::vocab::XSD_DOUBLE.into()));
                        let s = self
                            .params
                            .push(SqlValue::Text(tm_core::vocab::XSD_STRING.into()));
                        format!("CASE typeof({x}) WHEN 'integer' THEN {i} WHEN 'real' THEN {r} WHEN 'text' THEN {s} END")
                    }
                };
                Val::sql(sql, Dom::Computed(VClass::Iri), true)
            }
            Func::IsIri | Func::IsLiteral | Func::IsNumeric => {
                let v = &vals[0];
                let x = self.val_sql(v);
                let sql = match (&v.dom, f) {
                    (Dom::Term, Func::IsIri) => format!("(({x} & 15) IN (0, 1, 3, 4))"),
                    (Dom::Term, Func::IsLiteral) => format!("(({x} & 15) BETWEEN 5 AND 14)"),
                    (Dom::Term, _) => format!("(({x} & 15) IN (5, 13, 14))"),
                    (Dom::Computed(VClass::Iri), Func::IsIri) => {
                        format!("CASE WHEN {x} IS NULL THEN NULL ELSE 1 END")
                    }
                    (Dom::Computed(VClass::Iri), _) | (_, Func::IsIri) => {
                        format!("CASE WHEN {x} IS NULL THEN NULL ELSE 0 END")
                    }
                    (_, Func::IsLiteral) => format!("CASE WHEN {x} IS NULL THEN NULL ELSE 1 END"),
                    _ => format!("(typeof({x}) IN ('integer', 'real'))"),
                };
                Val::boolean(sql, true)
            }
            Func::StrLen => {
                let s = self.str_val(&vals[0]);
                Val::sql(format!("length({s})"), Dom::Computed(VClass::Int), true)
            }
            Func::UCase | Func::LCase => {
                let s = self.str_val(&vals[0]);
                let n = if f == Func::UCase {
                    "tm_ucase"
                } else {
                    "tm_lcase"
                };
                str_(format!("{n}({s})"))
            }
            Func::Contains => {
                let a = self.str_val(&vals[0]);
                let b = self.str_val(&vals[1]);
                Val::boolean(format!("(instr({a}, {b}) > 0)"), true)
            }
            Func::StrStarts => {
                let a = self.str_val(&vals[0]);
                let b = self.str_val(&vals[1]);
                Val::boolean(format!("(substr({a}, 1, length({b})) = {b})"), true)
            }
            Func::StrEnds => {
                let a = self.str_val(&vals[0]);
                let b = self.str_val(&vals[1]);
                Val::boolean(
                    format!(
                        "CASE WHEN {a} IS NULL OR {b} IS NULL THEN NULL WHEN length({b}) = 0 THEN 1 \
                         ELSE substr({a}, -length({b})) = {b} END"
                    ),
                    true,
                )
            }
            Func::Regex => {
                let a = self.str_val(&vals[0]);
                let p = self.str_val(&vals[1]);
                let fl = match vals.get(2) {
                    Some(v) => self.str_val(v),
                    None => "''".to_string(),
                };
                Val::boolean(format!("tm_regex({a}, {p}, {fl})"), true)
            }
        })
    }

    fn lookup(&mut self, l: &PLookup, rel: &Rel) -> Result<Val> {
        let sv = self.value(&l.subject, rel)?;
        let s = self.val_sql(&sv);
        let x = self.alias('x');
        let k = self.params.id(l.pred);
        let mut w = vec![format!("{x}.s = {s}"), format!("{x}.p = {k}")];
        w.extend(view_predicates(&x, &l.view, &mut self.params));
        let volatile = l.volatile
            && l.view.tx == ResolvedTx::Now
            && l.view.valid == tm_core::ValidSel::Unfiltered;
        let vol = if volatile {
            let v = self.alias('v');
            Some(format!(
                "(SELECT {v}.value FROM volatile AS {v} WHERE {v}.s = {s} AND {v}.key = {k})"
            ))
        } else {
            None
        };
        let (sql, dom) = match l.multi {
            LookupMode::Single => (
                format!(
                    "(SELECT {x}.o FROM triple AS {x} WHERE {} ORDER BY {x}.eid LIMIT 1)",
                    w.join(" AND ")
                ),
                Dom::Term,
            ),
            LookupMode::ListIfMany => {
                let q = self.alias('q');
                (
                    format!(
                        "(SELECT CASE count(*) WHEN 0 THEN NULL WHEN 1 THEN min({q}.o) \
                         ELSE json_group_array({q}.o) END FROM (SELECT {x}.o AS o, min({x}.eid) AS e \
                         FROM triple AS {x} WHERE {} GROUP BY {x}.o ORDER BY e) AS {q})",
                        w.join(" AND ")
                    ),
                    Dom::TermOrList,
                )
            }
        };
        let sql = match vol {
            Some(v) => format!("COALESCE({sql}, {v})"),
            None => sql,
        };
        Ok(Val::sql(sql, dom, true))
    }

    /// Compiles a condition to a SQL boolean with three-valued logic. `positive`
    /// is true when unknown and false are interchangeable (a top-level conjunct),
    /// which allows index-friendly range forms.
    pub fn cond(&mut self, e: &PExpr, rel: &Rel, positive: bool) -> Result<String> {
        Ok(match e {
            PExpr::Cmp(op, a, b) => self.compare(*op, a, b, rel, positive)?,
            PExpr::SameTerm(a, b) => {
                let (a, b) = (self.value(a, rel)?, self.value(b, rel)?);
                let (a, b) = if a.konst.is_some() { (b, a) } else { (a, b) };
                let x = self.val_sql(&a);
                match &b.konst {
                    Some(k) => match (k.id, &a.dom) {
                        (Some(id), Dom::Term) => {
                            let p = self.params.id(id);
                            format!("{x} = {p}")
                        }
                        (_, Dom::Computed(c)) if compatible(*c, &k.value) => {
                            let y = self.val_sql(&b);
                            format!("{x} = {y}")
                        }
                        _ => format!("CASE WHEN {x} IS NULL THEN NULL ELSE 0 END"),
                    },
                    None => {
                        let y = self.val_sql(&b);
                        if a.dom == b.dom {
                            format!("{x} = {y}")
                        } else {
                            format!("CASE WHEN {x} IS NULL OR {y} IS NULL THEN NULL ELSE 0 END")
                        }
                    }
                }
            }
            PExpr::And(xs) if xs.is_empty() => "1".to_string(),
            PExpr::Or(xs) if xs.is_empty() => "0".to_string(),
            PExpr::And(xs) | PExpr::Or(xs) => {
                let j = if matches!(e, PExpr::And(_)) {
                    " AND "
                } else {
                    " OR "
                };
                let parts: Vec<String> = xs
                    .iter()
                    .map(|x| self.cond(x, rel, positive))
                    .collect::<Result<_>>()?;
                format!("({})", parts.join(j))
            }
            PExpr::Not(x) => format!("NOT ({})", self.cond(x, rel, false)?),
            PExpr::Bound(v) => match rel.col(v) {
                Some(c) if c.sql != "NULL" => format!("{} IS NOT NULL", c.sql),
                _ => "0".to_string(),
            },
            PExpr::In(a, xs, neg) => {
                if xs.is_empty() {
                    return Ok(if *neg { "1" } else { "0" }.to_string());
                }
                let parts: Vec<String> = xs
                    .iter()
                    .map(|x| self.compare(CmpOp::Eq, a, x, rel, false))
                    .collect::<Result<_>>()?;
                let any = format!("({})", parts.join(" OR "));
                if *neg {
                    format!("NOT {any}")
                } else {
                    any
                }
            }
            PExpr::Exists(n, neg) => self.exists(n, *neg, rel)?,
            PExpr::Bool(b) => if *b { "1" } else { "0" }.to_string(),
            PExpr::Null => "NULL".to_string(),
            other => {
                let v = self.value(other, rel)?;
                self.ebv(&v)
            }
        })
    }

    /// The effective boolean value of a value.
    fn ebv(&mut self, v: &Val) -> String {
        if let Some(k) = &v.konst {
            return match &k.value {
                Value::Bool(b) => if *b { "1" } else { "0" }.to_string(),
                Value::Int(i) => if *i != 0 { "1" } else { "0" }.to_string(),
                Value::Str(s) => if s.is_empty() { "0" } else { "1" }.to_string(),
                _ => "NULL".to_string(),
            };
        }
        let x = self.val_sql(v);
        match &v.dom {
            Dom::Term => {
                let n = self.num_of(v, &x);
                format!(
                    "CASE WHEN {x} IS NULL THEN NULL WHEN ({x} & 15) = 6 THEN ({x} >> 4) = 1 \
                     WHEN ({x} & 15) = 5 THEN ({x} >> 4) <> 0 WHEN ({x} & 15) = 9 THEN (({x} >> 4) & 15) <> 0 \
                     WHEN ({x} & 15) IN (10, 11) THEN 1 WHEN ({x} & 15) IN (13, 14) THEN {n} <> 0 END"
                )
            }
            Dom::Computed(VClass::Bool) => x,
            Dom::Computed(VClass::Int | VClass::Double) => format!("({x} <> 0)"),
            Dom::Computed(VClass::Str | VClass::Iri) => format!("(length({x}) > 0)"),
            _ => format!("CASE typeof({x}) WHEN 'text' THEN length({x}) > 0 ELSE {x} <> 0 END"),
        }
    }

    /// `[NOT] EXISTS (SELECT 1 FROM … WHERE … AND <correlation>)`, correlated on
    /// the variables the nested tree shares with the outer row.
    fn exists(&mut self, n: &Node, neg: bool, outer: &Rel) -> Result<String> {
        let mut inner = self.compile(n)?;
        let shared: Vec<(Col, Col)> = inner
            .cols
            .iter()
            .filter_map(|(v, ic)| outer.col(v).map(|oc| (oc.clone(), ic.clone())))
            .collect();
        for (oc, ic) in shared {
            if oc.sql == "NULL" {
                continue;
            }
            let (cond, _) = self.join_eq(&oc, &ic, false)?;
            inner.conds.push(cond);
        }
        let body = self.select(
            &inner,
            &Select {
                list: vec![("1".to_string(), "c0".to_string())],
                ..Select::default()
            },
        );
        Ok(format!("{}EXISTS ({body})", if neg { "NOT " } else { "" }))
    }

    /// Value equality of two columns of different domains.
    pub fn value_eq_cols(&mut self, a: &Col, b: &Col) -> Result<String> {
        let va = Val::sql(a.sql.clone(), a.dom.clone(), a.mm);
        let vb = Val::sql(b.sql.clone(), b.dom.clone(), b.mm);
        Ok(self.value_eq(&va, &a.sql, &vb, &b.sql))
    }

    fn value_eq(&mut self, a: &Val, x: &str, b: &Val, y: &str) -> String {
        match (&a.dom, &b.dom) {
            (Dom::Term, Dom::Term) => {
                let nx = self.num_of(a, x);
                let ny = self.num_of(b, y);
                format!(
                    "CASE WHEN tm_kind({x}) = {K_NUM} AND tm_kind({y}) = {K_NUM} THEN {nx} = {ny} \
                     WHEN ({x} & 15) = 7 AND ({y} & 15) = 7 THEN ({x} >> 15) = ({y} >> 15) ELSE {x} = {y} END"
                )
            }
            (Dom::Computed(_), Dom::Term) => self.value_eq(b, y, a, x),
            (Dom::Term, Dom::Computed(c)) => {
                let head = format!("CASE WHEN {x} IS NULL OR {y} IS NULL THEN NULL");
                match c {
                    VClass::Int | VClass::Double => {
                        let nx = self.num_of(a, x);
                        format!("{head} WHEN tm_kind({x}) = {K_NUM} THEN {nx} = {y} ELSE 0 END")
                    }
                    VClass::Str => {
                        let sx = self.str_of(a, x);
                        format!("{head} WHEN tm_kind({x}) = 9 THEN {sx} = {y} ELSE 0 END")
                    }
                    VClass::Iri => {
                        let sx = self.str_of(a, x);
                        format!("{head} WHEN ({x} & 15) = 0 THEN {sx} = {y} ELSE 0 END")
                    }
                    VClass::Bool => {
                        format!("{head} WHEN ({x} & 15) = 6 THEN ({x} >> 4) = {y} ELSE 0 END")
                    }
                    VClass::Dynamic => {
                        let kx = self.key_of(a, x);
                        format!(
                            "{head} WHEN tm_kind({x}) = tm_vkind({y}, 0) THEN {kx} = tm_vkey({y}, 0) ELSE 0 END"
                        )
                    }
                }
            }
            _ => format!("{x} = {y}"),
        }
    }

    fn compare(
        &mut self,
        op: CmpOp,
        a: &PExpr,
        b: &PExpr,
        rel: &Rel,
        positive: bool,
    ) -> Result<String> {
        let a = self.value(a, rel)?;
        let b = self.value(b, rel)?;
        let (op, a, b) = if a.konst.is_some() && b.konst.is_none() {
            (flip(op), b, a)
        } else {
            (op, a, b)
        };
        if let Some(k) = b.konst.clone() {
            let x = self.val_sql(&a);
            let a = Val {
                sql: Some(x.clone()),
                konst: None,
                ..a
            };
            return Ok(match op {
                CmpOp::Eq => self.eq_const(&a, &x, &k, positive),
                CmpOp::Ne => format!("NOT ({})", self.eq_const(&a, &x, &k, false)),
                _ => self.ord_const(op, &a, &x, &k, positive),
            });
        }
        let x = self.val_sql(&a);
        let y = self.val_sql(&b);
        Ok(match op {
            CmpOp::Eq => self.value_eq(&a, &x, &b, &y),
            CmpOp::Ne => format!("NOT ({})", self.value_eq(&a, &x, &b, &y)),
            _ => {
                let s = sym(op);
                match (&a.dom, &b.dom) {
                    (Dom::Computed(c), Dom::Computed(d))
                        if (c.numeric() && d.numeric()) || (c == d && *c == VClass::Str) =>
                    {
                        format!("({x} {s} {y})")
                    }
                    _ => {
                        let (ka, kb) = (self.kind_of(&a, &x), self.kind_of(&b, &y));
                        let (xa, xb) = (self.key_of(&a, &x), self.key_of(&b, &y));
                        format!("CASE WHEN {ka} = {kb} THEN {xa} {s} {xb} END")
                    }
                }
            }
        })
    }

    fn eq_const(&mut self, a: &Val, x: &str, k: &PConst, positive: bool) -> String {
        let null_else_false = format!("CASE WHEN {x} IS NULL THEN NULL ELSE 0 END");
        match &a.dom {
            Dom::Term => {
                if kind_of_value(&k.value) == K_NUM {
                    let n = match native(&k.value) {
                        Some(n) => self.params.push(n),
                        None => return null_else_false,
                    };
                    let nx = self.num_of(a, x);
                    return format!(
                        "CASE WHEN {x} IS NULL THEN NULL WHEN tm_kind({x}) = {K_NUM} THEN {nx} = {n} ELSE 0 END"
                    );
                }
                if let Value::DateTime { ms, .. } = k.value {
                    if positive {
                        let lo = self.params.push(SqlValue::Integer((ms << 15) | 7));
                        let hi = self.params.push(SqlValue::Integer((ms << 15) | 0x7FF7));
                        return format!("({x} BETWEEN {lo} AND {hi} AND ({x} & 15) = 7)");
                    }
                    let m = self.params.push(SqlValue::Integer(ms));
                    return format!(
                        "CASE WHEN {x} IS NULL THEN NULL WHEN ({x} & 15) = 7 THEN ({x} >> 15) = {m} ELSE 0 END"
                    );
                }
                match k.id.or_else(|| self.synthetic_id(&k.value)) {
                    Some(id) => {
                        let p = self.params.id(id);
                        format!("{x} = {p}")
                    }
                    None => null_else_false,
                }
            }
            Dom::Computed(c) if compatible(*c, &k.value) => {
                let p = self.params.push(native(&k.value).expect("compatible"));
                format!("{x} = {p}")
            }
            _ => null_else_false,
        }
    }

    fn ord_const(&mut self, op: CmpOp, a: &Val, x: &str, k: &PConst, positive: bool) -> String {
        let s = sym(op);
        let kind = kind_of_value(&k.value);
        match &a.dom {
            Dom::Term => {
                if let Value::DateTime { ms, .. } = k.value {
                    if positive {
                        let bound = match op {
                            CmpOp::Lt | CmpOp::Ge => (ms << 15) | 7,
                            _ => (ms << 15) | 0x7FF7,
                        };
                        let p = self.params.push(SqlValue::Integer(bound));
                        return format!("(({x} & 15) = 7 AND {x} {s} {p})");
                    }
                    let m = self.params.push(SqlValue::Integer(ms));
                    return format!("CASE WHEN ({x} & 15) = 7 THEN ({x} >> 15) {s} {m} END");
                }
                let key = self.key_of(a, x);
                let blob = self.params.push(SqlValue::Blob(value_key(&k.value)));
                format!("CASE WHEN tm_kind({x}) = {kind} THEN {key} {s} {blob} END")
            }
            Dom::Computed(c) => {
                let direct = match c {
                    VClass::Int | VClass::Double => class_of(&k.value).numeric(),
                    VClass::Str => matches!(k.value, Value::Str(_)),
                    _ => false,
                };
                if direct {
                    let p = self.params.push(native(&k.value).expect("native"));
                    format!("({x} {s} {p})")
                } else {
                    let code = c.code();
                    let blob = self.params.push(SqlValue::Blob(value_key(&k.value)));
                    format!("CASE WHEN tm_vkind({x}, {code}) = {kind} THEN tm_vkey({x}, {code}) {s} {blob} END")
                }
            }
            _ => "NULL".to_string(),
        }
    }

    /// `ASC`/`DESC` with the NULL placement of `missing` (design D11).
    pub fn nulls(&self, desc: bool) -> &'static str {
        let first = match self.sem.missing {
            Missing::Unbound => !desc,
            Missing::Null3VL => desc,
        };
        match (desc, first) {
            (false, true) => "ASC NULLS FIRST",
            (false, false) => "ASC NULLS LAST",
            (true, true) => "DESC NULLS FIRST",
            (true, false) => "DESC NULLS LAST",
        }
    }
}
