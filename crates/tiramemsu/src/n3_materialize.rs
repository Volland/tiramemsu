//! Materialization of a stored rule set (`Db::materialize`, OpenSpec change
//! `add-n3-reasoning`, facade feature `n3`): reason on a snapshot outside the
//! writer, then write the conclusions in one short transaction through
//! `derive`, with the premises and the rule-set statement as support.

use tm_core::{Eid, Error, ObjectId, Result, TxOptions, TxReport, Valid, Value};
use tm_n3::vocab::{N3_DERIVED_BY, N3_RULE, N3_SKOLEM};
use tm_n3::{
    Conclusion, Limits, Premise, ReasonOptions, ReasonResult, Scope, Status, Term, TimeMode,
};

use crate::db::Db;
use crate::materialize::{self, Fate, Labels, MItem, MSupport, MTerm};

/// The options of [`Db::materialize`] (feature `n3`).
#[derive(Debug, Clone)]
pub struct MaterializeOptions {
    /// The statements loaded (default: the whole now view).
    pub scope: Scope,
    /// The valid-time window of the sweep (default: unbounded).
    pub window: Valid,
    /// The most segments the sweep runs (default 1 024).
    pub max_segments: usize,
    /// eyeron's limits for each segment.
    pub limits: Limits,
    /// Refuse to write the conclusions of an incomplete run (`N3Incomplete`).
    pub require_complete: bool,
    /// Run the commit with full semantics, report, then discard it.
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

/// What [`Db::materialize`] did (feature `n3`).
#[derive(Debug, Clone)]
pub struct MaterializeReport {
    /// The rule set.
    pub rule_set: Value,
    /// Its live `n3:source` statement `R` at the snapshot: a support of every
    /// conclusion written.
    pub rules_statement: Eid,
    /// The transaction number of the snapshot reasoned over (`t₀`).
    pub reasoned_at: u64,
    /// The report of the commit (of the discarded commit with `dry_run`).
    pub commit: Option<TxReport>,
    /// Conclusions written as new derived statements (not their links, labels or
    /// Skolem keys).
    pub derived: Vec<Eid>,
    /// Conclusions already live with the same content and support and an
    /// interval containing the new one.
    pub existing: Vec<Eid>,
    /// Conclusions widened to the hull of the old and the new interval, as
    /// `(old, new)`.
    pub widened: Vec<(Eid, Eid)>,
    /// The nodes invented for blank nodes of conclusions (`upsert_derived`).
    pub skolems: Vec<ObjectId>,
    /// Conclusions skipped because a support was no longer live at commit time,
    /// with every conclusion depending on one.
    pub stale_support: Vec<Conclusion>,
    /// Conclusions skipped because the store cannot hold them (a list or a
    /// formula term, a predicate that is not a writable IRI, a literal subject),
    /// with every conclusion depending on one.
    pub unstorable: Vec<Conclusion>,
    /// The status of the reasoning run.
    pub status: Status,
    /// True when the commit was a dry run.
    pub dry_run: bool,
}

/// The reasoning half of a materialization: what the commit writes.
struct Prepared {
    rule_set: Value,
    r: Eid,
    result: ReasonResult,
}

impl Db {
    /// Reasons with stored rule set `rule_set` over a snapshot of the now view
    /// and writes its conclusions as derived beliefs in one transaction
    /// (feature `n3`).
    ///
    /// The run loads the scope of `opts` (excluding the statements derived from
    /// the rule set's own live statement `R`, which it recomputes), sweeps the
    /// valid-time segments of `opts.window`, and writes every conclusion in proof
    /// order through `derive`, with support = its premise statements, the
    /// statements written for its derived premises, and `R`, plus the labels
    /// `(D n3:derivedBy rule_set)` and `(D n3:rule i)`. Retracting or correcting
    /// a premise, or editing the rule set, retracts the conclusion through the
    /// support cascade. A conclusion already live with the same content and
    /// support is `existing` when its interval contains the new one, and is
    /// superseded to the hull when they overlap or touch (`widened`). A blank
    /// node becomes the node of `upsert_derived(n3:skolem,
    /// "<R raw id>/<rule>/<label>", support)`, so re-runs reuse it. A conclusion
    /// whose support was retracted after the snapshot is skipped with its
    /// dependents and listed in `stale_support`.
    ///
    /// # Errors
    ///
    /// `UnknownRuleSet` when the rule set has no live `n3:source`;
    /// `N3NotMaterializable` for a non-monotonic, clock-dependent or
    /// untraced-premise rule set (before reasoning); `N3Contradiction` when an
    /// inference fuse fires and `N3Incomplete` for an incomplete run with
    /// `require_complete` (both before writing); the budget, read and write
    /// errors.
    ///
    /// ```
    /// # use tiramemsu::*;
    /// # let dir = tempfile::tempdir().unwrap();
    /// # let db = Db::open(dir.path().join("m.db"), OpenOptions::default())?;
    /// let v = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
    /// db.transact(TxOptions::default(), |tx| {
    ///     tx.assert(v("socrates"), Value::iri(vocab::RDF_TYPE), v("Human"), Valid::ALWAYS)?;
    ///     tx.store_rules(v("mortality"), "{ ?x a v:Human } => { ?x a v:Mortal } .")?;
    ///     Ok(())
    /// })?;
    /// let r = db.materialize(&v("mortality"), &MaterializeOptions::default())?;
    /// assert_eq!(r.derived.len(), 1);
    /// // a second run finds the conclusion and writes nothing
    /// let again = db.materialize(&v("mortality"), &MaterializeOptions::default())?;
    /// assert_eq!(again.existing, r.derived);
    /// assert!(again.commit.unwrap().asserted.is_empty());
    /// # Ok::<(), Error>(())
    /// ```
    // @tg: implements:: [[openspec:n3-materialization#Materialize a rule set]] {since: "2026-10-28"}
    pub fn materialize(
        &self,
        rule_set: &Value,
        opts: &MaterializeOptions,
    ) -> Result<MaterializeReport> {
        self.materialize_between(rule_set, opts, |_| Ok(()))
    }

    /// [`Db::materialize`] with `between` run after the snapshot is reasoned and
    /// before the commit opens: a test hook for writes that race the commit.
    #[doc(hidden)]
    pub fn materialize_between(
        &self,
        rule_set: &Value,
        opts: &MaterializeOptions,
        between: impl FnOnce(&Db) -> Result<()>,
    ) -> Result<MaterializeReport> {
        let prepared = self.prepare_materialize(rule_set, opts)?;
        between(self)?;
        self.commit_materialize(prepared, opts)
    }

    /// Reasons on a snapshot outside the writer (design N13) and applies the
    /// refusals that come before writing.
    // @tg: implements:: [[openspec:n3-materialization#Snapshot then commit]] {since: "2026-10-28"}
    fn prepare_materialize(&self, rule_set: &Value, opts: &MaterializeOptions) -> Result<Prepared> {
        let ropts = ReasonOptions {
            scope: opts.scope.clone(),
            window: opts.window,
            time: TimeMode::Pointwise,
            max_segments: opts.max_segments,
            proof_n3: false,
            limits: opts.limits,
        };
        let result = self.now().reason_for_materialization(rule_set, &ropts)?;
        if let Some(c) = &result.contradiction {
            return Err(Error::N3Contradiction { rule: c.rule });
        }
        if let (true, Status::Incomplete { limits }) = (opts.require_complete, &result.status) {
            return Err(Error::N3Incomplete {
                limits: limits.iter().map(|l| format!("{l:?}")).collect(),
                errors: result.errors.clone(),
            });
        }
        let r = result
            .rule_set_statement
            .ok_or_else(|| Error::UnknownRuleSet {
                rule_set: rule_set.to_string(),
            })?;
        Ok(Prepared {
            rule_set: rule_set.clone(),
            r,
            result,
        })
    }

    /// Writes the prepared conclusions in one transaction, in proof order.
    // @tg: implements:: [[openspec:n3-materialization#Valid time of materialized conclusions]] {since: "2026-10-28"}
    // @tg: implements:: [[openspec:n3-materialization#Conclusions fall with premises and rules]] {since: "2026-10-28"}
    fn commit_materialize(
        &self,
        p: Prepared,
        opts: &MaterializeOptions,
    ) -> Result<MaterializeReport> {
        let mut out = MaterializeReport {
            rule_set: p.rule_set.clone(),
            rules_statement: p.r,
            reasoned_at: p.result.t,
            commit: None,
            derived: Vec::new(),
            existing: Vec::new(),
            widened: Vec::new(),
            skolems: Vec::new(),
            stale_support: Vec::new(),
            unstorable: Vec::new(),
            status: p.result.status.clone(),
            dry_run: opts.dry_run,
        };
        let tx_opts = TxOptions {
            dry_run: opts.dry_run,
            ..TxOptions::default()
        };
        let items: Vec<MItem> = p.result.conclusions.iter().map(item).collect();
        let labels = Labels {
            derived_by: N3_DERIVED_BY,
            rule: N3_RULE,
            skolem: N3_SKOLEM,
        };
        let mut outcome = None;
        let report = self.transact(tx_opts, |tx| {
            outcome = Some(materialize::commit(tx, &items, &p.rule_set, p.r, &labels)?);
            Ok(())
        })?;
        let o = outcome.unwrap_or_default();
        out.derived = o.derived;
        out.existing = o.existing;
        out.widened = o.widened;
        out.skolems = o.skolems;
        for (c, fate) in p.result.conclusions.iter().zip(&o.fates) {
            match fate {
                Fate::Stale => out.stale_support.push(c.clone()),
                Fate::Unstorable => out.unstorable.push(c.clone()),
                Fate::Written(_) => {}
            }
        }
        out.commit = Some(report);
        Ok(out)
    }
}

/// A conclusion as an item of the shared commit.
fn item(c: &Conclusion) -> MItem {
    let term = |t: &Term| match t {
        Term::Value(v) => MTerm::Value(v.clone()),
        Term::Blank(label) => MTerm::Invent(label.clone()),
        Term::List(_) | Term::Formula(_) => MTerm::Unstorable,
    };
    MItem {
        s: term(&c.s),
        p: match &c.p {
            Term::Value(Value::Iri(p)) => Some(p.clone()),
            _ => None,
        },
        o: term(&c.o),
        valid: c.valid,
        rule: c.proof.rule,
        support: c
            .proof
            .premises
            .iter()
            .map(|p| match *p {
                Premise::Statement(e) => MSupport::Statement(e),
                Premise::Conclusion(j) => MSupport::Item(j),
            })
            .collect(),
    }
}
