//! Materialization of a stored ELPI program (`Db::materialize_elpi`, OpenSpec
//! change `add-elpi-materialization`, facade feature `elpi`): plan on a
//! snapshot outside the writer, then write the conclusions in one short
//! transaction through the commit core shared with N3 and Datalog, retract
//! what the run no longer derives, and label the transaction.

use std::collections::BTreeSet;

use tm_core::vocab::{
    ELPI_DERIVED_BY, ELPI_MATERIALIZED, ELPI_RECONCILED, ELPI_RECONCILED_COUNT, ELPI_RULE,
    ELPI_SKOLEM, SYS_DERIVED_FROM,
};
use tm_core::{Eid, ObjectId, Result, TxOptions, TxReport, Valid, Value, ViewSpec};
use tm_elpi::ElpiOptions;
use tm_elpi_store::{Plan, PlanItem};

use crate::db::Db;
use crate::materialize::{self, Fate, Labels, MItem, MSupport, MTerm};

/// Options of [`Db::materialize_elpi`].
#[derive(Debug, Clone, Default)]
pub struct MaterializeElpiOptions {
    /// Fuel, row budget and limit of the run.
    pub elpi: ElpiOptions,
    /// Report what would be written and write nothing.
    pub dry_run: bool,
}

/// What [`Db::materialize_elpi`] did.
#[derive(Debug, Clone)]
pub struct MaterializeElpiReport {
    /// The program.
    pub program: Value,
    /// Its `elpi:source` statement.
    pub program_statement: Eid,
    /// The last transaction of the snapshot the run read.
    pub reasoned_at: u64,
    /// The commit.
    pub commit: Option<TxReport>,
    /// Statements derived by this run.
    pub derived: Vec<Eid>,
    /// Conclusions already derived from the same support.
    pub existing: Vec<Eid>,
    /// `(old, new)` for conclusions whose interval widened.
    pub widened: Vec<(Eid, Eid)>,
    /// Earlier conclusions this run no longer derives, retracted.
    pub reconciled: Vec<Eid>,
    /// Conclusions skipped because a support was retracted since the snapshot.
    pub stale_support: Vec<PlanItem>,
    /// The program, when it may be non-monotonic: a later write can invalidate
    /// its conclusions before the next run.
    pub needs_rerun_after_writes: Vec<String>,
    /// True when nothing was written.
    pub dry_run: bool,
}

impl Db {
    /// Materializes stored ELPI program `program` (feature `elpi`): derives every
    /// conclusion of its `tm.conclude Fact Support` as a statement supported by
    /// the eids it states, the program's `elpi:source` statement and the sources
    /// of the programs it includes; retracts its earlier conclusions that this
    /// run no longer derives; and labels the transaction `(tx elpi:materialized
    /// program)`.
    ///
    /// # Errors
    ///
    /// `UnknownProgram`, `ProgramCycle`, the run errors of [`crate::View::elpi`],
    /// and the write errors.
    // @tg: implements:: [[openspec:elpi-materialization#Materialize an ELPI program]] {since: "2026-10-31"}
    // @tg: implements:: [[openspec:elpi-materialization#Reconcile every run]] {since: "2026-10-31"}
    // @tg: implements:: [[openspec:elpi-materialization#Conclusions fall with premises and programs]] {since: "2026-10-31"}
    // @tg: implements:: [[elpi#Reconciled Materialization]] {since: "2026-10-31"}
    pub fn materialize_elpi(
        &self,
        program: &Value,
        opts: &MaterializeElpiOptions,
    ) -> Result<MaterializeElpiReport> {
        let plan: Plan = self
            .now()
            .with_elpi_store(|s, _| tm_elpi_store::plan(s, &ViewSpec::NOW, program, &opts.elpi))?;
        let items: Vec<MItem> = plan
            .items
            .iter()
            .map(|i| MItem {
                s: MTerm::Value(i.s.clone()),
                p: Some(i.p.clone()),
                o: MTerm::Value(i.o.clone()),
                valid: i.valid,
                rule: 0,
                support: i.support.iter().map(|e| MSupport::Statement(*e)).collect(),
            })
            .collect();
        let labels = Labels {
            derived_by: ELPI_DERIVED_BY,
            rule: ELPI_RULE,
            skolem: ELPI_SKOLEM,
        };
        let source = plan.source;
        let mut done = None;
        let report = self.transact(
            TxOptions {
                dry_run: opts.dry_run,
                ..TxOptions::default()
            },
            |tx| {
                let out = materialize::commit(tx, &items, program, source, &labels)?;
                let kept: BTreeSet<Eid> = out
                    .fates
                    .iter()
                    .filter_map(|f| match f {
                        Fate::Written(e) => Some(*e),
                        _ => None,
                    })
                    .collect();
                // every run reconciles: the program's live conclusions it did
                // not derive again are retracted
                let mut reconciled = Vec::new();
                if let Some(df) = tx.lookup(&Value::iri(SYS_DERIVED_FROM))? {
                    let mut seen = BTreeSet::new();
                    for link in tx.live_rows(None, Some(df), Some(source.oid()))? {
                        let Some(d) = Eid::from_oid(ObjectId::from_raw(link.s)) else {
                            continue;
                        };
                        if kept.contains(&d) || !seen.insert(d) {
                            continue;
                        }
                        if tx.statement(d)?.is_some_and(|r| r.live()) {
                            reconciled.push(d);
                        }
                    }
                    for d in &reconciled {
                        tx.retract(*d)?;
                    }
                }
                let me = Value::Tx(tx.t());
                tx.assert(
                    me.clone(),
                    Value::iri(ELPI_MATERIALIZED),
                    program.clone(),
                    Valid::ALWAYS,
                )?;
                if !reconciled.is_empty() {
                    tx.assert(
                        me.clone(),
                        Value::iri(ELPI_RECONCILED),
                        program.clone(),
                        Valid::ALWAYS,
                    )?;
                    tx.assert(
                        me,
                        Value::iri(ELPI_RECONCILED_COUNT),
                        Value::Int(reconciled.len() as i64),
                        Valid::ALWAYS,
                    )?;
                }
                done = Some((out, reconciled));
                Ok(())
            },
        )?;
        let (out, reconciled) = done.unwrap_or_default();
        let stale_support = plan
            .items
            .iter()
            .zip(&out.fates)
            .filter(|(_, f)| matches!(f, Fate::Stale))
            .map(|(i, _)| i.clone())
            .collect();
        Ok(MaterializeElpiReport {
            program: program.clone(),
            program_statement: source,
            reasoned_at: plan.t,
            commit: Some(report),
            derived: out.derived,
            existing: out.existing,
            widened: out.widened,
            reconciled,
            stale_support,
            needs_rerun_after_writes: if plan.non_monotonic {
                vec![program.to_string()]
            } else {
                Vec::new()
            },
            dry_run: opts.dry_run,
        })
    }
}
