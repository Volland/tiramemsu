//! `REACH`: endpoints only, set semantics. A `(node, state)` pair is expanded once,
//! so cycles terminate without a hop bound; an end is emitted at its first (hence
//! minimal) hop count, each layer's new ends ordered by raw id.
//!
//! Time-respecting `REACH` is label-correcting instead: a pair is expanded again
//! whenever it is reached with a strictly earlier time, and the rows are emitted
//! when the search is complete, because a longer walk can still arrive earlier.

use std::collections::{HashMap, HashSet};

use tm_core::{ObjectId, Result};

use super::{expand, Ctx, Sink};
use crate::path::row::PathRow;

pub(super) fn run(ctx: &mut Ctx<'_, '_>, sink: Sink<'_>) -> Result<()> {
    let start = ctx.start.raw();
    let q0 = ctx.dfa.start;
    let mut visited: HashSet<(i64, u32)> = HashSet::new();
    let mut emitted: HashSet<i64> = HashSet::new();
    visited.insert((start, q0));
    ctx.budget.charge(1)?;
    let origin = ctx.start;
    let row = |end: i64, hops: u32| PathRow {
        start: origin,
        end: ObjectId::from_raw(end),
        hops,
        path: None,
        arrival: None,
    };
    if ctx.dfa.accepting[q0 as usize] {
        emitted.insert(start);
        if ctx.wants(start) && !sink(row(start, 0))? {
            return Ok(());
        }
        if ctx.end_filter.is_some_and(|e| e.raw() == start) {
            return Ok(());
        }
    }
    let mut frontier = vec![(start, q0)];
    let mut depth = 0u32;
    while ctx.more(depth, frontier.iter().copied())? {
        let exp = expand(ctx, &frontier)?;
        let mut next = Vec::new();
        let mut ends: Vec<i64> = Vec::new();
        for list in &exp {
            for (nb, t) in list {
                if visited.insert((nb.to, *t)) {
                    ctx.budget.charge(1)?;
                    next.push((nb.to, *t));
                    if ctx.dfa.accepting[*t as usize] && emitted.insert(nb.to) {
                        ends.push(nb.to);
                    }
                }
            }
        }
        depth += 1;
        ends.sort_unstable();
        for e in ends {
            if !ctx.wants(e) {
                continue;
            }
            if !sink(row(e, depth))? || ctx.end_filter.is_some() {
                return Ok(());
            }
        }
        frontier = next;
    }
    Ok(())
}

/// Time-respecting `REACH` from time `tau0` (`i64::MIN` is −∞).
///
/// `best[(node, state)]` is the earliest time the pair has been reached with; a
/// pair reached again with a strictly earlier time joins the next layer again. A
/// hop allowed at some time is allowed at every earlier one and never ends later
/// (`Nb::step_time` is monotone), so once no pair improves, `best` is the minimum
/// over every time-respecting walk within the hop bound, and an end's first layer
/// is its shortest time-respecting walk. Rows are `(end, first layer, earliest
/// arrival)` ordered by hops, then raw id.
pub(super) fn run_timed(ctx: &mut Ctx<'_, '_>, sink: Sink<'_>, tau0: i64) -> Result<()> {
    let start = ctx.start.raw();
    let q0 = ctx.dfa.start;
    let mut best: HashMap<(i64, u32), i64> = HashMap::new();
    best.insert((start, q0), tau0);
    ctx.budget.charge(1)?;
    // end -> (first hop count, earliest arrival)
    let mut ends: HashMap<i64, (u32, i64)> = HashMap::new();
    if ctx.dfa.accepting[q0 as usize] {
        ends.insert(start, (0, tau0));
    }
    let mut frontier: Vec<(i64, u32, i64)> = vec![(start, q0, tau0)];
    let mut depth = 0u32;
    while ctx.more(depth, frontier.iter().map(|(n, q, _)| (*n, *q)))? {
        let entries: Vec<(i64, u32)> = frontier.iter().map(|(n, q, _)| (*n, *q)).collect();
        let exp = expand(ctx, &entries)?;
        let mut next: Vec<(i64, u32, i64)> = Vec::new();
        let mut slot: HashMap<(i64, u32), usize> = HashMap::new();
        for ((_, _, tau), list) in frontier.iter().zip(&exp) {
            for (nb, t) in list {
                let Some(at) = nb.step_time(*tau) else {
                    continue;
                };
                let key = (nb.to, *t);
                if best.get(&key).is_some_and(|b| *b <= at) {
                    continue;
                }
                ctx.budget.charge(1)?;
                best.insert(key, at);
                match slot.get(&key) {
                    Some(&i) => next[i].2 = at,
                    None => {
                        slot.insert(key, next.len());
                        next.push((nb.to, *t, at));
                    }
                }
                if ctx.dfa.accepting[*t as usize] {
                    ends.entry(nb.to)
                        .and_modify(|(_, a)| *a = (*a).min(at))
                        .or_insert((depth + 1, at));
                }
            }
        }
        depth += 1;
        frontier = next;
    }
    let mut rows: Vec<(u32, i64, i64)> = ends
        .into_iter()
        .filter(|(e, _)| ctx.wants(*e))
        .map(|(e, (hops, at))| (hops, e, at))
        .collect();
    rows.sort_unstable();
    for (hops, e, at) in rows {
        let row = PathRow {
            start: ctx.start,
            end: ObjectId::from_raw(e),
            hops,
            path: None,
            arrival: ctx.arrival(at),
        };
        if !sink(row)? {
            return Ok(());
        }
    }
    Ok(())
}
