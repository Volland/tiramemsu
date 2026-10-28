//! Datalog constraints enforced at commit (OpenSpec change
//! `add-datalog-constraints`, facade feature `datalog`).
//!
//! A stored program is enforced by the statement `(program dl:enforced true)`.
//! Before every commit, the enforced programs' constraints are evaluated from
//! the transaction's added and retracted statements only
//! ([`tm_datalog::enforce::check_seeded`]); a program whose text or enforcement
//! changed in the transaction is checked in full. An `error` violation rolls
//! the transaction back; `warn` violations are listed in its report.

use std::time::Duration;

use tm_core::sqlstore::SqlStore;
use tm_core::store::{Pattern, ReadStore, Scope, StmtRow};
use tm_core::vocab::{DL_ENFORCED, DL_SOURCE};
use tm_core::{
    budget, ConstraintBudget, ConstraintViolation, Eid, Error, ObjectId, Result, TimeRef,
    TxHandle as Tx, TxSel, Valid, ValidSel, Value, ViewSpec,
};
use tm_datalog::enforce::{self, Seeds};
use tm_datalog::{DatalogOptions, Program, Severity};

/// What the checks of one commit did.
#[derive(Debug, Default)]
pub(crate) struct Notes {
    pub checks: u64,
    pub seed_rows: u64,
    pub violations: Vec<ConstraintViolation>,
}

/// Runs `f` on the transaction's own state, on either engine.
fn on_tx<R>(tx: &mut Tx<'_>, f: impl Fn(&dyn ReadStore) -> Result<R>) -> Result<R> {
    if let Some(r) = tx.read_store(&f) {
        return r;
    }
    tx.read_with(|e| f(&SqlStore::new(e)))
}

fn violation(program: &str, f: enforce::Found) -> ConstraintViolation {
    ConstraintViolation {
        program: program.to_string(),
        constraint: f.constraint,
        severity: f.severity.name().to_string(),
        bindings: f.bindings,
        eids: f.eids,
        valid: f.valid,
    }
}

/// The checks of one commit: every enforced program, seeded by the change or,
/// when its text or enforcement changed, in full.
// @tg: implements:: [[openspec:datalog-constraints#Error constraints are checked at commit]] {since: "2026-10-28"}
// @tg: implements:: [[openspec:datalog-constraints#Previews report violations]] {since: "2026-10-28"}
// @tg: implements:: [[openspec:datalog-constraints#Same verdict on every backend]] {since: "2026-10-28"}
// @tg: implements:: [[openspec:datalog-constraints#Constraint check budget]] {since: "2026-10-28"}
pub(crate) fn commit_check(
    tx: &mut Tx<'_>,
    dry_run: bool,
    limit: ConstraintBudget,
) -> Result<Notes> {
    let t = tx.t().0;
    let clock = tx.instant();
    let run = |store: &dyn ReadStore| -> Result<Notes> {
        let mut notes = Notes::default();
        let Some(enforced) = tm_store::dict::encode(store, &Value::iri(DL_ENFORCED))? else {
            return Ok(notes);
        };
        let Some(yes) = tm_store::dict::encode(store, &Value::Bool(true))? else {
            return Ok(notes);
        };
        let marks: Vec<StmtRow> = store
            .scan(
                Pattern {
                    s: None,
                    p: Some(enforced.raw()),
                    o: Some(yes.raw()),
                },
                Scope::Live,
            )?
            .collect::<Result<_>>()?;
        if marks.is_empty() {
            return Ok(notes);
        }
        let mine = |r: &StmtRow| r.t_add as u64 == t;
        let added: Vec<StmtRow> = store
            .added_since(t as i64 - 1)?
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .filter(mine)
            .collect();
        let retracted: Vec<StmtRow> = store
            .retracted_since(t as i64 - 1)?
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .filter(|r| r.t_ret.is_some_and(|x| x as u64 == t))
            .collect();
        let seeds = Seeds { added, retracted };
        let terms = tm_store::dict::TermCache::new(16);
        let post = ViewSpec::NOW;
        let pre = ViewSpec {
            tx: TxSel::AsOf(TimeRef::Tx(t.saturating_sub(1))),
            valid: ValidSel::Unfiltered,
        };
        for mark in marks {
            let node = terms.decode(store, ObjectId::from_raw(mark.s), false)?;
            let name = node.to_string();
            let Some((text, source)) = tm_datalog::stored_program(store, &post, &node)? else {
                return Err(Error::UnknownProgram { program: name });
            };
            let changed =
                mark.t_add as u64 == t || seeds.added.iter().any(|r| r.eid == source.oid().raw());
            let found = if changed {
                // new enforcement or new text: the whole program, and only
                // enforceable error constraints
                for (c, sev, why) in enforce::enforceability(&text)? {
                    if let (Severity::Error, Some(reason)) = (sev, why) {
                        return Err(Error::ConstraintNotEnforceable {
                            program: name,
                            constraint: c,
                            reason,
                        });
                    }
                }
                let r = tm_datalog::check(
                    store,
                    &post,
                    clock,
                    &Program::Text(text.clone()),
                    &[],
                    &DatalogOptions::default(),
                )?;
                notes.checks += r.checked.len() as u64;
                r.violated
                    .into_iter()
                    .flat_map(|c| {
                        let (n, s) = (c.name, c.severity);
                        c.violations.into_iter().map(move |v| enforce::Found {
                            constraint: n.clone(),
                            severity: s,
                            bindings: v
                                .bindings
                                .into_iter()
                                .map(|(k, t)| {
                                    (
                                        k,
                                        match t {
                                            tm_datalog::Term::Value(v) => v,
                                            tm_datalog::Term::Invented(i) => {
                                                Value::iri(format!("urn:tiramemsu:dl:invented:{i}"))
                                            }
                                        },
                                    )
                                })
                                .collect(),
                            eids: v.eids,
                            valid: v.valid,
                        })
                    })
                    .collect()
            } else {
                let r = enforce::check_seeded(
                    store,
                    &post,
                    &pre,
                    clock,
                    &name,
                    &text,
                    &seeds,
                    limit.max_seed_rows,
                )?;
                notes.checks += r.checked;
                notes.seed_rows += r.seed_rows;
                r.found
            };
            let (errors, warns): (Vec<_>, Vec<_>) = found
                .into_iter()
                .partition(|f| f.severity == Severity::Error);
            notes
                .violations
                .extend(warns.into_iter().map(|f| violation(&name, f)));
            if let Some(first) = errors.first() {
                let constraint = first.constraint.clone();
                let all: Vec<ConstraintViolation> =
                    errors.into_iter().map(|f| violation(&name, f)).collect();
                if !dry_run {
                    let violations = all
                        .into_iter()
                        .filter(|v| v.constraint == constraint)
                        .collect();
                    return Err(Error::ConstraintViolated {
                        program: name,
                        constraint,
                        violations,
                    });
                }
                notes.violations.extend(all);
            }
        }
        Ok(notes)
    };
    let meter = budget::Meter::new(budget::Interrupt::new(
        Some(Duration::from_millis(limit.timeout_ms)),
        None,
    ));
    let outer = budget::active();
    let r = if outer {
        on_tx(tx, run)
    } else {
        budget::scope(meter, || on_tx(tx, run))
    };
    r.map_err(|e| match e {
        Error::DeadlineExceeded { .. } if !outer => Error::ConstraintBudgetExceeded {
            program: "(enforced programs)".into(),
            constraint: "(deadline)".into(),
        },
        e => e,
    })
}

/// Turns enforcement of a stored program on or off (`TxDatalog::enforce`).
// @tg: implements:: [[openspec:datalog-constraints#Enforce a program]] {since: "2026-10-28"}
pub(crate) fn enforce(tx: &mut Tx<'_>, program: Value, on: bool) -> Result<Option<Eid>> {
    let enforced = Value::iri(DL_ENFORCED);
    let live = |tx: &mut Tx<'_>| -> Result<Vec<Eid>> {
        let (Some(s), Some(p)) = (tx.lookup(&program)?, tx.lookup(&enforced)?) else {
            return Ok(Vec::new());
        };
        Ok(tx
            .live_rows(Some(s), Some(p), None)?
            .into_iter()
            .filter_map(|r| Eid::from_oid(ObjectId::from_raw(r.eid)))
            .collect())
    };
    if !on {
        for e in live(tx)? {
            tx.retract(e)?;
        }
        return Ok(None);
    }
    let (Some(s), Some(p)) = (tx.lookup(&program)?, tx.lookup(&Value::iri(DL_SOURCE))?) else {
        return Err(Error::UnknownProgram {
            program: program.to_string(),
        });
    };
    let Some(row) = tx.live_rows(Some(s), Some(p), None)?.pop() else {
        return Err(Error::UnknownProgram {
            program: program.to_string(),
        });
    };
    let text = match tx.decode(ObjectId::from_raw(row.o))? {
        Value::Str(t) => t,
        _ => {
            return Err(Error::UnknownProgram {
                program: program.to_string(),
            })
        }
    };
    for (c, sev, why) in enforce::enforceability(&text)? {
        if let (Severity::Error, Some(reason)) = (sev, why) {
            return Err(Error::ConstraintNotEnforceable {
                program: program.to_string(),
                constraint: c,
                reason,
            });
        }
    }
    if let Some(e) = live(tx)?.first() {
        return Ok(Some(*e));
    }
    let a = tx.assert(program, enforced, Value::Bool(true), Valid::ALWAYS)?;
    Ok(Some(a.eid()))
}
