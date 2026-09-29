//! Transaction options, operation arguments and the transaction report.

use crate::error::{Error, Result};
use crate::id::{Eid, ObjectId, TxId};
use crate::value::Value;

pub use crate::event::{Event, Op};

/// A half-open valid-time interval `[from, to)` in epoch ms; `None` is unbounded.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Valid {
    /// Inclusive start.
    pub from: Option<i64>,
    /// Exclusive end.
    pub to: Option<i64>,
}

impl Valid {
    /// Valid for all time.
    pub const ALWAYS: Valid = Valid {
        from: None,
        to: None,
    };

    /// `[from, to)`.
    pub fn between(from: i64, to: i64) -> Valid {
        Valid {
            from: Some(from),
            to: Some(to),
        }
    }

    /// `[from, unbounded)`.
    pub fn from(from: i64) -> Valid {
        Valid {
            from: Some(from),
            to: None,
        }
    }

    /// `[unbounded, to)`.
    pub fn until(to: i64) -> Valid {
        Valid {
            from: None,
            to: Some(to),
        }
    }

    /// Fails with `InvalidInterval` when both bounds are present and `from >= to`.
    pub fn check(&self) -> Result<()> {
        match (self.from, self.to) {
            (Some(f), Some(t)) if f >= t => Err(Error::InvalidInterval { v_from: f, v_to: t }),
            _ => Ok(()),
        }
    }

    /// True when the interval contains the instant `d`.
    pub fn contains(&self, d: i64) -> bool {
        self.from.is_none_or(|f| f <= d) && self.to.is_none_or(|t| t > d)
    }

    /// The overlap test of `lat.md/time-model#Operations#Assert`.
    pub fn overlaps(&self, b: &Valid) -> bool {
        let a = self;
        (a.from.is_none() || b.to.is_none() || a.from < b.to)
            && (b.from.is_none() || a.to.is_none() || b.from < a.to)
    }
}

/// What assert does when a live overlapping match exists.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum OnExisting {
    /// Return the match unchanged.
    #[default]
    Return,
    /// Return the match and record `sys:confirmedBy` for it.
    Confirm,
}

/// Options of an assert.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct AssertOpts {
    /// Requested valid time.
    pub valid: Valid,
    /// Policy for an existing match.
    pub on_existing: OnExisting,
}

/// Result of an assert.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Asserted {
    /// A new statement was inserted.
    New(Eid),
    /// A live overlapping statement already existed.
    Existing(Eid),
}

impl Asserted {
    /// The eid in either case.
    pub fn eid(self) -> Eid {
        match self {
            Asserted::New(e) | Asserted::Existing(e) => e,
        }
    }

    /// True for `New`.
    pub fn is_new(self) -> bool {
        matches!(self, Asserted::New(_))
    }
}

/// Why a statement was retracted.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum RetKind {
    /// Named by a retract or matched by retract-matching.
    Explicit = 0,
    /// Reached from an explicit retraction.
    Cascade = 1,
    /// In the cascade set of a superseded statement.
    Supersede = 2,
    /// In the cascade set of a cardinality-one replacement.
    Cardinality = 3,
}

impl RetKind {
    /// From the stored integer.
    pub fn from_i64(v: i64) -> Option<RetKind> {
        match v {
            0 => Some(RetKind::Explicit),
            1 => Some(RetKind::Cascade),
            2 => Some(RetKind::Supersede),
            3 => Some(RetKind::Cardinality),
            _ => None,
        }
    }
}

/// Options of a transaction.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct TxOptions {
    /// Run with full semantics, report, then discard (ids are burned).
    pub dry_run: bool,
    /// Largest cascade set allowed per root retraction.
    pub max_cascade: usize,
}

impl Default for TxOptions {
    fn default() -> Self {
        TxOptions {
            dry_run: false,
            max_cascade: 10_000,
        }
    }
}

/// A supersede patch: only the object and the valid-time bounds can change.
///
/// `v_from`/`v_to`: `None` keeps the old bound, `Some(None)` clears it,
/// `Some(Some(x))` sets it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Patch {
    /// New object.
    pub o: Option<Value>,
    /// New start bound.
    pub v_from: Option<Option<i64>>,
    /// New end bound.
    pub v_to: Option<Option<i64>>,
}

/// A field value of a patch received from a binding (see [`Patch::from_fields`]).
#[derive(Clone, Debug, PartialEq)]
pub enum PatchField {
    /// A value (for `o`, and for the rejected `s` / `p`).
    Value(Value),
    /// A valid-time bound; `None` clears it (for `v_from` / `v_to`).
    Time(Option<i64>),
}

impl Patch {
    /// Builds a patch from named fields, as bindings receive it. Only `o`, `v_from`
    /// and `v_to` are accepted; `s` or `p` (or any unknown field, or a value of the
    /// wrong kind) fails with `InvalidPatch`.
    pub fn from_fields<'k>(
        fields: impl IntoIterator<Item = (&'k str, PatchField)>,
    ) -> Result<Patch> {
        let mut patch = Patch::default();
        for (name, field) in fields {
            match (name, field) {
                ("s" | "p", _) => {
                    return Err(Error::InvalidPatch(format!(
                        "{name} cannot be patched; supersede changes only o, v_from and v_to"
                    )))
                }
                ("o", PatchField::Value(v)) => patch.o = Some(v),
                ("v_from", PatchField::Time(t)) => patch.v_from = Some(t),
                ("v_to", PatchField::Time(t)) => patch.v_to = Some(t),
                (other, f) => {
                    return Err(Error::InvalidPatch(format!(
                        "unexpected patch field {other} = {f:?}"
                    )))
                }
            }
        }
        Ok(patch)
    }

    /// A patch that sets the object.
    pub fn object(o: impl Into<Value>) -> Patch {
        Patch {
            o: Some(o.into()),
            ..Patch::default()
        }
    }
}

/// What a committed (or dry-run) transaction did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TxReport {
    /// Transaction number (the would-be number for a dry run).
    pub t: TxId,
    /// Transaction instant.
    pub instant: i64,
    /// Every inserted statement, in insertion order.
    pub asserted: Vec<Eid>,
    /// Existing matches returned by assert, deduplicated, excluding `asserted`.
    pub existing: Vec<Eid>,
    /// Every retraction in order, with its kind.
    pub retracted: Vec<(Eid, RetKind)>,
    /// `(old, new)` pairs of every supersede, root first.
    pub superseded: Vec<(Eid, Eid)>,
}

/// One statement row as returned by a lookup.
///
/// Rows from an as-of view report `t_ret` and `ret_kind` as `None`, because any
/// retraction visible there happened after the view's transaction; use the
/// history view for real lifetimes.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Triple {
    /// Statement id.
    pub eid: Eid,
    /// Subject.
    pub s: ObjectId,
    /// Predicate.
    pub p: ObjectId,
    /// Object.
    pub o: ObjectId,
    /// Transaction that inserted it.
    pub t_add: TxId,
    /// Transaction that retracted it.
    pub t_ret: Option<TxId>,
    /// Valid-time start.
    pub v_from: Option<i64>,
    /// Valid-time end.
    pub v_to: Option<i64>,
    /// Why it was retracted.
    pub ret_kind: Option<RetKind>,
}

impl Triple {
    /// The valid interval.
    pub fn valid(&self) -> Valid {
        Valid {
            from: self.v_from,
            to: self.v_to,
        }
    }
}
