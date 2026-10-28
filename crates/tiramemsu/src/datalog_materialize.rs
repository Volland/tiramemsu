//! Materialization of a stored Datalog program (`Db::materialize_program`,
//! OpenSpec change `add-datalog-materialization`, facade feature `datalog`):
//! evaluate on a snapshot outside the writer, then write the conclusions in one
//! short transaction through the commit core shared with N3, retract what a
//! non-monotonic stratum no longer derives, and label the transaction.

use std::collections::BTreeSet;

use tm_core::store::{Pattern, ReadStore, Scope as StoreScope};
use tm_core::vocab::{
    DL_DERIVED_BY, DL_MATERIALIZED, DL_RECONCILED, DL_RECONCILED_COUNT, DL_RULE, DL_SKOLEM,
    SYS_DERIVED_FROM,
};
use tm_core::{Eid, Error, ObjectId, Result, TxOptions, TxReport, Valid, Value};
use tm_datalog::materialize::{self as plan_mod, Item, Support};
use tm_datalog::{
    Conclusion as DatalogConclusion, DatalogOptions, Limits as DatalogLimits,
    Program as DatalogProgram, Scope as DatalogScope, Status as DatalogStatus, Term as DatalogTerm,
};

use crate::db::Db;
use crate::materialize::{self, Fate, Labels, MItem, MSupport, MTerm};

/// The options of [`Db::materialize_program`] (feature `datalog`).
#[derive(Debug, Clone)]
pub struct MaterializeProgramOptions {
    /// The statements read (default: the whole now view).
    pub scope: DatalogScope,
    /// The valid-time window conclusions are clipped to (default: unbounded).
    pub window: Option<Valid>,
    /// The engine's limits.
    pub limits: DatalogLimits,
    /// Refuse to write the conclusions of an incomplete run (`DatalogIncomplete`).
    pub require_complete: bool,
    /// Run the commit with full semantics, report, then discard it.
    pub dry_run: bool,
}

impl Default for MaterializeProgramOptions {
    fn default() -> Self {
        MaterializeProgramOptions {
            scope: DatalogScope::All,
            window: None,
            limits: DatalogLimits::default(),
            require_complete: false,
            dry_run: false,
        }
    }
}

/// What [`Db::materialize_program`] did (feature `datalog`).
#[derive(Debug, Clone)]
pub struct MaterializeProgramReport {
    /// The program.
    pub program: Value,
    /// Its live `dl:source` statement `R` at the snapshot: a support of every
    /// conclusion written.
    pub program_statement: Eid,
    /// The transaction number of the snapshot evaluated (`t₀`).
    pub reasoned_at: u64,
    /// The report of the commit (of the discarded commit with `dry_run`).
    pub commit: Option<TxReport>,
    /// Conclusions written as new derived statements.
    pub derived: Vec<Eid>,
    /// Conclusions already live with the same content and support and an
    /// interval containing the new one.
    pub existing: Vec<Eid>,
    /// Conclusions widened to the hull of the old and the new interval, as
    /// `(old, new)`.
    pub widened: Vec<(Eid, Eid)>,
    /// Earlier conclusions of non-monotonic strata the run no longer derived,
    /// retracted.
    pub reconciled: Vec<Eid>,
    /// The nodes invented by `exists` heads (`upsert_derived`).
    pub skolems: Vec<ObjectId>,
    /// Conclusions skipped because a support was no longer live at commit time,
    /// with everything resting on them.
    pub stale_support: Vec<DatalogConclusion>,
    /// Conclusions skipped because the store cannot hold them (a literal
    /// subject, a predicate that is not a writable IRI), with their dependents.
    pub unstorable: Vec<DatalogConclusion>,
    /// The predicates of non-monotonic strata: a later write can invalidate
    /// their conclusions before the next run.
    pub needs_rerun_after_writes: Vec<String>,
    /// The status of the evaluation.
    pub status: DatalogStatus,
    /// True when the commit was a dry run.
    pub dry_run: bool,
}

struct Prepared {
    program: Value,
    r: Eid,
    plan: plan_mod::Plan,
}

fn conclusion(i: &Item) -> DatalogConclusion {
    DatalogConclusion {
        s: i.s.clone(),
        p: Value::iri(i.p.as_str()),
        o: i.o.clone(),
        valid: i.valid,
        proof: None,
    }
}

/// The live conclusions of `r` (statements with a live support link to it) and
/// every statement about them (links and labels), as eids.
fn own(store: &dyn ReadStore, r: Eid) -> Result<(Vec<i64>, Vec<i64>)> {
    let Some(df) = tm_store::dict::encode(store, &Value::iri(SYS_DERIVED_FROM))? else {
        return Ok((Vec::new(), Vec::new()));
    };
    let mut conclusions = Vec::new();
    for l in store.scan(
        Pattern {
            s: None,
            p: Some(df.raw()),
            o: Some(r.oid().raw()),
        },
        StoreScope::Live,
    )? {
        conclusions.push(l?.s);
    }
    let mut about = Vec::new();
    for &d in &conclusions {
        for row in store.scan(
            Pattern {
                s: Some(d),
                p: None,
                o: None,
            },
            StoreScope::Live,
        )? {
            about.push(row?.eid);
        }
    }
    Ok((conclusions, about))
}

impl Db {
    /// Evaluates stored Datalog program `program` over a snapshot of the now view
    /// and writes its conclusions as derived beliefs in one transaction (feature
    /// `datalog`).
    ///
    /// Every conclusion of an IRI relation that is not a stored statement is
    /// written through `derive`, one statement per maximal interval on which one
    /// derivation holds, with support = its premise statements, the statements
    /// written for its derived premises and the program statement `R`, plus the
    /// labels `(D dl:derivedBy program)` and `(D dl:rule i)`. Retracting or
    /// correcting a premise, or editing the program, retracts the conclusion
    /// through the support cascade. Strata that use negation or aggregates are
    /// reconciled: conclusions of the program they no longer derive are
    /// retracted, the transaction is labelled `(tx dl:reconciled program)` with a
    /// count, and the report lists their predicates in `needs_rerun_after_writes`
    /// (a later write can invalidate them before the next run). The transaction
    /// carries `(tx dl:materialized program)`. The run does not read the
    /// program's own earlier conclusions.
    ///
    /// # Errors
    ///
    /// `UnknownProgram`, the analysis errors, `DatalogIncomplete` with
    /// `require_complete`, and the budget, read and write errors.
    ///
    /// ```
    /// # use tiramemsu::*;
    /// # let dir = tempfile::tempdir().unwrap();
    /// # let db = Db::open(dir.path().join("m.db"), OpenOptions::default())?;
    /// let v = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
    /// db.transact(TxOptions::default(), |tx| {
    ///     tx.assert(v("alice"), v("manager"), v("bob"), Valid::ALWAYS)?;
    ///     tx.assert(v("bob"), v("manager"), v("carol"), Valid::ALWAYS)?;
    ///     tx.store_program(v("reporting"),
    ///         "v:reportsTo(X, Y) :- v:manager(X, Y).\n\
    ///          v:reportsTo(X, Z) :- v:reportsTo(X, Y), v:manager(Y, Z).")?;
    ///     Ok(())
    /// })?;
    /// let r = db.materialize_program(&v("reporting"), &MaterializeProgramOptions::default())?;
    /// assert_eq!(r.derived.len(), 3);
    /// # Ok::<(), Error>(())
    /// ```
    // @tg: implements:: [[openspec:datalog-materialization#Materialize a program]] {since: "2026-10-28"}
    pub fn materialize_program(
        &self,
        program: &Value,
        opts: &MaterializeProgramOptions,
    ) -> Result<MaterializeProgramReport> {
        self.materialize_program_between(program, opts, |_| Ok(()))
    }

    /// [`Db::materialize_program`] with `between` run after the snapshot is
    /// evaluated and before the commit opens: a test hook for writes that race
    /// the commit.
    #[doc(hidden)]
    pub fn materialize_program_between(
        &self,
        program: &Value,
        opts: &MaterializeProgramOptions,
        between: impl FnOnce(&Db) -> Result<()>,
    ) -> Result<MaterializeProgramReport> {
        let prepared = self.prepare_program(program, opts)?;
        between(self)?;
        self.commit_program(prepared, opts)
    }

    // @tg: implements:: [[openspec:datalog-materialization#Own conclusions are not input]] {since: "2026-10-28"}
    fn prepare_program(
        &self,
        program: &Value,
        opts: &MaterializeProgramOptions,
    ) -> Result<Prepared> {
        let dl = DatalogOptions {
            scope: opts.scope.clone(),
            window: opts.window,
            limits: opts.limits.clone(),
            require_complete: opts.require_complete,
            ..DatalogOptions::default()
        };
        let spec = tm_core::ViewSpec::NOW;
        let (plan, r) = self.now().with_store_pub(|store, clock| {
            let Some((_, r)) = tm_datalog::stored_program(store, &spec, program)? else {
                return Err(Error::UnknownProgram {
                    program: program.to_string(),
                });
            };
            let (conclusions, about) = own(store, r)?;
            let mut exclude = conclusions;
            exclude.extend(about);
            let plan = plan_mod::plan(
                store,
                &spec,
                clock,
                &DatalogProgram::Stored(program.clone()),
                &dl,
                exclude,
            )?;
            Ok((plan, r))
        })?;
        Ok(Prepared {
            program: program.clone(),
            r,
            plan,
        })
    }

    // @tg: implements:: [[openspec:datalog-materialization#Reconcile non-monotonic strata]] {since: "2026-10-28"}
    // @tg: implements:: [[openspec:datalog-materialization#Conclusions fall with premises and programs]] {since: "2026-10-28"}
    fn commit_program(
        &self,
        p: Prepared,
        opts: &MaterializeProgramOptions,
    ) -> Result<MaterializeProgramReport> {
        let term = |t: &DatalogTerm, plan: &plan_mod::Plan| match t {
            DatalogTerm::Value(v) => MTerm::Value(v.clone()),
            DatalogTerm::Invented(n) => MTerm::Invent(
                plan.invented
                    .get(*n as usize)
                    .cloned()
                    .unwrap_or_else(|| n.to_string()),
            ),
        };
        let items: Vec<MItem> = p
            .plan
            .items
            .iter()
            .map(|i| MItem {
                s: term(&i.s, &p.plan),
                p: Some(i.p.clone()),
                o: term(&i.o, &p.plan),
                valid: i.valid,
                rule: i.rule,
                support: i
                    .support
                    .iter()
                    .map(|s| match s {
                        Support::Statement(e) => MSupport::Statement(*e),
                        Support::Item(j) => MSupport::Item(*j),
                    })
                    .collect(),
            })
            .collect();
        let labels = Labels {
            derived_by: DL_DERIVED_BY,
            rule: DL_RULE,
            skolem: DL_SKOLEM,
        };
        let mut done = None;
        let reconcile: Vec<String> = p.plan.reconciled_predicates.clone();
        let report = self.transact(
            TxOptions {
                dry_run: opts.dry_run,
                ..TxOptions::default()
            },
            |tx| {
                let out = materialize::commit(tx, &items, &p.program, p.r, &labels)?;
                let kept: BTreeSet<Eid> = out
                    .fates
                    .iter()
                    .filter_map(|f| match f {
                        Fate::Written(e) => Some(*e),
                        _ => None,
                    })
                    .collect();
                let mut reconciled = Vec::new();
                if !reconcile.is_empty() {
                    if let Some(df) = tx.lookup(&Value::iri(SYS_DERIVED_FROM))? {
                        let mut preds = Vec::new();
                        for name in &reconcile {
                            if let Some(id) = tx.lookup(&Value::iri(name.as_str()))? {
                                preds.push(id.raw());
                            }
                        }
                        for link in tx.live_rows(None, Some(df), Some(p.r.oid()))? {
                            let Some(d) = Eid::from_oid(ObjectId::from_raw(link.s)) else {
                                continue;
                            };
                            if kept.contains(&d) {
                                continue;
                            }
                            let Some(row) = tx.statement(d)? else {
                                continue;
                            };
                            if row.live() && preds.contains(&row.p) {
                                reconciled.push(d);
                            }
                        }
                        for d in &reconciled {
                            tx.retract(*d)?;
                        }
                    }
                }
                let me = Value::Tx(tx.t());
                tx.assert(
                    me.clone(),
                    Value::iri(DL_MATERIALIZED),
                    p.program.clone(),
                    Valid::ALWAYS,
                )?;
                if !reconciled.is_empty() {
                    tx.assert(
                        me.clone(),
                        Value::iri(DL_RECONCILED),
                        p.program.clone(),
                        Valid::ALWAYS,
                    )?;
                    tx.assert(
                        me,
                        Value::iri(DL_RECONCILED_COUNT),
                        Value::Int(reconciled.len() as i64),
                        Valid::ALWAYS,
                    )?;
                }
                done = Some((out, reconciled));
                Ok(())
            },
        )?;
        let (out, reconciled) = done.unwrap_or_default();
        let mut stale = Vec::new();
        let mut unstorable = Vec::new();
        for (it, fate) in p.plan.items.iter().zip(&out.fates) {
            match fate {
                Fate::Stale => stale.push(conclusion(it)),
                Fate::Unstorable => unstorable.push(conclusion(it)),
                Fate::Written(_) => {}
            }
        }
        Ok(MaterializeProgramReport {
            program: p.program,
            program_statement: p.r,
            reasoned_at: p.plan.t,
            commit: Some(report),
            derived: out.derived,
            existing: out.existing,
            widened: out.widened,
            reconciled,
            skolems: out.skolems,
            stale_support: stale,
            unstorable,
            needs_rerun_after_writes: p.plan.reconciled_predicates,
            status: p.plan.status,
            dry_run: opts.dry_run,
        })
    }
}
