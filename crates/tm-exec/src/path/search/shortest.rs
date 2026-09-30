//! `ANY_SHORTEST` and `ALL_SHORTEST` (after Martens et al., "Evaluating regular path
//! queries under the all-shortest paths semantics"): a BFS records each
//! `(node, state)`'s first-discovery depth and its predecessors. Any-shortest keeps
//! the first predecessor, which, with the frontier in canonical order and neighbours
//! in hop-key order, is the lexicographically smallest shortest path. All-shortest
//! keeps every same-depth predecessor and enumerates the layered DAG backwards from
//! each layer's targets, so every minimal path appears once.
//!
//! The time-respecting variant (`run_timed`) keys a layer's entries by
//! `(node, state, time)` and keeps an entry only when its time is strictly earlier
//! than the pair's best time in every earlier layer (Pareto pruning per layer).

use std::collections::{HashMap, HashSet};

use tm_core::{ObjectId, Result};

use super::{expand, hop_of, row_of, Ctx, Sink};
use crate::path::row::Hop;

struct SNode {
    node: i64,
    state: u32,
    depth: u32,
    /// The search time of a time-respecting entry (`i64::MIN` otherwise).
    tau: i64,
    preds: Vec<(u32, Hop)>,
}

/// Every path from the root to `at`, as `(hop, node reached)` steps.
fn paths_to(arena: &[SNode], at: u32) -> Vec<Vec<(Hop, i64)>> {
    let n = &arena[at as usize];
    if n.preds.is_empty() {
        return vec![Vec::new()];
    }
    let mut out = Vec::new();
    for (p, h) in &n.preds {
        for mut prefix in paths_to(arena, *p) {
            prefix.push((*h, n.node));
            out.push(prefix);
        }
    }
    out
}

fn key_of(steps: &[(Hop, i64)]) -> Vec<(i64, u8, crate::path::row::Dir)> {
    steps
        .iter()
        .map(|(h, _)| (h.eid.raw(), h.kind as u8, h.dir))
        .collect()
}

pub(super) fn run(ctx: &mut Ctx<'_, '_>, sink: Sink<'_>, all: bool) -> Result<()> {
    let start = ctx.start;
    let q0 = ctx.dfa.start;
    let mut arena = vec![SNode {
        node: start.raw(),
        state: q0,
        depth: 0,
        tau: i64::MIN,
        preds: Vec::new(),
    }];
    let mut index: HashMap<(i64, u32), u32> = HashMap::new();
    index.insert((start.raw(), q0), 0);
    ctx.budget.charge(1)?;
    let mut emitted: HashSet<i64> = HashSet::new();
    if ctx.dfa.accepting[q0 as usize] {
        emitted.insert(start.raw());
        if ctx.wants(start.raw()) && !sink(row_of(start, &[], true, None))? {
            return Ok(());
        }
        if ctx.end_filter.is_some_and(|e| e.raw() == start.raw()) {
            return Ok(());
        }
    }
    let mut layer: Vec<u32> = vec![0];
    let mut depth = 0u32;
    while !layer.is_empty() && ctx.depth_ok(depth) {
        let entries: Vec<(i64, u32)> = layer
            .iter()
            .map(|i| (arena[*i as usize].node, arena[*i as usize].state))
            .collect();
        let exp = expand(ctx, &entries)?;
        let mut next: Vec<u32> = Vec::new();
        for (k, list) in exp.iter().enumerate() {
            let parent = layer[k];
            for (nb, t) in list {
                let h = hop_of(nb);
                match index.get(&(nb.to, *t)) {
                    None => {
                        ctx.budget.charge(1)?;
                        let idx = arena.len() as u32;
                        arena.push(SNode {
                            node: nb.to,
                            state: *t,
                            depth: depth + 1,
                            tau: i64::MIN,
                            preds: vec![(parent, h)],
                        });
                        index.insert((nb.to, *t), idx);
                        next.push(idx);
                    }
                    Some(&idx) if all && arena[idx as usize].depth == depth + 1 => {
                        ctx.budget.charge(1)?;
                        arena[idx as usize].preds.push((parent, h));
                    }
                    Some(_) => {}
                }
            }
        }
        depth += 1;
        let targets: Vec<u32> = next
            .iter()
            .copied()
            .filter(|i| {
                let n = &arena[*i as usize];
                ctx.dfa.accepting[n.state as usize] && !emitted.contains(&n.node)
            })
            .collect();
        let mut rows: Vec<Vec<(Hop, i64)>> = Vec::new();
        if all {
            for i in &targets {
                if !ctx.wants(arena[*i as usize].node) {
                    continue;
                }
                for p in paths_to(&arena, *i) {
                    ctx.budget.charge(1)?;
                    rows.push(p);
                }
            }
            rows.sort_by_key(|r| key_of(r));
        } else {
            let mut seen: HashSet<i64> = HashSet::new();
            for i in &targets {
                let n = &arena[*i as usize];
                if ctx.wants(n.node) && seen.insert(n.node) {
                    rows.push(paths_to(&arena, *i).swap_remove(0));
                }
            }
        }
        for i in &targets {
            emitted.insert(arena[*i as usize].node);
        }
        for steps in &rows {
            if !sink(row_of(ObjectId::from_raw(start.raw()), steps, true, None))? {
                return Ok(());
            }
        }
        // a pushed-down end is reached: any-shortest is done, all-shortest has just
        // finished the end's layer
        if ctx.end_filter.is_some() && !rows.is_empty() {
            return Ok(());
        }
        layer = next;
    }
    Ok(())
}

/// Time-respecting `ANY_SHORTEST` / `ALL_SHORTEST` from time `tau0` (`i64::MIN` is
/// −∞). An entry of layer `k` is `(node, state, time)`; it is dropped when the pair
/// had an equal or earlier time in an earlier layer: every continuation of it is
/// allowed from that earlier entry too (`Nb::step_time` is monotone) and ends
/// sooner, so it lies on no shortest path. Entries of one layer with different
/// times are all kept, since each may be the only one that can continue. Every
/// shortest time-respecting walk is then a path of the layered DAG, and entries are
/// created in (parent order, hop key) order, so the first target of an end is the
/// lexicographically smallest shortest path, as in `run`.
pub(super) fn run_timed(ctx: &mut Ctx<'_, '_>, sink: Sink<'_>, all: bool, tau0: i64) -> Result<()> {
    let start = ctx.start;
    let q0 = ctx.dfa.start;
    let mut arena = vec![SNode {
        node: start.raw(),
        state: q0,
        depth: 0,
        tau: tau0,
        preds: Vec::new(),
    }];
    ctx.budget.charge(1)?;
    // the earliest time of each pair over the completed layers
    let mut best: HashMap<(i64, u32), i64> = HashMap::new();
    best.insert((start.raw(), q0), tau0);
    let mut emitted: HashSet<i64> = HashSet::new();
    if ctx.dfa.accepting[q0 as usize] {
        emitted.insert(start.raw());
        if ctx.wants(start.raw()) && !sink(row_of(start, &[], true, ctx.arrival(tau0)))? {
            return Ok(());
        }
        if ctx.end_filter.is_some_and(|e| e.raw() == start.raw()) {
            return Ok(());
        }
    }
    let mut layer: Vec<u32> = vec![0];
    let mut depth = 0u32;
    while !layer.is_empty() && ctx.depth_ok(depth) {
        let entries: Vec<(i64, u32)> = layer
            .iter()
            .map(|i| (arena[*i as usize].node, arena[*i as usize].state))
            .collect();
        let exp = expand(ctx, &entries)?;
        let mut next: Vec<u32> = Vec::new();
        let mut index: HashMap<(i64, u32, i64), u32> = HashMap::new();
        for (k, list) in exp.iter().enumerate() {
            let parent = layer[k];
            let tau = arena[parent as usize].tau;
            for (nb, t) in list {
                let Some(at) = nb.step_time(tau) else {
                    continue;
                };
                if best.get(&(nb.to, *t)).is_some_and(|b| *b <= at) {
                    continue;
                }
                let h = hop_of(nb);
                match index.get(&(nb.to, *t, at)) {
                    None => {
                        ctx.budget.charge(1)?;
                        let idx = arena.len() as u32;
                        arena.push(SNode {
                            node: nb.to,
                            state: *t,
                            depth: depth + 1,
                            tau: at,
                            preds: vec![(parent, h)],
                        });
                        index.insert((nb.to, *t, at), idx);
                        next.push(idx);
                    }
                    Some(&idx) if all => {
                        ctx.budget.charge(1)?;
                        arena[idx as usize].preds.push((parent, h));
                    }
                    Some(_) => {}
                }
            }
        }
        depth += 1;
        for i in &next {
            let n = &arena[*i as usize];
            best.entry((n.node, n.state))
                .and_modify(|b| *b = (*b).min(n.tau))
                .or_insert(n.tau);
        }
        let targets: Vec<u32> = next
            .iter()
            .copied()
            .filter(|i| {
                let n = &arena[*i as usize];
                ctx.dfa.accepting[n.state as usize] && !emitted.contains(&n.node)
            })
            .collect();
        let mut rows: Vec<(Vec<(Hop, i64)>, i64)> = Vec::new();
        if all {
            for i in &targets {
                let n = &arena[*i as usize];
                if !ctx.wants(n.node) {
                    continue;
                }
                for p in paths_to(&arena, *i) {
                    ctx.budget.charge(1)?;
                    rows.push((p, n.tau));
                }
            }
            rows.sort_by_key(|(r, _)| key_of(r));
        } else {
            let mut seen: HashSet<i64> = HashSet::new();
            for i in &targets {
                let n = &arena[*i as usize];
                if ctx.wants(n.node) && seen.insert(n.node) {
                    rows.push((paths_to(&arena, *i).swap_remove(0), n.tau));
                }
            }
        }
        for i in &targets {
            emitted.insert(arena[*i as usize].node);
        }
        for (steps, at) in &rows {
            let row = row_of(
                ObjectId::from_raw(start.raw()),
                steps,
                true,
                ctx.arrival(*at),
            );
            if !sink(row)? {
                return Ok(());
            }
        }
        if ctx.end_filter.is_some() && !rows.is_empty() {
            return Ok(());
        }
        layer = next;
    }
    Ok(())
}
