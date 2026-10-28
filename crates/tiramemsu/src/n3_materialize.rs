// STUB: replaced by the real implementation at merge
//! Materialization of a stored rule set (`Db::materialize`, feature `n3`).
//!
//! This file only fixes the public API so the surfaces compile; every call
//! fails with `Unsupported`.

use tm_n3::{Conclusion, Limits, Scope, Status};

use crate::{Db, Eid, Error, ObjectId, Result, TxReport, Valid, Value};

/// The options of [`Db::materialize`].
#[derive(Clone, Debug, PartialEq)]
pub struct MaterializeOptions {
    /// The statements loaded (default: the whole view).
    pub scope: Scope,
    /// The valid-time window of the sweep (default: unbounded).
    pub window: Valid,
    /// The most segments a sweep runs.
    pub max_segments: usize,
    /// eyeron's limits for each run.
    pub limits: Limits,
    /// Refuse to commit an incomplete run.
    pub require_complete: bool,
    /// Report what would be written and write nothing.
    pub dry_run: bool,
}

impl Default for MaterializeOptions {
    fn default() -> MaterializeOptions {
        MaterializeOptions {
            scope: Scope::All,
            window: Valid::ALWAYS,
            max_segments: 1024,
            limits: Limits::default(),
            require_complete: false,
            dry_run: false,
        }
    }
}

/// What [`Db::materialize`] did.
#[derive(Clone, Debug, PartialEq)]
pub struct MaterializeReport {
    /// The rule set.
    pub rule_set: Value,
    /// The rule set's live `n3:source` statement.
    pub rules_statement: Eid,
    /// The transaction the reasoning snapshot was taken at.
    pub reasoned_at: u64,
    /// The commit, `None` when nothing was written.
    pub commit: Option<TxReport>,
    /// The conclusions written.
    pub derived: Vec<Eid>,
    /// The conclusions already live with the same support.
    pub existing: Vec<Eid>,
    /// Derivations widened: `(old, new)`.
    pub widened: Vec<(Eid, Eid)>,
    /// The Skolem nodes used.
    pub skolems: Vec<ObjectId>,
    /// Conclusions skipped because a support was retracted meanwhile.
    pub stale_support: Vec<Conclusion>,
    /// Conclusions skipped because a term cannot be stored (a list or formula,
    /// a predicate that is not an IRI or is reserved, a literal subject).
    pub unstorable: Vec<Conclusion>,
    /// The completion status of the reasoning run.
    pub status: Status,
    /// True for a dry run.
    pub dry_run: bool,
}

impl Db {
    /// Materializes the stored rule set `rule_set` (stub: always `Unsupported`).
    ///
    /// # Errors
    ///
    /// `Unsupported` until the real implementation is merged.
    pub fn materialize(
        &self,
        rule_set: &Value,
        opts: &MaterializeOptions,
    ) -> Result<MaterializeReport> {
        let _ = (rule_set, opts);
        Err(Error::unsupported("materialize (stub)"))
    }
}
