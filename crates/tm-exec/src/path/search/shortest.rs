//! `ANY_SHORTEST` and `ALL_SHORTEST` (after Martens et al., "Evaluating regular path
//! queries under the all-shortest paths semantics"): a BFS records each
//! `(node, state)`'s first-discovery depth and its predecessors. Any-shortest keeps
//! the first predecessor, which, with the frontier in canonical order and neighbours
//! in hop-key order, is the lexicographically smallest shortest path. All-shortest
//! keeps every same-depth predecessor and enumerates the layered DAG backwards from
//! each layer's targets, so every minimal path appears once.

use std::collections::{HashMap, HashSet};

use tm_core::{ObjectId, Result};

use super::{expand, hop_of, row_of, Ctx, Sink};
use crate::path::row::Hop;

struct SNode {
    node: i64,
    state: u32,
    depth: u32,
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
        preds: Vec::new(),
    }];
    let mut index: HashMap<(i64, u32), u32> = HashMap::new();
    index.insert((start.raw(), q0), 0);
    ctx.budget.charge(1)?;
    let mut emitted: HashSet<i64> = HashSet::new();
    if ctx.dfa.accepting[q0 as usize] {
        emitted.insert(start.raw());
        if ctx.wants(start.raw()) && !sink(row_of(start, &[], true))? {
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
            if !sink(row_of(ObjectId::from_raw(start.raw()), steps, true))? {
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
