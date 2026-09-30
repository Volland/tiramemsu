//! `TRAIL`: every path in which no relationship identity repeats. The search tree
//! lives in an arena of `(node, state, parent, hop)` entries, so trails that share
//! a prefix share memory; the identity check walks the parent chain (at most the
//! hop bound). Layers come out in hop-key order because each layer expands its
//! parent in arena order over hop-key-sorted neighbours.

use std::ops::Range;

use tm_core::{ObjectId, Result};

use super::{expand, hop_of, row_of, Ctx, Sink};
use crate::path::row::Hop;

struct TNode {
    node: i64,
    state: u32,
    parent: u32,
    hop: Option<Hop>,
}

const ROOT: u32 = u32::MAX;

fn on_chain(arena: &[TNode], mut at: u32, ident: (i64, u8)) -> bool {
    while at != ROOT {
        let n = &arena[at as usize];
        if n.hop.as_ref().is_some_and(|h| h.identity() == ident) {
            return true;
        }
        at = n.parent;
    }
    false
}

fn steps_of(arena: &[TNode], mut at: u32) -> Vec<(Hop, i64)> {
    let mut steps = Vec::new();
    while at != ROOT {
        let n = &arena[at as usize];
        if let Some(h) = n.hop {
            steps.push((h, n.node));
        }
        at = n.parent;
    }
    steps.reverse();
    steps
}

pub(super) fn run(ctx: &mut Ctx<'_, '_>, sink: Sink<'_>) -> Result<()> {
    let start = ctx.start;
    let q0 = ctx.dfa.start;
    let mut arena = vec![TNode {
        node: start.raw(),
        state: q0,
        parent: ROOT,
        hop: None,
    }];
    ctx.budget.charge(1)?;
    if ctx.dfa.accepting[q0 as usize] && ctx.wants(start.raw()) && !sink(row_of(start, &[], true))?
    {
        return Ok(());
    }
    let mut layer: Range<usize> = 0..1;
    let mut depth = 0u32;
    while !layer.is_empty() && ctx.depth_ok(depth) {
        let entries: Vec<(i64, u32)> = layer
            .clone()
            .map(|i| (arena[i].node, arena[i].state))
            .collect();
        let exp = expand(ctx, &entries)?;
        let first_new = arena.len();
        for (k, list) in exp.iter().enumerate() {
            let parent = (layer.start + k) as u32;
            for (nb, t) in list {
                let h = hop_of(nb);
                if on_chain(&arena, parent, h.identity()) {
                    continue;
                }
                ctx.budget.charge(1)?;
                arena.push(TNode {
                    node: nb.to,
                    state: *t,
                    parent,
                    hop: Some(h),
                });
            }
        }
        depth += 1;
        for i in first_new..arena.len() {
            let n = &arena[i];
            if ctx.dfa.accepting[n.state as usize] && ctx.wants(n.node) {
                let steps = steps_of(&arena, i as u32);
                if !sink(row_of(ObjectId::from_raw(start.raw()), &steps, true))? {
                    return Ok(());
                }
            }
        }
        layer = first_new..arena.len();
    }
    Ok(())
}
