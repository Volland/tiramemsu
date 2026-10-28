//! What the reasoning commands share (features `n3` and `datalog`): instants,
//! valid-time windows, and values and intervals for a person.
//!
//! Instants are epoch milliseconds or RFC 3339 dates and date-times
//! (`2024-06-01`, `2024-06-01T12:00:00Z`); a valid-time window is
//! `<from>..<to>` with either side left empty for an open end.

use tiramemsu::value::{parse_date, parse_datetime};
use tiramemsu::*;

pub fn usage(msg: impl Into<String>) -> Error {
    Error::invalid_query(msg)
}

/// An instant: epoch milliseconds, or an RFC 3339 date or date-time.
pub fn instant(s: &str) -> Result<i64> {
    let s = s.trim();
    if let Ok(ms) = s.parse::<i64>() {
        return Ok(ms);
    }
    parse_datetime(s)
        .map(|(ms, _)| ms)
        .or_else(|| parse_date(s).map(|d| d * 86_400_000))
        .ok_or_else(|| {
            usage(format!(
                "{s:?} is not an instant (epoch milliseconds or an RFC 3339 date or date-time)"
            ))
        })
}

/// A window `<from>..<to>`; an empty side is an open end.
pub fn window(s: &str) -> Result<Valid> {
    let (from, to) = s
        .split_once("..")
        .ok_or_else(|| usage(format!("{s:?} is not a window <from>..<to>")))?;
    let side = |x: &str| {
        if x.trim().is_empty() {
            Ok(None)
        } else {
            instant(x).map(Some)
        }
    };
    Ok(Valid {
        from: side(from)?,
        to: side(to)?,
    })
}

/// The view a reasoning command reads: as of a transaction or an instant, or
/// now, restricted to the statements valid at an instant.
pub fn view<'a>(
    db: &'a Db,
    as_of: Option<u64>,
    as_of_instant: Option<&str>,
    valid_at: Option<&str>,
) -> Result<View<'a>> {
    let view = match (as_of, as_of_instant) {
        (Some(t), _) => db.as_of(TimeRef::Tx(t)),
        (_, Some(i)) => db.as_of(TimeRef::Instant(instant(i)?)),
        _ => db.now(),
    };
    Ok(match valid_at {
        Some(i) => view.valid_at(instant(i)?),
        None => view,
    })
}

/// A value for a person: IRIs under one of `prefixes` (`("v:", namespace)`)
/// as prefixed names.
pub fn value(prefixes: &[(&str, &str)], v: &Value) -> String {
    match v {
        Value::Iri(iri) => prefixes
            .iter()
            .find_map(|(p, ns)| {
                iri.strip_prefix(ns)
                    .filter(|local| !local.is_empty() && !local.contains(['/', '#', ':']))
                    .map(|local| format!("{p}{local}"))
            })
            .unwrap_or_else(|| v.to_string()),
        Value::Int(i) => i.to_string(),
        Value::Bool(b) => b.to_string(),
        other => other.to_string(),
    }
}

fn bound(b: Option<i64>, open: &str) -> String {
    b.map_or(open.to_string(), |ms| rdf::datetime_lexical(ms, Some(0)))
}

pub fn interval(v: &Valid) -> String {
    if v.from.is_none() && v.to.is_none() {
        return "always".into();
    }
    format!("[{}, {})", bound(v.from, "-∞"), bound(v.to, "∞"))
}

pub fn print_json(j: &serde_json::Value) {
    println!("{j}");
}
