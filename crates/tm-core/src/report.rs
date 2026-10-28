//! Transaction options, operation arguments and the transaction report.

use crate::error::{Error, Result};
use crate::id::{Eid, ObjectId, TxId};
use crate::value::Value;

pub use crate::event::{Event, Op};

/// A half-open valid-time interval `[from, to)` in epoch ms; `None` is unbounded.
///
/// Valid time says when a fact is true in the world, independent of when the
/// database learned it (transaction time).
///
/// ```
/// use tm_core::Valid;
///
/// let v = Valid::between(100, 200);
/// assert!(v.contains(100) && !v.contains(200));
/// assert!(Valid::ALWAYS.contains(i64::MIN));
/// assert!(v.check().is_ok());
/// assert!(Valid::between(5, 5).check().is_err());
/// ```
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

    /// The overlap test of `lat.md/time-model#Operations#Assert`: the same test as
    /// [`Valid::intersects`].
    pub fn overlaps(&self, b: &Valid) -> bool {
        self.intersects(b)
    }

    /// True when the two intervals share an instant: `a.from < b.to ∧ b.from <
    /// a.to`, an open end comparing past every bound. This is the one definition
    /// of overlap: assert's test, SPARQL's and Cypher's `intersects` and the
    /// Datalog builtin use it. It holds exactly for the relations `overlaps`,
    /// `starts`, `during`, `finishes`, `equals` and their converses.
    // @tg: implements:: [[openspec:datalog-temporal#Allen relations]] {since: "2026-10-28", confidence: partial}
    pub fn intersects(&self, b: &Valid) -> bool {
        let a = self;
        (a.from.is_none() || b.to.is_none() || a.from < b.to)
            && (b.from.is_none() || a.to.is_none() || b.from < a.to)
    }

    /// The Allen relation of `self` to `b`, both proper intervals. Exactly one
    /// holds for any pair; an open start equals an open start and an open end
    /// equals an open end.
    ///
    /// ```
    /// use tm_core::{Allen, Valid};
    ///
    /// assert_eq!(Valid::between(1, 3).relate(&Valid::between(3, 5)), Allen::Meets);
    /// assert_eq!(Valid::ALWAYS.relate(&Valid::ALWAYS), Allen::Equals);
    /// assert_eq!(Valid::from(5).relate(&Valid::between(1, 9)), Allen::OverlappedBy);
    /// ```
    pub fn relate(&self, b: &Valid) -> Allen {
        use std::cmp::Ordering::*;
        // endpoints on the extended line: an open start is −∞, an open end +∞
        let lo = |x: Option<i64>| x.map_or(i128::MIN, i128::from);
        let hi = |x: Option<i64>| x.map_or(i128::MAX, i128::from);
        let (af, at, bf, bt) = (lo(self.from), hi(self.to), lo(b.from), hi(b.to));
        if at < bf {
            return Allen::Before;
        }
        if bt < af {
            return Allen::After;
        }
        if at == bf {
            return Allen::Meets;
        }
        if bt == af {
            return Allen::MetBy;
        }
        match (af.cmp(&bf), at.cmp(&bt)) {
            (Equal, Equal) => Allen::Equals,
            (Equal, Less) => Allen::Starts,
            (Equal, Greater) => Allen::StartedBy,
            (Greater, Equal) => Allen::Finishes,
            (Less, Equal) => Allen::FinishedBy,
            (Greater, Less) => Allen::During,
            (Less, Greater) => Allen::Contains,
            (Less, Less) => Allen::Overlaps,
            (Greater, Greater) => Allen::OverlappedBy,
        }
    }
}

/// The thirteen relations of Allen's interval algebra over half-open intervals.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Allen {
    /// `a.to < b.from`.
    Before,
    /// `a.to = b.from`.
    Meets,
    /// `a.from < b.from < a.to < b.to`.
    Overlaps,
    /// `a.from = b.from ∧ a.to < b.to`.
    Starts,
    /// `b.from < a.from ∧ a.to < b.to`.
    During,
    /// `b.from < a.from ∧ a.to = b.to`.
    Finishes,
    /// Same endpoints.
    Equals,
    /// Converse of `Finishes`.
    FinishedBy,
    /// Converse of `During`.
    Contains,
    /// Converse of `Starts`.
    StartedBy,
    /// Converse of `Overlaps`.
    OverlappedBy,
    /// Converse of `Meets`.
    MetBy,
    /// Converse of `Before`.
    After,
}

impl Allen {
    /// Every relation, in declaration order.
    pub const ALL: [Allen; 13] = [
        Allen::Before,
        Allen::Meets,
        Allen::Overlaps,
        Allen::Starts,
        Allen::During,
        Allen::Finishes,
        Allen::Equals,
        Allen::FinishedBy,
        Allen::Contains,
        Allen::StartedBy,
        Allen::OverlappedBy,
        Allen::MetBy,
        Allen::After,
    ];

    /// The stable lower-case name (`"overlappedBy"`, ...).
    pub fn name(self) -> &'static str {
        match self {
            Allen::Before => "before",
            Allen::Meets => "meets",
            Allen::Overlaps => "overlaps",
            Allen::Starts => "starts",
            Allen::During => "during",
            Allen::Finishes => "finishes",
            Allen::Equals => "equals",
            Allen::FinishedBy => "finishedBy",
            Allen::Contains => "contains",
            Allen::StartedBy => "startedBy",
            Allen::OverlappedBy => "overlappedBy",
            Allen::MetBy => "metBy",
            Allen::After => "after",
        }
    }

    /// The relation with this name.
    pub fn from_name(s: &str) -> Option<Allen> {
        Allen::ALL.into_iter().find(|a| a.name() == s)
    }

    /// The converse relation: `b.relate(a)` when `a.relate(b)` is `self`.
    pub fn converse(self) -> Allen {
        match self {
            Allen::Before => Allen::After,
            Allen::Meets => Allen::MetBy,
            Allen::Overlaps => Allen::OverlappedBy,
            Allen::Starts => Allen::StartedBy,
            Allen::During => Allen::Contains,
            Allen::Finishes => Allen::FinishedBy,
            Allen::Equals => Allen::Equals,
            Allen::FinishedBy => Allen::Finishes,
            Allen::Contains => Allen::During,
            Allen::StartedBy => Allen::Starts,
            Allen::OverlappedBy => Allen::Overlaps,
            Allen::MetBy => Allen::Meets,
            Allen::After => Allen::Before,
        }
    }

    /// True for the nine relations whose intervals share an instant.
    pub fn intersecting(self) -> bool {
        matches!(
            self,
            Allen::Overlaps
                | Allen::Starts
                | Allen::During
                | Allen::Finishes
                | Allen::Equals
                | Allen::FinishedBy
                | Allen::Contains
                | Allen::StartedBy
                | Allen::OverlappedBy
        )
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

/// The bound on the constraint checks of one commit (OpenSpec change
/// `add-datalog-constraints`).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ConstraintBudget {
    /// The longest the checks may take, in milliseconds.
    pub timeout_ms: u64,
    /// The most seed rows (changed statements a check starts from).
    pub max_seed_rows: u64,
}

impl Default for ConstraintBudget {
    fn default() -> Self {
        ConstraintBudget {
            timeout_ms: 50,
            max_seed_rows: 100_000,
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
    /// The bound on this transaction's constraint checks (`None`: the database's).
    pub constraint_budget: Option<ConstraintBudget>,
}

impl Default for TxOptions {
    fn default() -> Self {
        TxOptions {
            dry_run: false,
            max_cascade: 10_000,
            constraint_budget: None,
        }
    }
}

/// One violation of a Datalog constraint found at commit.
#[derive(Clone, Debug, PartialEq)]
pub struct ConstraintViolation {
    /// The program node, rendered.
    pub program: String,
    /// The constraint's name.
    pub constraint: String,
    /// `"warn"` or `"error"`.
    pub severity: String,
    /// The body's named variables and their values.
    pub bindings: Vec<(String, Value)>,
    /// The statements its derivation used.
    pub eids: Vec<Eid>,
    /// When it holds.
    pub valid: Vec<Valid>,
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

/// The ids a transaction has allocated so far ([`crate::Tx::id_usage`]). In a dry
/// run or a speculation these are the ids that are burned: the counters advance
/// for real after the rollback, so none of them is ever issued again.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IdUsage {
    /// Every statement number taken, ascending (statements and memberships).
    pub statements: Vec<Eid>,
    /// Anonymous `NODE` ids taken.
    pub nodes: u64,
    /// `BNODE` ids taken.
    pub blank_nodes: u64,
    /// Term-dictionary ids taken by values new to the dictionary.
    pub terms: u64,
}

/// What a committed (or dry-run) transaction did.
#[derive(Clone, Debug, Default, PartialEq)]
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
    /// Every new graph membership `(e sys:inGraph g)`, in insertion order. Memberships
    /// are listed here and not in `asserted`.
    pub memberships: Vec<Eid>,
    /// Every retracted membership with its kind, listed here and not in `retracted`.
    pub memberships_retracted: Vec<(Eid, RetKind)>,
    /// The Datalog constraints evaluated at commit.
    pub constraint_checks: u64,
    /// The changed statements those checks started from.
    pub seed_rows: u64,
    /// The `warn` violations the transaction introduced (and, in a dry run, the
    /// `error` ones it would have been refused for).
    pub constraint_violations: Vec<ConstraintViolation>,
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
