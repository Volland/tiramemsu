//! The view → SQL predicate mapping: the only code in `tm-exec` that writes time
//! predicates (`lat.md/query#Views and Scans`). Pattern aliases, the canonical-eid
//! subquery, virtual predicates, lookups, the volatile branch and the `tm_path`
//! view text all go through it.
//!
//! It delegates to [`tm_core::scan_predicates`], so the whole workspace has one
//! mapping; views reach it already resolved (`AsOf` holds a transaction number).

use tm_core::value::format_datetime;
use tm_core::{TimeRef, TxSel, ValidSel, ViewSpec};

use crate::sqlgen::ParamAlloc;

/// A resolved transaction-time selector.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum ResolvedTx {
    /// Live statements.
    Now,
    /// Statements believed at transaction `t` (`t ≥ 1`).
    AsOf(u64),
    /// Every statement.
    History,
}

/// A view whose time references are resolved to transaction numbers.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct ResolvedView {
    /// Transaction time.
    pub tx: ResolvedTx,
    /// Valid time.
    pub valid: ValidSel,
}

impl ResolvedView {
    /// `{Now, Unfiltered}`.
    pub const NOW: ResolvedView = ResolvedView {
        tx: ResolvedTx::Now,
        valid: ValidSel::Unfiltered,
    };

    fn spec(&self) -> ViewSpec {
        ViewSpec {
            tx: match self.tx {
                ResolvedTx::Now => TxSel::Now,
                ResolvedTx::AsOf(t) => TxSel::AsOf(TimeRef::Tx(t)),
                ResolvedTx::History => TxSel::History,
            },
            valid: self.valid,
        }
    }
}

/// The time predicates of `v` on the `triple` alias `alias`, binding `t` and `d`
/// as parameters (one number for `t`, used twice):
///
/// | View part | Predicate |
/// |---|---|
/// | `Now` | `a.t_ret IS NULL` (verbatim) |
/// | `AsOf(t)` | `a.t_add <= ?t AND (a.t_ret IS NULL OR a.t_ret > ?t)` |
/// | `History` | none |
/// | `At(d)` | `(a.v_from IS NULL OR a.v_from <= ?d) AND (a.v_to IS NULL OR a.v_to > ?d)` |
// @lat: [[query#Views and Scans]]
pub fn view_predicates(alias: &str, v: &ResolvedView, p: &mut ParamAlloc) -> Vec<String> {
    let s = tm_core::scan_predicates(&v.spec(), alias, p.core());
    if s.is_empty() {
        Vec::new()
    } else {
        vec![s]
    }
}

/// The `view` argument text of `tm_path`: `now | asOf/<t> | history`, optionally
/// followed by `;validAt/<RFC 3339 instant>`.
pub fn view_text(v: &ResolvedView) -> String {
    let tx = match v.tx {
        ResolvedTx::Now => "now".to_string(),
        ResolvedTx::AsOf(t) => format!("asOf/{t}"),
        ResolvedTx::History => "history".to_string(),
    };
    match v.valid {
        ValidSel::Unfiltered => tx,
        ValidSel::At(d) => format!("{tx};validAt/{}", format_datetime(d, Some(0))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tm_core::SqlValue;

    fn preds(v: ResolvedView) -> (String, Vec<SqlValue>) {
        let mut p = ParamAlloc::default();
        p.push(SqlValue::Integer(-1)); // an earlier parameter
        let s = view_predicates("t0", &v, &mut p).join(" AND ");
        (s, p.values().to_vec())
    }

    #[test]
    fn six_combinations() {
        let d = 1_700_000_000_000;
        let now = ResolvedView::NOW;
        let asof = ResolvedView {
            tx: ResolvedTx::AsOf(150),
            valid: ValidSel::Unfiltered,
        };
        let hist = ResolvedView {
            tx: ResolvedTx::History,
            valid: ValidSel::Unfiltered,
        };
        let at = |v: ResolvedView| ResolvedView {
            valid: ValidSel::At(d),
            ..v
        };
        let valid = "(t0.v_from IS NULL OR t0.v_from <= ?2) AND (t0.v_to IS NULL OR t0.v_to > ?2)";
        assert_eq!(preds(now).0, "t0.t_ret IS NULL");
        let (s, p) = preds(asof);
        assert_eq!(s, "t0.t_add <= ?2 AND (t0.t_ret IS NULL OR t0.t_ret > ?2)");
        assert_eq!(p[1], SqlValue::Integer(150));
        assert_eq!(p.len(), 2, "one parameter number reused for t");
        assert_eq!(preds(hist).0, "");
        assert_eq!(preds(at(now)).0, format!("t0.t_ret IS NULL AND {valid}"));
        let (s, p) = preds(at(asof));
        assert_eq!(
            s,
            "t0.t_add <= ?2 AND (t0.t_ret IS NULL OR t0.t_ret > ?2) AND \
             (t0.v_from IS NULL OR t0.v_from <= ?3) AND (t0.v_to IS NULL OR t0.v_to > ?3)"
        );
        assert_eq!(p[2], SqlValue::Integer(d));
        let (s, p) = preds(at(hist));
        assert_eq!(s, valid);
        assert_eq!(p[1], SqlValue::Integer(d));
    }

    #[test]
    fn path_view_text() {
        assert_eq!(view_text(&ResolvedView::NOW), "now");
        let v = ResolvedView {
            tx: ResolvedTx::AsOf(150),
            valid: ValidSel::At(1_740_787_200_000),
        };
        assert_eq!(view_text(&v), "asOf/150;validAt/2025-03-01T00:00:00.000Z");
    }
}
