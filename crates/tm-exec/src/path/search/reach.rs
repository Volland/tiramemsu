//! `REACH`: endpoints only, set semantics. A `(node, state)` pair is expanded once,
//! so cycles terminate without a hop bound; an end is emitted at its first (hence
//! minimal) hop count, each layer's new ends ordered by raw id.

use std::collections::HashSet;

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
    while !frontier.is_empty() && ctx.depth_ok(depth) {
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
