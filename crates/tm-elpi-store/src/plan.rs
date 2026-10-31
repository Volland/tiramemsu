//! Materialization plans (OpenSpec change `add-elpi-materialization`): run
//! `tm.conclude` on a snapshot with the program's own conclusions hidden, and
//! return the items the facade's shared commit writes.

use std::collections::BTreeSet;

use tm_core::store::{Pattern, ReadStore, Scope};
use tm_core::vocab::SYS_DERIVED_FROM;
use tm_core::{Eid, Result, Valid, Value, ViewSpec};
use tm_elpi::{Cost, ElpiOptions};
use tm_store::dict;

use crate::programs;

/// One conclusion to derive.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanItem {
    /// Subject.
    pub s: Value,
    /// Predicate IRI.
    pub p: String,
    /// Object.
    pub o: Value,
    /// When it holds.
    pub valid: Valid,
    /// The eids it states, plus the sources of the programs it includes.
    pub support: Vec<Eid>,
}

/// What a materialization will write.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    /// The program's own `elpi:source` statement.
    pub source: Eid,
    /// The conclusions, in a stable order.
    pub items: Vec<PlanItem>,
    /// Every predicate the program may conclude (for reconciliation).
    pub predicates: Vec<String>,
    /// True when the program may be non-monotonic.
    pub non_monotonic: bool,
    /// The last transaction of the snapshot.
    pub t: u64,
    /// What the run cost.
    pub cost: Cost,
}

/// The live statements that cite `source` as support, and those about them.
fn own(store: &dyn ReadStore, source: Eid) -> Result<Vec<i64>> {
    let Some(df) = dict::encode(store, &Value::iri(SYS_DERIVED_FROM))? else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    let mut conclusions = Vec::new();
    for l in store.scan(
        Pattern::new(None, Some(df.raw()), Some(source.oid().raw())),
        Scope::Live,
    )? {
        let l = l?;
        out.push(l.eid);
        conclusions.push(l.s);
    }
    for &d in &conclusions {
        out.push(d);
        for row in store.scan(Pattern::new(Some(d), None, None), Scope::Live)? {
            out.push(row?.eid);
        }
    }
    Ok(out)
}

/// The plan of stored program `program` over `spec`.
// @tg: implements:: [[openspec:elpi-materialization#Own conclusions are not input]] {since: "2026-10-31"}
// @tg: implements:: [[openspec:elpi-materialization#Materialize an ELPI program]] {since: "2026-10-31", confidence: partial}
pub fn plan(
    store: &dyn ReadStore,
    spec: &ViewSpec,
    program: &Value,
    opts: &ElpiOptions,
) -> Result<Plan> {
    let src = programs::resolve(store, spec, program)?;
    let source =
        src.parts
            .last()
            .and_then(|p| p.source)
            .ok_or_else(|| tm_core::Error::UnknownProgram {
                program: program.to_string(),
            })?;
    let mut opts = opts.clone();
    opts.exclude.extend(own(store, source)?);
    let included: Vec<Eid> = src.sources().into_iter().filter(|e| *e != source).collect();
    let r = tm_elpi::conclude(store, spec, &src, &opts)?;
    let mut predicates = BTreeSet::new();
    let items = r
        .conclusions
        .into_iter()
        .filter_map(|c| {
            let Value::Iri(p) = c.p else { return None };
            predicates.insert(p.clone());
            let mut support = c.support;
            support.extend(included.iter().copied());
            support.sort();
            support.dedup();
            Some(PlanItem {
                s: c.s,
                p,
                o: c.o,
                valid: c.valid,
                support,
            })
        })
        .collect();
    Ok(Plan {
        source,
        items,
        predicates: predicates.into_iter().collect(),
        non_monotonic: r.non_monotonic,
        t: tm_store::read::last_t(store)?,
        cost: r.cost,
    })
}
