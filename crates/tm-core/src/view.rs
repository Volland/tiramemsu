//! View descriptors and the one function that writes time predicates.

use crate::exec::Params;

/// A point in transaction time.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum TimeRef {
    /// A transaction number `t`.
    Tx(u64),
    /// A wall-clock instant in epoch ms; resolves to the largest `t` with `instant <= ms`.
    Instant(i64),
}

/// Transaction-time selector.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum TxSel {
    /// Live statements (`t_ret IS NULL`).
    #[default]
    Now,
    /// Statements believed at a past transaction.
    AsOf(TimeRef),
    /// Every statement ever committed.
    History,
}

/// Valid-time selector.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum ValidSel {
    /// No valid-time filter (the default).
    #[default]
    Unfiltered,
    /// Only statements valid at this instant (epoch ms).
    At(i64),
}

/// A view: a transaction-time selector plus a valid-time selector. A pure value.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ViewSpec {
    /// Transaction time.
    pub tx: TxSel,
    /// Valid time.
    pub valid: ValidSel,
}

impl ViewSpec {
    /// The now view.
    pub const NOW: ViewSpec = ViewSpec {
        tx: TxSel::Now,
        valid: ValidSel::Unfiltered,
    };

    /// The now view.
    pub fn now() -> ViewSpec {
        ViewSpec::NOW
    }

    /// An as-of view.
    pub fn as_of(at: TimeRef) -> ViewSpec {
        ViewSpec {
            tx: TxSel::AsOf(at),
            valid: ValidSel::Unfiltered,
        }
    }

    /// The history view.
    pub fn history() -> ViewSpec {
        ViewSpec {
            tx: TxSel::History,
            valid: ValidSel::Unfiltered,
        }
    }

    /// A copy of this view filtered to statements valid at `ms`.
    pub fn valid_at(self, ms: i64) -> ViewSpec {
        ViewSpec {
            valid: ValidSel::At(ms),
            ..self
        }
    }
}

/// Emits the time predicates of `spec` for the `triple` alias `alias`, binding
/// constants into `params`. This is the only code that writes time predicates.
///
/// | TxSel | Predicate |
/// |---|---|
/// | `Now` | `a.t_ret IS NULL` (verbatim, so the partial `live_*` indexes apply) |
/// | `AsOf(Tx(t))` | `a.t_add <= ?t AND (a.t_ret IS NULL OR a.t_ret > ?t)` |
/// | `AsOf(Instant(ms))` | the same, with `?t` = `(SELECT coalesce(max(t), 0) FROM tx WHERE instant <= ?ms)` |
/// | `History` | none |
///
/// `ValidSel::At(d)` adds `(a.v_from IS NULL OR a.v_from <= ?d) AND (a.v_to IS NULL OR a.v_to > ?d)`.
/// The result may be empty (history, unfiltered).
// @lat: [[query#Views and Scans]]
pub fn scan_predicates(spec: &ViewSpec, alias: &str, params: &mut Params) -> String {
    let a = alias;
    let mut parts: Vec<String> = Vec::new();
    match spec.tx {
        TxSel::Now => parts.push(format!("{a}.t_ret IS NULL")),
        TxSel::AsOf(at) => {
            let t = match at {
                TimeRef::Tx(t) => params.push(t as i64),
                TimeRef::Instant(ms) => {
                    let p = params.push(ms);
                    format!("(SELECT coalesce(max(t), 0) FROM tx WHERE instant <= {p})")
                }
            };
            parts.push(format!(
                "{a}.t_add <= {t} AND ({a}.t_ret IS NULL OR {a}.t_ret > {t})"
            ));
        }
        TxSel::History => {}
    }
    if let ValidSel::At(d) = spec.valid {
        let d = params.push(d);
        parts.push(format!(
            "({a}.v_from IS NULL OR {a}.v_from <= {d}) AND ({a}.v_to IS NULL OR {a}.v_to > {d})"
        ));
    }
    parts.join(" AND ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes() {
        let mut p = Params::new();
        assert_eq!(
            scan_predicates(&ViewSpec::NOW, "a", &mut p),
            "a.t_ret IS NULL"
        );
        let mut p = Params::new();
        assert_eq!(
            scan_predicates(&ViewSpec::as_of(TimeRef::Tx(5)), "a", &mut p),
            "a.t_add <= ?1 AND (a.t_ret IS NULL OR a.t_ret > ?1)"
        );
        let mut p = Params::new();
        assert_eq!(scan_predicates(&ViewSpec::history(), "a", &mut p), "");
        let mut p = Params::new();
        assert_eq!(
            scan_predicates(&ViewSpec::history().valid_at(9), "a", &mut p),
            "(a.v_from IS NULL OR a.v_from <= ?1) AND (a.v_to IS NULL OR a.v_to > ?1)"
        );
        let mut p = Params::new();
        let s = scan_predicates(&ViewSpec::as_of(TimeRef::Instant(7)), "t0", &mut p);
        assert!(s.contains("SELECT coalesce(max(t), 0) FROM tx WHERE instant <= ?1"));
        assert_eq!(p.values().len(), 1);
    }
}
