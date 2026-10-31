//! ELPI constraints enforced at commit (OpenSpec change `add-elpi-programs`,
//! `lat.md/elpi#Two Constraint Entry Points`).
//!
//! Before every commit, each program marked `(program elpi:enforced true)`
//! runs its `tm.violation-delta` with `tm.added` and `tm.retracted` reading the
//! transaction's own changes; a program whose mark or source (or an included
//! source) changed in the transaction runs its full `tm.violation` instead.

use std::collections::{BTreeSet, HashMap};

use tm_core::store::{Pattern, ReadStore, Scope, StmtRow};
use tm_core::vocab::ELPI_ENFORCED;
use tm_core::{
    ConstraintBudget, ConstraintViolation, Eid, Error, ObjectId, Result, TimeRef, TxHandle as Tx,
    TxSel, ValidSel, Value, ViewSpec,
};
use tm_elpi::{ElpiOptions, Seeds, Severity, Violation};
use tm_store::dict::{self, TermCache};

use crate::{on_tx, programs};

/// Resolution steps granted per millisecond of the constraint budget's deadline.
pub const FUEL_PER_MS: u64 = 4_000;

/// What the checks of one commit did.
#[derive(Debug, Default)]
pub struct Notes {
    /// Programs checked.
    pub checks: u64,
    /// Rows the builtins returned.
    pub seed_rows: u64,
    /// `warn` violations, and with `dry_run` the `error` ones.
    pub violations: Vec<ConstraintViolation>,
}

fn marks(store: &dyn ReadStore) -> Result<Vec<StmtRow>> {
    let (Some(enforced), Some(yes)) = (
        dict::encode(store, &Value::iri(ELPI_ENFORCED))?,
        dict::encode(store, &Value::Bool(true))?,
    ) else {
        return Ok(Vec::new());
    };
    store
        .scan(
            Pattern::new(None, Some(enforced.raw()), Some(yes.raw())),
            Scope::Live,
        )?
        .collect()
}

fn reported(program: &str, v: &Violation) -> ConstraintViolation {
    ConstraintViolation {
        program: program.to_string(),
        constraint: v.name.clone(),
        severity: v.severity.name().to_string(),
        bindings: vec![("message".to_string(), Value::str(v.message.as_str()))],
        eids: v.eids.clone(),
        valid: vec![v.valid],
    }
}

fn budget_err(program: &str, e: Error) -> Error {
    match e {
        Error::ElpiOutOfFuel { message } => Error::ConstraintBudgetExceeded {
            program: program.to_string(),
            constraint: format!("({message})"),
        },
        e => e,
    }
}

/// The checks of one commit.
// @tg: implements:: [[openspec:elpi-constraints#Delta checks at commit]] {since: "2026-10-31"}
// @tg: implements:: [[openspec:elpi-constraints#ELPI constraint budget]] {since: "2026-10-31"}
// @tg: implements:: [[openspec:elpi-constraints#Enforce an ELPI program]] {since: "2026-10-31"}
pub fn commit_check(tx: &mut Tx<'_>, dry_run: bool, limit: ConstraintBudget) -> Result<Notes> {
    let t = tx.t().0;
    let run = |store: &dyn ReadStore| -> Result<Notes> {
        let mut notes = Notes::default();
        let marks = marks(store)?;
        if marks.is_empty() {
            return Ok(notes);
        }
        let added: Vec<StmtRow> = store
            .added_since(t as i64 - 1)?
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .filter(|r| r.t_add as u64 == t)
            .collect();
        let retracted: Vec<StmtRow> = store
            .retracted_since(t as i64 - 1)?
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .filter(|r| r.t_ret.is_some_and(|x| x as u64 == t))
            .collect();
        let new: BTreeSet<i64> = added.iter().map(|r| r.eid).collect();
        let seeds = Seeds { added, retracted };
        let opts = ElpiOptions {
            max_rows: limit.max_seed_rows,
            // lean-elpi runs a few million resolution steps a second: the
            // deadline becomes fuel, so a looping check stops near it
            fuel: limit.timeout_ms.saturating_mul(FUEL_PER_MS).max(100_000),
            ..ElpiOptions::default()
        };
        let pre = ViewSpec {
            tx: TxSel::AsOf(TimeRef::Tx(t.saturating_sub(1))),
            valid: ValidSel::Unfiltered,
        };
        let terms = TermCache::new(16);
        for mark in marks {
            let node = terms.decode(store, ObjectId::from_raw(mark.s), false)?;
            let name = node.to_string();
            let src = programs::resolve(store, &ViewSpec::NOW, &node)?;
            let changed = mark.t_add as u64 == t
                || src.sources().iter().any(|e| new.contains(&e.oid().raw()));
            notes.checks += 1;
            let found: Vec<Violation> = if changed {
                let r = tm_elpi::check(store, &ViewSpec::NOW, &src, None, &opts)
                    .map_err(|e| budget_err(&name, e))?;
                notes.seed_rows += r.cost.rows_read;
                r.violations
            } else {
                let r = tm_elpi::check_delta(store, &ViewSpec::NOW, &src, &seeds, &opts)
                    .map_err(|e| budget_err(&name, e))?;
                notes.seed_rows += r.cost.rows_read;
                // drop what the view as of the previous transaction already held
                let mut before: HashMap<String, Vec<Vec<Eid>>> = HashMap::new();
                let mut kept = Vec::new();
                for v in r.violations {
                    if v.eids.iter().any(|e| new.contains(&e.oid().raw())) {
                        kept.push(v);
                        continue;
                    }
                    if !before.contains_key(&v.name) {
                        let old = match tm_elpi::check(store, &pre, &src, Some(&v.name), &opts) {
                            Ok(c) => c.violations.into_iter().map(|x| x.eids).collect(),
                            Err(Error::ElpiOutOfFuel { .. }) => Vec::new(),
                            Err(e) => return Err(e),
                        };
                        before.insert(v.name.clone(), old);
                    }
                    if !before[&v.name].contains(&v.eids) {
                        kept.push(v);
                    }
                }
                kept
            };
            let (errors, warns): (Vec<_>, Vec<_>) = found
                .into_iter()
                .partition(|v| v.severity == Severity::Error);
            notes
                .violations
                .extend(warns.iter().map(|v| reported(&name, v)));
            if let Some(first) = errors.first() {
                let constraint = first.name.clone();
                if !dry_run {
                    let violations = errors
                        .iter()
                        .filter(|v| v.name == constraint)
                        .map(|v| reported(&name, v))
                        .collect();
                    return Err(Error::ConstraintViolated {
                        program: name,
                        constraint,
                        violations,
                    });
                }
                notes
                    .violations
                    .extend(errors.iter().map(|v| reported(&name, v)));
            }
        }
        Ok(notes)
    };
    // nothing enforced (the usual case): one dictionary lookup
    let any = on_tx(tx, |s| {
        Ok(dict::encode(s, &Value::iri(ELPI_ENFORCED))?.is_some())
    })?;
    if !any {
        return Ok(Notes::default());
    }
    on_tx(tx, run)
}
