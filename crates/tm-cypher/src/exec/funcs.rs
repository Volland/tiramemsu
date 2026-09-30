//! Scalar built-in functions and procedures (`cypher-read`, "Expressions and
//! built-in functions").

use tm_core::vocab::{XSD_DATE, XSD_DATETIME};
use tm_core::Value;
use tm_ir::vocab as irv;
use tm_ir::{Expr, Func, Op, TermOrVar, TriplePattern, View};

use super::access::{is_sys, retract_kind_name, stmt_of, tv, RDF_TYPE};
use super::Exec;
use crate::ast::Expr as AstExpr;
use crate::error::{CResult, CypherError};
use crate::value::{fmt_float, Val};

fn err(msg: String) -> CypherError {
    CypherError::eval(msg)
}

fn bad(name: &str, v: &Val) -> CypherError {
    err(format!("{name}() is not defined for {}", v.type_name()))
}

fn arity(name: &str, args: &[Val], lo: usize, hi: usize) -> CResult<()> {
    if args.len() < lo || args.len() > hi {
        return Err(err(format!(
            "{name}() takes {lo}..{hi} arguments, got {}",
            args.len()
        )));
    }
    Ok(())
}

fn to_f(v: &Val) -> Option<f64> {
    match v {
        Val::Int(i) => Some(*i as f64),
        Val::Float(x) => Some(*x),
        _ => None,
    }
}

fn parse_int_text(s: &str) -> Option<i64> {
    let t = s.trim();
    if let Ok(i) = t.parse::<i64>() {
        return Some(i);
    }
    if let Ok(x) = t.parse::<f64>() {
        if x.is_finite() && x.abs() < 9.2e18 {
            return Some(x.trunc() as i64);
        }
    }
    None
}

/// Resolves the fixed offset of a named zone at a local reading.
fn zone_offset_minutes(zone: &str, local_ms: i64) -> Option<i16> {
    use chrono::{Offset, TimeZone};
    let tz: chrono_tz::Tz = zone.parse().ok()?;
    let naive = chrono::DateTime::from_timestamp_millis(local_ms)?.naive_utc();
    let off = tz
        .offset_from_local_datetime(&naive)
        .earliest()?
        .fix()
        .local_minus_utc();
    Some((off / 60) as i16)
}

/// Parses a `datetime('…')` argument. Named zones (`[Europe/Kyiv]`) are resolved to
/// their offset at that instant and the name is dropped (D20).
pub fn parse_datetime_text(s: &str, local: bool) -> Option<Val> {
    let (body, zone) = match s.find('[') {
        Some(i) if s.ends_with(']') => (&s[..i], Some(&s[i + 1..s.len() - 1])),
        _ => (s, None),
    };
    let lit = Value::literal(body, Some(XSD_DATETIME), None);
    let Value::DateTime { ms, tz } = lit else {
        return None;
    };
    if local {
        // the local reading, whatever offset was given
        let local_ms = ms + tz.unwrap_or(0) as i64 * 60_000;
        return Some(Val::LocalDateTime(local_ms));
    }
    match (tz, zone) {
        (Some(tz), _) => Some(Val::DateTime { ms, tz }),
        (None, Some(z)) => {
            let off = zone_offset_minutes(z, ms)?;
            Some(Val::DateTime {
                ms: ms - off as i64 * 60_000,
                tz: off,
            })
        }
        (None, None) => Some(Val::DateTime { ms, tz: 0 }),
    }
}

impl Exec<'_> {
    pub(crate) fn call(&mut self, name: &str, args: Vec<Val>, e: &AstExpr) -> CResult<Val> {
        let lname = name.to_ascii_lowercase();
        let n = lname.as_str();
        // functions that treat null specially
        match n {
            "coalesce" => {
                return Ok(args.into_iter().find(|v| !v.is_null()).unwrap_or(Val::Null));
            }
            "exists" => {
                arity(n, &args, 1, 1)?;
                return Ok(Val::Bool(!args[0].is_null()));
            }
            "isempty" => {
                arity(n, &args, 1, 1)?;
                return Ok(match &args[0] {
                    Val::Null => Val::Null,
                    Val::Str(s) => Val::Bool(s.is_empty()),
                    Val::List(l) => Val::Bool(l.is_empty()),
                    Val::Map(m) => Val::Bool(m.is_empty()),
                    o => return Err(bad(n, o)),
                });
            }
            "e" => return Ok(Val::Float(std::f64::consts::E)),
            "pi" => return Ok(Val::Float(std::f64::consts::PI)),
            "timestamp" => return Ok(Val::Int(self.now_ms)),
            "rand" => {
                let x = (self.now_ms as u64)
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(self.uid as u64);
                return Ok(Val::Float(((x >> 11) as f64) / ((1u64 << 53) as f64)));
            }
            "range" => return range(&args),
            _ => {}
        }
        if args.iter().any(Val::is_null) && !matches!(n, "date" | "datetime" | "localdatetime") {
            // every other function is null-propagating
            if matches!(
                n,
                "id" | "elementid"
                    | "labels"
                    | "type"
                    | "keys"
                    | "properties"
                    | "startnode"
                    | "endnode"
                    | "size"
                    | "head"
                    | "last"
                    | "tail"
                    | "tostring"
                    | "tointeger"
                    | "tofloat"
                    | "toboolean"
                    | "tolower"
                    | "toupper"
                    | "trim"
                    | "ltrim"
                    | "rtrim"
                    | "substring"
                    | "replace"
                    | "split"
                    | "left"
                    | "right"
                    | "reverse"
                    | "abs"
                    | "ceil"
                    | "floor"
                    | "round"
                    | "sign"
                    | "sqrt"
                    | "nodes"
                    | "relationships"
                    | "length"
                    | "exp"
                    | "log"
                    | "log10"
                    | "sin"
                    | "cos"
                    | "tan"
                    | "cot"
                    | "asin"
                    | "acos"
                    | "atan"
                    | "atan2"
                    | "degrees"
                    | "radians"
                    | "haversin"
                    | "char_length"
                    | "character_length"
                    | "isnan"
            ) {
                return Ok(Val::Null);
            }
            if matches!(
                n,
                "tostringornull" | "tointegerornull" | "tofloatornull" | "tobooleanornull"
            ) {
                return Ok(Val::Null);
            }
        }
        if matches!(n, "labels" | "keys" | "properties" | "type") {
            if let Some(a) = args.first() {
                self.check_not_deleted(a)?;
            }
        }
        let a0 = args.first();
        Ok(match n {
            "id" => {
                arity(n, &args, 1, 1)?;
                match a0.unwrap() {
                    Val::Node(t) => match self.runner.object_id(t).map_err(super::core)? {
                        Some(i) => Val::Int(i),
                        None => Val::Null,
                    },
                    Val::Rel(e) => Val::Int(e.oid().raw()),
                    o => return Err(bad(n, o)),
                }
            }
            "elementid" => {
                arity(n, &args, 1, 1)?;
                match a0.unwrap() {
                    Val::Node(t) => Val::Str(t.lexical()),
                    Val::Rel(e) => Val::Str(Value::Stmt(*e).lexical()),
                    o => return Err(bad(n, o)),
                }
            }
            "labels" => {
                arity(n, &args, 1, 1)?;
                match a0.unwrap() {
                    Val::Node(t) => {
                        if matches!(
                            t,
                            Value::Iri(_) | Value::Node(_) | Value::BNode(_) | Value::Stmt(_)
                        ) {
                            let view = self.view();
                            Val::List(self.labels_of(t, view)?.into_iter().map(Val::Str).collect())
                        } else {
                            Val::Null
                        }
                    }
                    Val::Rel(e) => {
                        let view = self.view();
                        Val::List(
                            self.labels_of(&Value::Stmt(*e), view)?
                                .into_iter()
                                .map(Val::Str)
                                .collect(),
                        )
                    }
                    o => return Err(bad(n, o)),
                }
            }
            "type" => {
                arity(n, &args, 1, 1)?;
                match stmt_of(a0.unwrap()) {
                    Some(e) => match self.stmt_parts(e)? {
                        Some((_, p, _)) => Val::Str(self.vocab.render(&p)),
                        None => Val::Null,
                    },
                    None => return Err(bad(n, a0.unwrap())),
                }
            }
            "startnode" | "endnode" => {
                arity(n, &args, 1, 1)?;
                match stmt_of(a0.unwrap()) {
                    Some(e) => match self.stmt_parts(e)? {
                        Some((s, _, o)) => {
                            if n == "startnode" {
                                Val::from_entity(&s)
                            } else {
                                Val::from_entity(&o)
                            }
                        }
                        None => Val::Null,
                    },
                    None => return Err(bad(n, a0.unwrap())),
                }
            }
            "keys" => {
                arity(n, &args, 1, 1)?;
                match a0.unwrap() {
                    Val::Map(m) => Val::List(m.keys().cloned().map(Val::Str).collect()),
                    v @ (Val::Node(_) | Val::Rel(_)) => {
                        let m = self.all_props(v)?;
                        Val::List(m.keys().cloned().map(Val::Str).collect())
                    }
                    o => return Err(bad(n, o)),
                }
            }
            "properties" => {
                arity(n, &args, 1, 1)?;
                match a0.unwrap() {
                    Val::Map(m) => Val::Map(m.clone()),
                    v @ (Val::Node(_) | Val::Rel(_)) => Val::Map(self.all_props(v)?),
                    o => return Err(bad(n, o)),
                }
            }
            "size" | "length" | "char_length" | "character_length" => {
                arity(n, &args, 1, 1)?;
                match a0.unwrap() {
                    Val::List(l) => Val::Int(l.len() as i64),
                    Val::Str(s) => Val::Int(s.chars().count() as i64),
                    Val::Path(p) => Val::Int((p.len() / 2) as i64),
                    o => return Err(bad(n, o)),
                }
            }
            "nodes" => match a0 {
                Some(Val::Path(p)) => Val::List(
                    p.iter()
                        .filter(|x| !matches!(x, Val::Rel(_)))
                        .cloned()
                        .collect(),
                ),
                Some(o) => return Err(bad(n, o)),
                None => return Err(err("nodes() needs an argument".into())),
            },
            "relationships" => match a0 {
                Some(Val::Path(p)) => Val::List(
                    p.iter()
                        .filter(|x| matches!(x, Val::Rel(_)))
                        .cloned()
                        .collect(),
                ),
                Some(o) => return Err(bad(n, o)),
                None => return Err(err("relationships() needs an argument".into())),
            },
            "head" => match a0 {
                Some(Val::List(l)) => l.first().cloned().unwrap_or(Val::Null),
                Some(o) => return Err(bad(n, o)),
                None => Val::Null,
            },
            "last" => match a0 {
                Some(Val::List(l)) => l.last().cloned().unwrap_or(Val::Null),
                Some(o) => return Err(bad(n, o)),
                None => Val::Null,
            },
            "tail" => match a0 {
                Some(Val::List(l)) => Val::List(l.iter().skip(1).cloned().collect()),
                Some(o) => return Err(bad(n, o)),
                None => Val::Null,
            },
            "reverse" => match a0 {
                Some(Val::List(l)) => Val::List(l.iter().rev().cloned().collect()),
                Some(Val::Str(s)) => Val::Str(s.chars().rev().collect()),
                Some(o) => return Err(bad(n, o)),
                None => Val::Null,
            },
            "tostring" | "tostringornull" => {
                arity(n, &args, 1, 1)?;
                match a0.unwrap() {
                    Val::Null => Val::Null,
                    Val::Str(s) => Val::Str(s.clone()),
                    v @ (Val::Bool(_)
                    | Val::Int(_)
                    | Val::Float(_)
                    | Val::Date(_)
                    | Val::DateTime { .. }
                    | Val::LocalDateTime(_)) => Val::Str(v.to_display()),
                    o if n == "tostringornull" => {
                        let _ = o;
                        Val::Null
                    }
                    o => return Err(bad(n, o)),
                }
            }
            "tointeger" | "tointegerornull" => match a0.unwrap() {
                Val::Int(i) => Val::Int(*i),
                Val::Float(x) => {
                    if x.is_finite() && x.abs() < 9.2e18 {
                        Val::Int(x.trunc() as i64)
                    } else {
                        Val::Null
                    }
                }
                Val::Str(s) => parse_int_text(s).map_or(Val::Null, Val::Int),
                Val::Bool(b) => Val::Int(*b as i64),
                o if n == "tointegerornull" => {
                    let _ = o;
                    Val::Null
                }
                o => return Err(bad(n, o)),
            },
            "tofloat" | "tofloatornull" => match a0.unwrap() {
                Val::Int(i) => Val::Float(*i as f64),
                Val::Float(x) => Val::Float(*x),
                Val::Str(s) => s.trim().parse::<f64>().map_or(Val::Null, Val::Float),
                o if n == "tofloatornull" => {
                    let _ = o;
                    Val::Null
                }
                o => return Err(bad(n, o)),
            },
            "toboolean" | "tobooleanornull" => match a0.unwrap() {
                Val::Bool(b) => Val::Bool(*b),
                Val::Str(s) => match s.to_ascii_lowercase().as_str() {
                    "true" => Val::Bool(true),
                    "false" => Val::Bool(false),
                    _ => Val::Null,
                },
                Val::Int(i) if n == "toboolean" || n == "tobooleanornull" => Val::Bool(*i != 0),
                o if n == "tobooleanornull" => {
                    let _ = o;
                    Val::Null
                }
                o => return Err(bad(n, o)),
            },
            "tolower" | "toupper" | "trim" | "ltrim" | "rtrim" => match a0.unwrap() {
                Val::Str(s) => Val::Str(match n {
                    "tolower" => s.to_lowercase(),
                    "toupper" => s.to_uppercase(),
                    "trim" => s.trim().to_string(),
                    "ltrim" => s.trim_start().to_string(),
                    _ => s.trim_end().to_string(),
                }),
                o => return Err(bad(n, o)),
            },
            "substring" => {
                arity(n, &args, 2, 3)?;
                let Val::Str(s) = &args[0] else {
                    return Err(bad(n, &args[0]));
                };
                let Val::Int(start) = &args[1] else {
                    return Err(bad(n, &args[1]));
                };
                if *start < 0 {
                    return Err(err("substring() start must not be negative".into()));
                }
                let chars: Vec<char> = s.chars().collect();
                let st = (*start as usize).min(chars.len());
                let end = match args.get(2) {
                    Some(Val::Int(l)) => {
                        if *l < 0 {
                            return Err(err("substring() length must not be negative".into()));
                        }
                        (st + *l as usize).min(chars.len())
                    }
                    Some(Val::Null) => return Ok(Val::Null),
                    Some(o) => return Err(bad(n, o)),
                    None => chars.len(),
                };
                Val::Str(chars[st..end].iter().collect())
            }
            "left" | "right" => {
                arity(n, &args, 2, 2)?;
                let Val::Str(s) = &args[0] else {
                    return Err(bad(n, &args[0]));
                };
                let Val::Int(k) = &args[1] else {
                    return Err(bad(n, &args[1]));
                };
                if *k < 0 {
                    return Err(err(format!("{n}() length must not be negative")));
                }
                let chars: Vec<char> = s.chars().collect();
                let k = (*k as usize).min(chars.len());
                Val::Str(if n == "left" {
                    chars[..k].iter().collect()
                } else {
                    chars[chars.len() - k..].iter().collect()
                })
            }
            "replace" => {
                arity(n, &args, 3, 3)?;
                match (&args[0], &args[1], &args[2]) {
                    (Val::Str(s), Val::Str(a), Val::Str(b)) => Val::Str(s.replace(a.as_str(), b)),
                    _ => return Err(bad(n, &args[0])),
                }
            }
            "split" => {
                arity(n, &args, 2, 2)?;
                match (&args[0], &args[1]) {
                    (Val::Str(s), Val::Str(d)) => Val::List(if d.is_empty() {
                        s.chars().map(|c| Val::Str(c.to_string())).collect()
                    } else {
                        s.split(d.as_str())
                            .map(|x| Val::Str(x.to_string()))
                            .collect()
                    }),
                    _ => return Err(bad(n, &args[0])),
                }
            }
            "abs" => match a0.unwrap() {
                Val::Int(i) => Val::Int(
                    i.checked_abs()
                        .ok_or_else(|| err("integer overflow".into()))?,
                ),
                Val::Float(x) => Val::Float(x.abs()),
                o => return Err(bad(n, o)),
            },
            "sign" => match a0.unwrap() {
                Val::Int(i) => Val::Int(i.signum()),
                Val::Float(x) => Val::Int(if *x > 0.0 {
                    1
                } else if *x < 0.0 {
                    -1
                } else {
                    0
                }),
                o => return Err(bad(n, o)),
            },
            "ceil" | "floor" | "round" | "sqrt" | "exp" | "log" | "log10" | "sin" | "cos"
            | "tan" | "cot" | "asin" | "acos" | "atan" | "degrees" | "radians" | "haversin" => {
                let x = to_f(a0.unwrap()).ok_or_else(|| bad(n, a0.unwrap()))?;
                Val::Float(match n {
                    "ceil" => x.ceil(),
                    "floor" => x.floor(),
                    "round" => {
                        let p = match args.get(1) {
                            Some(Val::Int(p)) => *p,
                            _ => 0,
                        };
                        let m = 10f64.powi(p as i32);
                        (x * m + if x < 0.0 { -0.5 } else { 0.5 }).trunc() / m
                    }
                    "sqrt" => x.sqrt(),
                    "exp" => x.exp(),
                    "log" => x.ln(),
                    "log10" => x.log10(),
                    "sin" => x.sin(),
                    "cos" => x.cos(),
                    "tan" => x.tan(),
                    "cot" => 1.0 / x.tan(),
                    "asin" => x.asin(),
                    "acos" => x.acos(),
                    "atan" => x.atan(),
                    "degrees" => x.to_degrees(),
                    "radians" => x.to_radians(),
                    _ => (1.0 - x.cos()) / 2.0,
                })
            }
            "atan2" => {
                arity(n, &args, 2, 2)?;
                let y = to_f(&args[0]).ok_or_else(|| bad(n, &args[0]))?;
                let x = to_f(&args[1]).ok_or_else(|| bad(n, &args[1]))?;
                Val::Float(y.atan2(x))
            }
            "isnan" => match to_f(a0.unwrap()) {
                Some(x) => Val::Bool(x.is_nan()),
                None => return Err(bad(n, a0.unwrap())),
            },
            "nullif" => {
                arity(n, &args, 2, 2)?;
                if crate::value::eq3(&args[0], &args[1]) == Some(true) {
                    Val::Null
                } else {
                    args[0].clone()
                }
            }
            "date" => match args.first() {
                None => Val::Date(self.now_ms.div_euclid(86_400_000)),
                Some(Val::Null) => Val::Null,
                Some(Val::Str(s)) => match Value::literal(s, Some(XSD_DATE), None) {
                    Value::Date(d) => Val::Date(d),
                    _ => return Err(err(format!("invalid date '{s}'"))),
                },
                Some(Val::Date(d)) => Val::Date(*d),
                Some(Val::DateTime { ms, tz }) => {
                    Val::Date((ms + *tz as i64 * 60_000).div_euclid(86_400_000))
                }
                Some(Val::LocalDateTime(ms)) => Val::Date(ms.div_euclid(86_400_000)),
                Some(o) => return Err(bad(n, o)),
            },
            "datetime" | "localdatetime" => {
                let local = n == "localdatetime";
                match args.first() {
                    None => {
                        if local {
                            Val::LocalDateTime(self.now_ms)
                        } else {
                            Val::DateTime {
                                ms: self.now_ms,
                                tz: 0,
                            }
                        }
                    }
                    Some(Val::Null) => Val::Null,
                    Some(Val::Str(s)) => parse_datetime_text(s, local)
                        .ok_or_else(|| err(format!("invalid {n} '{s}'")))?,
                    Some(Val::DateTime { ms, tz }) => {
                        if local {
                            Val::LocalDateTime(ms + *tz as i64 * 60_000)
                        } else {
                            Val::DateTime { ms: *ms, tz: *tz }
                        }
                    }
                    Some(Val::LocalDateTime(ms)) => {
                        if local {
                            Val::LocalDateTime(*ms)
                        } else {
                            Val::DateTime { ms: *ms, tz: 0 }
                        }
                    }
                    Some(Val::Date(d)) => {
                        let ms = d * 86_400_000;
                        if local {
                            Val::LocalDateTime(ms)
                        } else {
                            Val::DateTime { ms, tz: 0 }
                        }
                    }
                    Some(Val::Map(_)) => {
                        return Err(CypherError::unsupported(
                            format!("{n}() with a map argument"),
                            Some(e.span),
                        ))
                    }
                    Some(o) => return Err(bad(n, o)),
                }
            }
            other => {
                return Err(CypherError::unsupported(
                    format!("function `{other}`"),
                    Some(e.span),
                ));
            }
        })
    }

    /// The built-in name procedures: (yield column, values).
    pub(crate) fn builtin_procedure(&mut self, lname: &str) -> CResult<(&'static str, Vec<Val>)> {
        let view: View = self.view();
        let f = self.flags(view)?;
        let (s, p, o) = ("s", "p", "o");
        let op = Op::Triple(TriplePattern::new(
            TermOrVar::var(s),
            TermOrVar::var(p),
            TermOrVar::var(o),
            view,
        ))
        .extend("lit", Expr::Func(Func::IsLiteral, vec![Expr::var(o)]))
        .project_distinct(&[p, "lit", o]);
        let _ = irv::SYS_SUBJECT;
        let _ = tv;
        match lname {
            "db.labels" => {
                let op = Op::Triple(TriplePattern::new(
                    TermOrVar::var(s),
                    TermOrVar::iri(RDF_TYPE),
                    TermOrVar::var(o),
                    view,
                ))
                .project_distinct(&[o]);
                let rows = self.run_op(op)?;
                let mut names: Vec<String> = Vec::new();
                for r in &rows.rows {
                    if let Some(Value::Iri(i)) = &r[0] {
                        if !is_sys(i) {
                            names.push(self.vocab.render(i));
                        }
                    }
                }
                names.sort();
                names.dedup();
                Ok(("label", names.into_iter().map(Val::Str).collect()))
            }
            "db.relationshiptypes" | "db.propertykeys" => {
                let want_rel = lname == "db.relationshiptypes";
                let rows = self.run_op(op)?;
                let (pi, li, oi) = (
                    rows.col(p).unwrap_or(0),
                    rows.col("lit").unwrap_or(1),
                    rows.col(o).unwrap_or(2),
                );
                let mut names: Vec<String> = Vec::new();
                for r in &rows.rows {
                    let Some(Value::Iri(pred)) = &r[pi] else {
                        continue;
                    };
                    if is_sys(pred) || pred == RDF_TYPE || irv::is_virtual(pred) {
                        continue;
                    }
                    let lit = matches!(r[li], Some(Value::Bool(true)));
                    let _ = oi;
                    let is_rel =
                        f.edge_true.contains(pred) || (!lit && !f.edge_false.contains(pred));
                    if is_rel == want_rel {
                        names.push(self.vocab.render(pred));
                    }
                }
                names.sort();
                names.dedup();
                Ok((
                    if want_rel {
                        "relationshipType"
                    } else {
                        "propertyKey"
                    },
                    names.into_iter().map(Val::Str).collect(),
                ))
            }
            other => Err(CypherError::unsupported(
                format!("procedure `{other}`"),
                None,
            )),
        }
    }
}

fn range(args: &[Val]) -> CResult<Val> {
    arity("range", args, 2, 3)?;
    let (Val::Int(a), Val::Int(b)) = (&args[0], &args[1]) else {
        return if args.iter().any(Val::is_null) {
            Ok(Val::Null)
        } else {
            Err(bad("range", &args[0]))
        };
    };
    let step = match args.get(2) {
        Some(Val::Int(s)) => *s,
        Some(Val::Null) => return Ok(Val::Null),
        Some(o) => return Err(bad("range", o)),
        None => 1,
    };
    if step == 0 {
        return Err(err("range() step must not be zero".into()));
    }
    let mut out = Vec::new();
    let mut x = *a;
    while (step > 0 && x <= *b) || (step < 0 && x >= *b) {
        out.push(Val::Int(x));
        match x.checked_add(step) {
            Some(n) => x = n,
            None => break,
        }
        if out.len() > 50_000_000 {
            return Err(err("range() is too large".into()));
        }
    }
    Ok(Val::List(out))
}

/// Cypher float rendering re-exported for tests.
pub fn float_text(x: f64) -> String {
    fmt_float(x)
}

/// The retract kind name (re-exported for the facade tests).
pub fn kind_name(k: i64) -> Option<&'static str> {
    retract_kind_name(k)
}
