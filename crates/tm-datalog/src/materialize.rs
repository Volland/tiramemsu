//! What materializing a program writes (OpenSpec change
//! `add-datalog-materialization`, `lat.md/datalog#Shared Materialization With
//! Reconciliation`): every conclusion of an IRI relation that is not a stored
//! statement, as items in derivation order, each over an interval on which one
//! derivation holds, with its support resolved to stored statements and to
//! earlier items. Plain-named relations are not stored: a premise on one is
//! replaced by that fact's own support.

use rustc_hash::FxHashMap;
use tm_core::store::ReadStore;
use tm_core::{Eid, Error, ObjectId, Result, Valid, ValidSel, ViewSpec};
use tm_store::read::View;

use crate::eval::{Engine, EvalOptions, Prem};
use crate::interval::{IvSet, NEG_INF, POS_INF};
use crate::program::{self, RuleKind};
use crate::run::{DatalogOptions, Program, Status, TimeMode};
use crate::source::{Stored, TimeView};
use crate::syntax::Pred;
use crate::terms::{Term, Terms};

/// One premise of an item.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Support {
    /// A stored statement.
    Statement(Eid),
    /// An earlier item.
    Item(usize),
}

/// One conclusion to write.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// Subject.
    pub s: Term,
    /// Predicate IRI.
    pub p: String,
    /// Object.
    pub o: Term,
    /// The interval this derivation covers.
    pub valid: Valid,
    /// The rule (index among the text's rules).
    pub rule: usize,
    /// The stratum of its relation is monotonic.
    pub monotonic: bool,
    /// Its support, without the program statement.
    pub support: Vec<Support>,
}

/// A materialization plan.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    /// Items in derivation order (every `Support::Item` points backwards).
    pub items: Vec<Item>,
    /// The IRI relations of non-monotonic strata (reconciled).
    pub reconciled_predicates: Vec<String>,
    /// A stable label per invented node, indexed by its number in the items'
    /// `Term::Invented`: the rule and the values it was invented for.
    pub invented: Vec<String>,
    /// The view's transaction.
    pub t: u64,
    /// Completion.
    pub status: Status,
}

/// Per (relation, row): the items written for it, as `(from, to, item)`.
type ItemIndex = FxHashMap<(u32, u32), Vec<(i64, i64, usize)>>;

fn ext(x: i64) -> Option<i64> {
    (x != NEG_INF && x != POS_INF).then_some(x)
}

/// Plans the materialization of `program` over `spec`, without the stored
/// statements `exclude` (the program's own earlier conclusions).
// @tg: implements:: [[openspec:datalog-materialization#Valid time of materialized conclusions]] {since: "2026-10-28"}
// @tg: implements:: [[openspec:datalog-materialization#Own conclusions are not input]] {since: "2026-10-28", confidence: partial}
pub fn plan(
    store: &dyn ReadStore,
    spec: &ViewSpec,
    clock: i64,
    program: &Program,
    opts: &DatalogOptions,
    exclude: Vec<i64>,
) -> Result<Plan> {
    let text = match program {
        Program::Text(t) => t.clone(),
        Program::Stored(v) => {
            crate::run::stored_program(store, spec, v)?
                .ok_or_else(|| Error::UnknownProgram {
                    program: v.to_string(),
                })?
                .0
        }
    };
    let prog = program::compile(&text)?;
    for s in &prog.strata {
        let name = || prog.rels[s.rels[0] as usize].name.to_string();
        if s.metric_recursion && opts.window.is_none() {
            return Err(Error::DatalogUnboundedRecursion { relation: name() });
        }
        if s.invention_recursion && opts.limits.max_new_values.is_none() {
            return Err(Error::DatalogUnboundedInvention { relation: name() });
        }
    }
    let window = opts
        .window
        .map(|w| (w.from.unwrap_or(NEG_INF), w.to.unwrap_or(POS_INF)));
    let view = View::resolve(store, spec)?;
    let time = match (opts.time, spec.valid) {
        (TimeMode::Atemporal, _) => TimeView::Atemporal,
        (_, ValidSel::At(t)) => TimeView::At(t),
        _ => TimeView::Valid,
    };
    let terms = Terms::new();
    let scope = crate::run::scope_eids(store, &view, &opts.scope)?;
    let stored = Stored::new(view, scope, exclude.into_iter().collect(), time);
    let mut eng = Engine::new(
        &prog,
        store,
        stored,
        &terms,
        EvalOptions {
            atemporal: opts.time == TimeMode::Atemporal,
            window,
            limits: opts.limits.clone(),
            proof: true,
            parallel_threshold: opts.parallel_threshold,
            now: clock,
            cheap_estimates: false,
        },
    )?;
    eng.run()?;
    let status = if eng.limits_hit.is_empty() {
        Status::Complete
    } else {
        Status::Incomplete {
            limits: eng
                .limits_hit
                .iter()
                .map(|l| l.name().to_string())
                .collect(),
        }
    };
    if let (true, Status::Incomplete { limits }) = (opts.require_complete, &status) {
        return Err(Error::DatalogIncomplete {
            limits: limits.clone(),
        });
    }
    let monotonic: FxHashMap<u32, bool> = prog
        .strata
        .iter()
        .flat_map(|s| s.rels.iter().map(move |&r| (r, s.monotonic)))
        .collect();
    let storable = |rel: u32| {
        let r = &prog.rels[rel as usize];
        !r.hidden && matches!(r.name, Pred::Iri(_))
    };
    // every (relation, row, portion) whose derivation is not a seed, cut into
    // maximal intervals outside the stored part and the window, in derivation order
    struct Piece {
        rel: u32,
        row: u32,
        d: u32,
        from: i64,
        to: i64,
    }
    let clip = |iv: &IvSet| match window {
        Some((a, b)) => iv.clip(a, b),
        None => iv.clone(),
    };
    let mut pieces: Vec<Piece> = Vec::new();
    for (ri, rel) in eng.sh.rels.iter().enumerate() {
        if !storable(ri as u32) {
            continue;
        }
        for row in 0..rel.len() as u32 {
            for (part, d) in &rel.prov[row as usize] {
                if prog.rules[eng.derivs[*d as usize].rule as usize].kind == RuleKind::Seed {
                    continue;
                }
                let own = clip(&part.subtract(&rel.stored[row as usize]));
                for &(a, b) in own.intervals() {
                    pieces.push(Piece {
                        rel: ri as u32,
                        row,
                        d: *d,
                        from: a,
                        to: b,
                    });
                }
            }
        }
    }
    pieces.sort_by_key(|p| (p.d, p.from));
    // splits each piece at its premises' portion boundaries, so one derivation
    // and one support hold over each item
    let mut items: Vec<Item> = Vec::new();
    let mut index: ItemIndex = FxHashMap::default();
    // the support covering `start` of row (rel, row), and up to where it holds
    fn cover(
        eng: &Engine<'_, '_>,
        index: &ItemIndex,
        storable: &dyn Fn(u32) -> bool,
        rel: u32,
        row: u32,
        start: i64,
        depth: usize,
    ) -> Result<Option<(i64, Vec<Support>)>> {
        if depth > 64 {
            return Ok(None);
        }
        let r = &eng.sh.rels[rel as usize];
        let Some((part, d)) = r.prov[row as usize].iter().find(|(p, _)| p.contains(start)) else {
            return Ok(None);
        };
        let pend = part
            .intervals()
            .iter()
            .find(|(a, b)| *a <= start && start < *b)
            .map_or(start, |x| x.1);
        let deriv = &eng.derivs[*d as usize];
        if deriv.rule == u32::MAX || eng.sh.prog.rules[deriv.rule as usize].kind == RuleKind::Seed {
            let e = match deriv.premises.first() {
                Some(Prem::Stmt(e)) => *e,
                _ => return Ok(None),
            };
            let eid = Eid::from_oid(ObjectId::from_raw(e))
                .ok_or_else(|| Error::storage("not a statement id"))?;
            return Ok(Some((pend, vec![Support::Statement(eid)])));
        }
        if storable(rel) {
            if let Some(&(_, b, ix)) = index
                .get(&(rel, row))
                .and_then(|v| v.iter().find(|(a, b, _)| *a <= start && start < *b))
            {
                return Ok(Some((b.min(pend), vec![Support::Item(ix)])));
            }
            return Ok(None);
        }
        // a plain-named relation: its own support
        let mut end = pend;
        let mut out = Vec::new();
        for p in &deriv.premises {
            match *p {
                Prem::Stmt(e) => {
                    out.push(Support::Statement(
                        Eid::from_oid(ObjectId::from_raw(e))
                            .ok_or_else(|| Error::storage("not a statement id"))?,
                    ));
                }
                Prem::Row(r2, row2) => {
                    match cover(eng, index, storable, r2, row2, start, depth + 1)? {
                        Some((e2, mut s2)) => {
                            end = end.min(e2);
                            out.append(&mut s2);
                        }
                        None => return Ok(None),
                    }
                }
            }
        }
        Ok(Some((end, out)))
    }
    for pc in &pieces {
        let deriv = eng.derivs[pc.d as usize].clone();
        let mut start = pc.from;
        while start < pc.to {
            let mut end = pc.to;
            let mut support: Vec<Support> = Vec::new();
            let mut ok = true;
            for p in &deriv.premises {
                match *p {
                    Prem::Stmt(e) => support.push(Support::Statement(
                        Eid::from_oid(ObjectId::from_raw(e))
                            .ok_or_else(|| Error::storage("not a statement id"))?,
                    )),
                    Prem::Row(r2, row2) => {
                        match cover(&eng, &index, &storable, r2, row2, start, 0)? {
                            Some((e2, mut s2)) => {
                                end = end.min(e2.max(start + 1));
                                support.append(&mut s2);
                            }
                            None => {
                                ok = false;
                            }
                        }
                    }
                }
            }
            if !ok {
                // a premise not covered at this instant (it came later): skip one unit
                start = end;
                continue;
            }
            support.sort();
            support.dedup();
            let r = &eng.sh.rels[pc.rel as usize];
            let tuple = r.tuple(pc.row);
            let p = match &prog.rels[pc.rel as usize].name {
                Pred::Iri(i) => i.clone(),
                Pred::Name(n) => n.clone(),
            };
            let rule = match prog.rules[deriv.rule as usize].kind {
                RuleKind::Rule(i) => i,
                _ => 0,
            };
            let ix = items.len();
            items.push(Item {
                s: terms.term(store, tuple[0])?,
                p,
                o: terms.term(store, tuple[1])?,
                valid: Valid {
                    from: ext(start),
                    to: ext(end),
                },
                rule,
                monotonic: monotonic.get(&pc.rel).copied().unwrap_or(true),
                support,
            });
            index
                .entry((pc.rel, pc.row))
                .or_default()
                .push((start, end, ix));
            start = end;
        }
    }
    let mut reconciled: Vec<String> = prog
        .strata
        .iter()
        .filter(|s| !s.monotonic)
        .flat_map(|s| s.rels.iter())
        .filter(|&&r| storable(r))
        .map(|&r| match &prog.rels[r as usize].name {
            Pred::Iri(i) => i.clone(),
            Pred::Name(n) => n.clone(),
        })
        .collect();
    reconciled.sort();
    reconciled.dedup();
    let t = match spec.tx {
        tm_core::TxSel::AsOf(tm_core::TimeRef::Tx(t)) => t,
        _ => tm_store::read::last_t(store)?,
    };
    let mut invented = Vec::new();
    for n in 0..terms.invented_count() {
        let label = match terms.invent_key(n) {
            Some((rule, vals)) => {
                // the key ends with the position of the `exists` variable, not a term
                let (pos, frontier) = vals.split_last().map_or((0, &[][..]), |(p, f)| (*p, f));
                let mut parts = Vec::new();
                for &x in frontier {
                    parts.push(terms.value(store, x)?.to_string());
                }
                format!("{rule}:{pos}:{}", parts.join(","))
            }
            None => n.to_string(),
        };
        invented.push(label);
    }
    Ok(Plan {
        items,
        invented,
        reconciled_predicates: reconciled,
        t,
        status,
    })
}
