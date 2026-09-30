//! The per-pattern View: a transaction-time and a valid-time selector.

use tm_core::ViewSpec;
pub use tm_core::{TimeRef, TxSel, ValidSel};

/// The time selection of one triple or path pattern.
///
/// A view pairs a transaction-time selector (what the database believed) with a
/// valid-time selector (when the fact held). The default is [`View::NOW`]; valid
/// time is filtered only when asked for.
///
/// # Example
///
/// ```
/// use tm_ir::{TimeRef, TxSel, ValidSel, View};
///
/// let v = View::as_of_tx(150).valid_at(1_700_000_000_000);
/// assert_eq!(v.tx, TxSel::AsOf(TimeRef::Tx(150)));
/// assert_eq!(v.valid, ValidSel::At(1_700_000_000_000));
/// assert_eq!(View::default(), View::NOW);
/// ```
///
/// Pattern views are always explicit: front ends lower with the handle's
/// [`View`] (the view descriptor) as the default and [`View::overlay`] a
/// query-level or per-pattern clause on top of it.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct View {
    /// Transaction time: `Now`, `AsOf(TimeRef)` or `History`.
    pub tx: TxSel,
    /// Valid time: `Unfiltered` or `At(epoch_ms)`.
    pub valid: ValidSel,
}

impl View {
    /// `{Now, Unfiltered}`: live statements, valid time unfiltered.
    pub const NOW: View = View {
        tx: TxSel::Now,
        valid: ValidSel::Unfiltered,
    };

    /// `{Now, Unfiltered}`.
    pub fn now() -> View {
        View::NOW
    }

    /// `{AsOf(at), Unfiltered}`.
    pub fn as_of(at: TimeRef) -> View {
        View {
            tx: TxSel::AsOf(at),
            valid: ValidSel::Unfiltered,
        }
    }

    /// `{AsOf(Tx(t)), Unfiltered}`.
    pub fn as_of_tx(t: u64) -> View {
        View::as_of(TimeRef::Tx(t))
    }

    /// `{History, Unfiltered}`.
    pub fn history() -> View {
        View {
            tx: TxSel::History,
            valid: ValidSel::Unfiltered,
        }
    }

    /// This view restricted to statements valid at `ms`.
    pub fn valid_at(self, ms: i64) -> View {
        View {
            valid: ValidSel::At(ms),
            ..self
        }
    }

    /// Merges a clause part by part: a given selector replaces this view's
    /// selector of the same kind, a missing one keeps it.
    pub fn overlay(self, tx: Option<TxSel>, valid: Option<ValidSel>) -> View {
        View {
            tx: tx.unwrap_or(self.tx),
            valid: valid.unwrap_or(self.valid),
        }
    }

    /// The equivalent core view spec.
    pub fn spec(self) -> ViewSpec {
        ViewSpec {
            tx: self.tx,
            valid: self.valid,
        }
    }
}

impl From<ViewSpec> for View {
    fn from(s: ViewSpec) -> View {
        View {
            tx: s.tx,
            valid: s.valid,
        }
    }
}

impl From<View> for ViewSpec {
    fn from(v: View) -> ViewSpec {
        v.spec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // query-ir "Overlaying one part of a view"
    #[test]
    fn overlay_replaces_only_the_given_part() {
        let d = 1_700_000_000_000;
        let base = View::now().valid_at(d);
        let v = base.overlay(Some(TxSel::AsOf(TimeRef::Tx(150))), None);
        assert_eq!(
            v,
            View {
                tx: TxSel::AsOf(TimeRef::Tx(150)),
                valid: ValidSel::At(d)
            }
        );
        assert_eq!(base.overlay(None, None), base);
        assert_eq!(base.overlay(None, Some(ValidSel::Unfiltered)), View::now());
    }

    #[test]
    fn spec_round_trip() {
        let v = View::as_of_tx(3).valid_at(9);
        assert_eq!(View::from(v.spec()), v);
    }
}
