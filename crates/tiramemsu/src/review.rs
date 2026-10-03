//! Memory conflict review (OpenSpec change `add-memory-conflict-review`): conflict
//! inspection on a view and noncommitting previews of a bundle import.
//!
//! Both are read-side tools. Neither resolves anything: a conflict lists the
//! attributed evidence of every disagreeing value and leaves the choice to the
//! caller, and a preview is not a reservation. Applying a previewed bundle is an
//! ordinary write ([`Tx::import_bundle`](tm_core::Tx::import_bundle) in
//! [`Db::transact`]) that revalidates against the database as it is then.

use tm_core::{
    conflict, Bundle, Conflict, ConflictQuery, Error, IdUsage, ImportReport, Result, TermReader,
    TxId, TxOptions, TxReport,
};

use crate::db::Db;
use crate::view::View;

/// The scope a [`BundlePreview`] was computed in.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PreviewScope {
    /// The last committed transaction the preview read: the preview holds for
    /// this state only, and nothing is reserved for a later application.
    pub basis: TxId,
    /// The transaction number the import would have had (not consumed).
    pub t: TxId,
    /// The instant the import would have had (epoch ms).
    pub instant: i64,
}

/// The result of [`Db::preview_bundle`]: what importing a bundle now would do,
/// computed with the full write semantics and discarded.
///
/// When the import would fail on schema or write semantics, `failure` holds the
/// error and `import` and `report` are `None`. Either way the graph, the
/// transaction log and the event log are unchanged; only the id counters advance
/// over the ids the preview allocated (`burned`), as for any dry run.
#[derive(Debug)]
pub struct BundlePreview {
    /// Where every bundle statement would land: `new: true` is a proposed
    /// assertion, `new: false` reuses a live statement. `None` on failure.
    pub import: Option<ImportReport>,
    /// The full dry-run report: proposed `asserted` and reused `existing`
    /// statements, and the dependency changes (`retracted` by cardinality-one
    /// replacement, `memberships`). `None` on failure.
    pub report: Option<TxReport>,
    /// The schema or write-semantics failure the import would raise
    /// (`UniqueViolation`, `ValueTypeMismatch`, `SubjectTypeMismatch`,
    /// `ReservedNamespace`, `InvalidInterval`, `InvalidGraphName`, `SelfReference`).
    pub failure: Option<Error>,
    /// The ids allocated by the preview, which are burned: never issued again.
    pub burned: IdUsage,
    /// The state the preview was computed in.
    pub scope: PreviewScope,
}

impl BundlePreview {
    /// True when the import would commit if the database did not change first.
    pub fn would_commit(&self) -> bool {
        self.failure.is_none()
    }
}

/// True for the errors a preview reports as its outcome rather than raising: the
/// write semantics refusing the bundle's statements.
fn is_write_failure(e: &Error) -> bool {
    matches!(
        e,
        Error::UniqueViolation { .. }
            | Error::ValueTypeMismatch { .. }
            | Error::SubjectTypeMismatch { .. }
            | Error::ReservedNamespace(_)
            | Error::InvalidInterval { .. }
            | Error::InvalidGraphName { .. }
            | Error::SelfReference(_)
    )
}

impl Db {
    /// Previews importing `bundle` now: runs [`Tx::import_bundle`] with its full
    /// validation and write semantics as a dry run and discards every effect.
    /// The preview lists proposed and reused statements, the dependency changes,
    /// the schema failure if there is one, the burned ids and the scope it was
    /// computed in.
    ///
    /// A preview is not a reservation. Applying the bundle later is an explicit
    /// write (`tx.import_bundle` in [`Db::transact`]) that revalidates against
    /// the database as it is then: it either commits atomically or fails with
    /// the schema error, whatever the preview said. A bundle is interchange, not
    /// replica-log synchronization.
    ///
    /// # Errors
    ///
    /// `InvalidTerm` for a malformed bundle (before the writer is taken),
    /// `Unsupported` for a reference cycle; `ImportInProgress`, `Reentrant`,
    /// budget errors and `Sqlite` as for [`Db::transact`]. Schema failures are
    /// not errors: they are the preview's [`BundlePreview::failure`].
    ///
    /// ```
    /// # use tiramemsu::*;
    /// # let dir = tempfile::tempdir().unwrap();
    /// let a = Db::open(dir.path().join("a.db"), OpenOptions::default())?;
    /// let b = Db::open(dir.path().join("b.db"), OpenOptions::default())?;
    /// let v = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
    /// let r = a.transact(TxOptions::default(), |tx| {
    ///     let job = tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?.eid();
    ///     tx.assert(job, v("confidence"), Value::Double(0.8), Valid::ALWAYS)?;
    ///     Ok(())
    /// })?;
    /// let bundle = a.now().bundle(r.asserted[0])?;
    ///
    /// let preview = b.preview_bundle(&bundle)?;
    /// assert!(preview.would_commit());
    /// assert!(preview.import.unwrap().statements.iter().all(|s| s.new));
    /// assert!(b.events_since(0)?.is_empty()); // nothing was written
    ///
    /// // applying is an explicit write that validates again
    /// let applied = b.transact(TxOptions::default(), |tx| tx.import_bundle(&bundle).map(|_| ()))?;
    /// assert_eq!(applied.asserted.len(), 2);
    /// # Ok::<(), Error>(())
    /// ```
    ///
    /// [`Tx::import_bundle`]: tm_core::Tx::import_bundle
    // @lat: [[data-model#Fact Bundles#Import Preview]]
    pub fn preview_bundle(&self, bundle: &Bundle) -> Result<BundlePreview> {
        // a malformed bundle is refused before the writer is taken
        bundle.check()?;
        let mut outcome: Option<(Result<ImportReport>, IdUsage, TxId, i64)> = None;
        let report = self.transact(
            TxOptions {
                dry_run: true,
                ..TxOptions::default()
            },
            |tx| {
                let (t, instant) = (tx.t(), tx.instant());
                let r = tx.import_bundle(bundle);
                if let Err(e) = &r {
                    if !is_write_failure(e) {
                        // a cycle, a budget stop, a host error: the preview fails
                        return r.map(|_| ());
                    }
                }
                // a reported failure is discarded like a success; its partial
                // report is dropped below
                outcome = Some((r, tx.id_usage(), t, instant));
                Ok(())
            },
        )?;
        let (r, burned, t, instant) = outcome.expect("the dry run ran its body");
        let scope = PreviewScope {
            basis: TxId(t.0 - 1),
            t,
            instant,
        };
        Ok(match r {
            Ok(import) => BundlePreview {
                import: Some(import),
                report: Some(report),
                failure: None,
                burned,
                scope,
            },
            Err(e) => BundlePreview {
                import: None,
                report: None,
                failure: Some(e),
                burned,
                scope,
            },
        })
    }
}

// Conflict inspection, kept in its own block.
impl View<'_> {
    /// The conflicts of this view: subject/predicate pairs with distinct objects
    /// whose half-open valid intervals overlap, each with the windows in which
    /// they overlap and the attributed evidence of every statement involved
    /// (statement ids, valid time, asserting transaction, confidence when
    /// present, confirmations, authors and sources). Parallel statements with
    /// the same object are support for that value, not a conflict; disjoint
    /// valid times are not reported. Nothing is scored or chosen, and a
    /// multi-valued predicate is never called a schema violation
    /// ([`Conflict::declared_many`] says when it is declared so).
    ///
    /// Read-only: it never retracts, supersedes or confirms anything, and runs in
    /// one read snapshot as one budgeted operation. `q` filters by subject and
    /// predicate and limits the count; see [`ConflictQuery`].
    ///
    /// # Errors
    ///
    /// `Unsupported` on the history view (use an as-of view: history holds
    /// statements that were never believed together), the budget errors, and
    /// `Sqlite`.
    ///
    /// ```
    /// # use tiramemsu::*;
    /// # let dir = tempfile::tempdir().unwrap();
    /// # let db = Db::open(dir.path().join("m.db"), OpenOptions::default())?;
    /// let v = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
    /// db.transact(TxOptions::default(), |tx| {
    ///     tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::from(0))?;
    ///     tx.assert(v("alice"), v("worksAt"), v("initech"), Valid::between(10, 20))?;
    ///     tx.assert(v("bob"), v("worksAt"), v("acme"), Valid::between(0, 10))?;
    ///     tx.assert(v("bob"), v("worksAt"), v("initech"), Valid::from(10))?; // a later job
    ///     Ok(())
    /// })?;
    /// let conflicts = db.now().conflicts(&ConflictQuery::default())?;
    /// assert_eq!(conflicts.len(), 1); // alice only
    /// let c = &conflicts[0];
    /// assert_eq!(db.now().decode(c.s)?, v("alice"));
    /// assert_eq!(c.overlaps, vec![Valid::between(10, 20)]);
    /// assert_eq!(c.values.len(), 2);
    /// assert_eq!(c.values[0].statements[0].confidence, None); // absent, not invented
    /// # Ok::<(), Error>(())
    /// ```
    // @lat: [[query#Conflict Inspection]]
    pub fn conflicts(&self, q: &ConflictQuery) -> Result<Vec<Conflict>> {
        let spec = self.spec();
        self.op(|| {
            self.exec(|e, cache| match cache {
                Some(r) => conflict::inspect(e, &spec, q, r, true),
                None => conflict::inspect(e, &spec, q, &TermReader::new(64), false),
            })
        })
    }
}
