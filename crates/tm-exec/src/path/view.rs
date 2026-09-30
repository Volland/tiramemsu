//! The `view` argument text of `tm_path` and its resolution.

use tm_core::value::{parse_date, parse_datetime};
use tm_core::vocab::TM;
use tm_core::{Executor, Result, TimeRef, TxSel, ValidSel, ViewSpec};

use crate::plan::resolve::resolve;
use crate::scan::ResolvedView;

fn instant_ms(text: &str) -> Option<i64> {
    if text.contains('T') {
        parse_datetime(text).map(|(ms, _)| ms)
    } else {
        parse_date(text).map(|d| d * 86_400_000)
    }
}

/// Parses `now | asOf/<t> | asOf/<instant> | history`, optionally followed by
/// `;validAt/<d>`, or `validAt/<d>` alone; every part may carry the `tm:` IRI prefix.
pub fn parse_view(text: &str) -> std::result::Result<ViewSpec, String> {
    let mut spec = ViewSpec::NOW;
    let (mut tx_set, mut valid_set) = (false, false);
    for part in text.split(';') {
        let part = part.trim();
        let part = part.strip_prefix(TM).unwrap_or(part);
        let bad = || format!("malformed view `{text}`");
        if part == "now" || part == "history" || part.starts_with("asOf/") {
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
    Ok(spec)
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
    }
}
