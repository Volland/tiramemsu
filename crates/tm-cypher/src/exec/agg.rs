//! Aggregates (`cypher-read`, "Aggregation").

use std::collections::HashSet;

use super::{Exec, Row};
use crate::ast::Expr;
use crate::error::{CResult, CypherError};
use crate::value::{group_key, order, Val};

impl Exec<'_> {
    /// Evaluates the aggregate call `name(args)` over `group`.
    pub(crate) fn aggregate(
        &mut self,
        name: &str,
        distinct: bool,
        args: &[Expr],
        group: &[Row],
    ) -> CResult<Val> {
        let n = name.to_ascii_lowercase();
        let mut vals: Vec<Val> = Vec::with_capacity(group.len());
        let mut extra: Option<Val> = None;
        for r in group {
            let Some(a) = args.first() else {
                return Err(CypherError::eval(format!("{name}() needs an argument")));
            };
            let v = self.eval(a, r)?;
            if extra.is_none() && args.len() > 1 {
                extra = Some(self.eval(&args[1], r)?);
            }
            if !v.is_null() {
                vals.push(v);
            }
        }
        if distinct {
            let mut seen = HashSet::new();
            vals.retain(|v| seen.insert(group_key(v)));
        }
        Ok(match n.as_str() {
            "count" => Val::Int(vals.len() as i64),
            "collect" => Val::List(vals),
            "min" => vals.into_iter().min_by(order).unwrap_or(Val::Null),
            "max" => vals.into_iter().max_by(order).unwrap_or(Val::Null),
            "sum" => {
                if vals.is_empty() {
                    return Ok(Val::Null);
                }
                let mut all_int = true;
                let mut isum: i64 = 0;
                let mut fsum = 0.0f64;
                for v in &vals {
                    match v {
                        Val::Int(i) => {
                            isum = isum
                                .checked_add(*i)
                                .ok_or_else(|| CypherError::eval("integer overflow"))?;
                            fsum += *i as f64;
                        }
                        Val::Float(x) => {
                            all_int = false;
                            fsum += x;
                        }
                        o => {
                            return Err(CypherError::eval(format!(
                                "sum() is not defined for {}",
                                o.type_name()
                            )))
                        }
                    }
                }
                if all_int {
                    Val::Int(isum)
                } else {
                    Val::Float(fsum)
                }
            }
            "avg" => {
                if vals.is_empty() {
                    return Ok(Val::Null);
                }
                let mut s = 0.0;
                for v in &vals {
                    match v {
                        Val::Int(i) => s += *i as f64,
                        Val::Float(x) => s += x,
                        o => {
                            return Err(CypherError::eval(format!(
                                "avg() is not defined for {}",
                                o.type_name()
                            )))
                        }
                    }
                }
                Val::Float(s / vals.len() as f64)
            }
            "stdev" | "stdevp" => {
                let xs = floats(&vals, &n)?;
                if xs.is_empty() {
                    return Ok(Val::Float(0.0));
                }
                let mean = xs.iter().sum::<f64>() / xs.len() as f64;
                let ss: f64 = xs.iter().map(|x| (x - mean).powi(2)).sum();
                let denom = if n == "stdev" {
                    xs.len() as f64 - 1.0
                } else {
                    xs.len() as f64
                };
                if denom <= 0.0 {
                    return Ok(Val::Float(0.0));
                }
                Val::Float((ss / denom).sqrt())
            }
            "percentilecont" | "percentiledisc" => {
                let mut xs = floats(&vals, &n)?;
                let p = match extra {
                    Some(Val::Float(p)) => p,
                    Some(Val::Int(p)) => p as f64,
                    _ => return Err(CypherError::eval("percentile needs a number in [0, 1]")),
                };
                if !(0.0..=1.0).contains(&p) {
                    return Err(CypherError::eval("percentile must be between 0 and 1"));
                }
                if xs.is_empty() {
                    return Ok(Val::Null);
                }
                xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                if n == "percentiledisc" {
                    let idx = ((p * xs.len() as f64).ceil() as usize)
                        .saturating_sub(1)
                        .min(xs.len() - 1);
                    let v = xs[idx];
                    // keep integers integral when the inputs were integers
                    if vals.iter().all(|v| matches!(v, Val::Int(_))) {
                        Val::Int(v as i64)
                    } else {
                        Val::Float(v)
                    }
                } else {
                    let pos = p * (xs.len() as f64 - 1.0);
                    let (lo, hi) = (pos.floor() as usize, pos.ceil() as usize);
                    Val::Float(xs[lo] + (xs[hi] - xs[lo]) * (pos - lo as f64))
                }
            }
            other => {
                return Err(CypherError::unsupported(
                    format!("aggregate `{other}`"),
                    None,
                ))
            }
        })
    }
}

fn floats(vals: &[Val], name: &str) -> CResult<Vec<f64>> {
    vals.iter()
        .map(|v| match v {
            Val::Int(i) => Ok(*i as f64),
            Val::Float(x) => Ok(*x),
            o => Err(CypherError::eval(format!(
                "{name}() is not defined for {}",
                o.type_name()
            ))),
        })
        .collect()
}
