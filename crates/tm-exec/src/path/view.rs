//! The `view` argument text of `tm_path` (with its `timeRespecting` and `hopCap`
//! parts) and its resolution.

use tm_core::value::{parse_date, parse_datetime};
use tm_core::vocab::TM;
use tm_core::{Executor, Result, TimeRef, TxSel, ValidSel, ViewSpec};

use super::engine::TimeRespecting;
use crate::plan::resolve::resolve;
use crate::scan::ResolvedView;

fn instant_ms(text: &str) -> Option<i64> {
    if text.contains('T') {
        parse_datetime(text).map(|(ms, _)| ms)
    } else {
        parse_date(text).map(|d| d * 86_400_000)
    }
}

/// An integer of epoch milliseconds, else an RFC 3339 date or date-time.
fn epoch_or_instant(text: &str) -> Option<i64> {
    let digits = text.strip_prefix('-').unwrap_or(text);
    if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
        text.parse().ok()
    } else {
        instant_ms(text)
    }
}

/// Parses `now | asOf/<t> | asOf/<instant> | history`, optionally followed by
/// `;validAt/<d>`, or `validAt/<d>` alone; every part may carry the `tm:` IRI prefix.
/// A `timeRespecting` part is not a view: use [`parse_view_arg`].
pub fn parse_view(text: &str) -> std::result::Result<ViewSpec, String> {
    match parse_view_arg(text)? {
        (spec, None) => Ok(spec),
        (_, Some(_)) => Err(format!(
            "malformed view `{text}`: timeRespecting is not a view"
        )),
    }
}

/// The parsed `view` argument of a `tm_path` call.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ViewArg {
    /// The view of every hop.
    pub spec: ViewSpec,
    /// The `timeRespecting[/<t>]` part.
    pub time_respecting: Option<TimeRespecting>,
    /// The `hopCap` part: the `max_hops` argument is the configured hop cap on an
    /// unbounded expression (completeness reporting only).
    pub hop_cap: bool,
}

/// Parses the `view` argument of `tm_path`: the parts of [`parse_view`] plus an
/// optional `timeRespecting` or `timeRespecting/<t>` part (`t` an RFC 3339 date or
/// date-time, or an integer of epoch milliseconds), in any order, each part at
/// most once. `timeRespecting` alone means `now;timeRespecting`. A `hopCap` part
/// is refused here; [`parse_call_view`] reads it.
pub fn parse_view_arg(
    text: &str,
) -> std::result::Result<(ViewSpec, Option<TimeRespecting>), String> {
    let a = parse_call_view(text)?;
    if a.hop_cap {
        return Err(format!(
            "malformed view `{text}`: hopCap is a tm_path option"
        ));
    }
    Ok((a.spec, a.time_respecting))
}

/// [`parse_view_arg`] plus a `hopCap` part (at most once, any order), which marks
/// the `max_hops` argument of the call as the configured hop cap.
pub fn parse_call_view(text: &str) -> std::result::Result<ViewArg, String> {
    let mut spec = ViewSpec::NOW;
    let mut hop_cap = false;
    let (mut tx_set, mut valid_set) = (false, false);
    let mut time_respecting = None;
    for part in text.split(';') {
        let part = part.trim();
        let part = part.strip_prefix(TM).unwrap_or(part);
        let bad = || format!("malformed view `{text}`");
        if part == "hopCap" {
            if hop_cap {
                return Err(bad());
            }
            hop_cap = true;
        } else if part == "timeRespecting" || part.starts_with("timeRespecting/") {
            if time_respecting.is_some() {
                return Err(bad());
            }
            let after = match part.strip_prefix("timeRespecting/") {
                None => None,
                Some(a) => Some(epoch_or_instant(a).ok_or_else(bad)?),
            };
            time_respecting = Some(TimeRespecting { after });
        } else if part == "now" || part == "history" || part.starts_with("asOf/") {
            if tx_set {
                return Err(bad());
            }
            tx_set = true;
            spec.tx = if part == "now" {
                TxSel::Now
            } else if part == "history" {
                TxSel::History
            } else {
                let a = &part["asOf/".len()..];
                if !a.is_empty() && a.bytes().all(|b| b.is_ascii_digit()) {
                    TxSel::AsOf(TimeRef::Tx(a.parse().map_err(|_| bad())?))
                } else {
                    TxSel::AsOf(TimeRef::Instant(instant_ms(a).ok_or_else(bad)?))
                }
            };
        } else if let Some(a) = part.strip_prefix("validAt/") {
            if valid_set {
                return Err(bad());
            }
            valid_set = true;
            spec.valid = ValidSel::At(instant_ms(a).ok_or_else(bad)?);
        } else {
            return Err(bad());
        }
    }
    Ok(ViewArg {
        spec,
        time_respecting,
        hop_cap,
    })
}

/// Resolves a view on `exec`; `None` when it selects a point before transaction 1.
pub fn resolve_view(exec: &mut dyn Executor, spec: ViewSpec) -> Result<Option<ResolvedView>> {
    resolve(exec, &spec.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    // path-table-function "View argument text"
    #[test]
    fn view_forms() {
        assert_eq!(parse_view("now").unwrap(), ViewSpec::NOW);
        assert_eq!(
            parse_view("asOf/15").unwrap(),
            ViewSpec::as_of(TimeRef::Tx(15))
        );
        assert_eq!(parse_view("history").unwrap(), ViewSpec::history());
        assert_eq!(
            parse_view("history;validAt/2021-06-01").unwrap(),
            ViewSpec::history().valid_at(1_622_505_600_000)
        );
        assert_eq!(
            parse_view("validAt/2021-06-01").unwrap(),
            ViewSpec::NOW.valid_at(1_622_505_600_000)
        );
        assert_eq!(
            parse_view("urn:tiramemsu:tm:asOf/15").unwrap(),
            parse_view("asOf/15").unwrap()
        );
        assert!(matches!(
            parse_view("asOf/2021-06-01T00:00:00Z").unwrap().tx,
            TxSel::AsOf(TimeRef::Instant(_))
        ));
        for bad in [
            "asOf/yesterday",
            "asOf/",
            "then",
            "now;now",
            "validAt/5",
            "",
        ] {
            assert!(parse_view(bad).is_err(), "{bad}");
        }
        // a time-respecting part is not a view
        assert!(parse_view("timeRespecting").is_err());
    }

    // @lat: [[tests#Query#Time Respecting View Text]]
    #[test]
    fn time_respecting_view_parts() {
        let tr = |after| Some(TimeRespecting { after });
        assert_eq!(parse_view_arg("now").unwrap(), (ViewSpec::NOW, None));
        assert_eq!(
            parse_view_arg("timeRespecting").unwrap(),
            (ViewSpec::NOW, tr(None))
        );
        assert_eq!(
            parse_view_arg("now;timeRespecting/2024-06-01").unwrap(),
            (ViewSpec::NOW, tr(Some(1_717_200_000_000)))
        );
        assert_eq!(
            parse_view_arg("timeRespecting/1717200000000;asOf/15").unwrap(),
            (
                ViewSpec::as_of(TimeRef::Tx(15)),
                tr(Some(1_717_200_000_000))
            )
        );
        assert_eq!(
            parse_view_arg("history;validAt/2021-06-01;timeRespecting/-5").unwrap(),
            (
                ViewSpec::history().valid_at(1_622_505_600_000),
                tr(Some(-5))
            )
        );
        assert_eq!(
            parse_view_arg("urn:tiramemsu:tm:timeRespecting/2024-06-01T12:00:00Z").unwrap(),
            (ViewSpec::NOW, tr(Some(1_717_243_200_000)))
        );
        let a = parse_call_view("hopCap;asOf/15").unwrap();
        assert!(a.hop_cap);
        assert_eq!(a.spec, ViewSpec::as_of(TimeRef::Tx(15)));
        assert!(!parse_call_view("now").unwrap().hop_cap);
        assert!(parse_view_arg("hopCap").is_err());
        assert!(parse_call_view("hopCap;hopCap").is_err());
        for bad in [
            "timeRespecting;timeRespecting",
            "timeRespecting/soon",
            "timeRespecting/",
            "timeRespectingly",
            "now;timeRespecting/1;now",
        ] {
            assert!(parse_view_arg(bad).is_err(), "{bad}");
        }
    }
}
