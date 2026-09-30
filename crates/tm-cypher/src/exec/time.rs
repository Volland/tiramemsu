//! `USE` clauses to views: a scope stack merged selector by selector
//! (design Decision 8).

use tm_core::Value;
use tm_ir::{TimeRef, TxSel, ValidSel, View};

use super::Exec;
use crate::ast::{TimeArg, TimeSel, TxClause};
use crate::error::{CResult, CypherError};
use crate::value::Val;

/// The instant in epoch milliseconds of a date, datetime or date-time text.
fn instant_of(v: &Val) -> Option<i64> {
    match v {
        Val::Date(d) => Some(d * 86_400_000),
        Val::DateTime { ms, .. } => Some(*ms),
        _ => None,
    }
}

fn arg_value(ex: &Exec, a: &TimeArg) -> CResult<Val> {
    Ok(match a {
        TimeArg::Int(i) => Val::Int(*i),
        TimeArg::Param(p) => ex
            .params
            .get(p)
            .cloned()
            .ok_or_else(|| CypherError::eval(format!("missing parameter ${p}")))?,
        TimeArg::DateTime(s) => match Value::literal(s, Some(tm_core::vocab::XSD_DATETIME), None) {
            Value::DateTime { ms, tz: Some(tz) } => Val::DateTime { ms, tz },
            Value::DateTime { ms, tz: None } => Val::DateTime { ms, tz: 0 },
            _ => return Err(CypherError::eval(format!("invalid datetime '{s}'"))),
        },
        TimeArg::Date(s) => match Value::literal(s, Some(tm_core::vocab::XSD_DATE), None) {
            Value::Date(d) => Val::Date(d),
            _ => return Err(CypherError::eval(format!("invalid date '{s}'"))),
        },
    })
}

/// Merges `t` into the current view of `ex` (handle → query → CALL body → nested).
pub fn resolve(ex: &mut Exec, t: &TimeSel) -> CResult<View> {
    let base = ex.view();
    let tx = match &t.tx {
        None => None,
        Some(TxClause::History) => Some(TxSel::History),
        Some(TxClause::AsOf(a, _)) => {
            let v = arg_value(ex, a)?;
            Some(TxSel::AsOf(match v {
                Val::Int(i) => TimeRef::Tx(i.max(0) as u64),
                Val::DateTime { ms, .. } => TimeRef::Instant(ms),
                other => {
                    return Err(CypherError::eval(format!(
                        "AS OF needs an integer or a datetime, got {}",
                        other.type_name()
                    )))
                }
            }))
        }
    };
    let valid = match &t.valid {
        None => None,
        Some((a, _)) => {
            let v = arg_value(ex, a)?;
            match instant_of(&v) {
                Some(ms) => Some(ValidSel::At(ms)),
                None => {
                    return Err(CypherError::eval(format!(
                        "VALID AT needs a date or a datetime, got {}",
                        v.type_name()
                    )))
                }
            }
        }
    };
    Ok(base.overlay(tx, valid))
}
