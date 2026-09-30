//! The search modes: breadth-first search over `(node, DFA state)`, one layer per
//! hop count, one batched fetch round per layer. Every mode shares `expand`.

mod reach;
mod shortest;
mod trail;

use std::collections::{HashMap, HashSet};

use tm_core::{Error, ObjectId, Result};
use tm_ir::PathMode;

use super::automaton::Dfa;
use super::fetch::{Fetcher, Nb};
use super::resolve::Resolved;
use super::row::{Hop, Path, PathRow};

/// Bounds the in-memory search state of one evaluation.
#[derive(Debug)]
pub struct StateBudget {
    used: usize,
    limit: usize,
}

impl StateBudget {
    /// A budget of `limit` search states.
    pub fn new(limit: usize) -> StateBudget {
        StateBudget { used: 0, limit }
    }

    /// Charges `n` states; fails with `PathLimitExceeded` past the limit.
    pub fn charge(&mut self, n: usize) -> Result<()> {
        self.used += n;
        if self.used > self.limit {
            return Err(Error::PathLimitExceeded { limit: self.limit });
        }
        Ok(())
    }
}

/// Everything one search needs.
pub struct Ctx<'a, 'b> {
    /// The automaton.
    pub dfa: &'a Dfa,
    /// Its resolved alphabet.
    pub res: &'a Resolved,
    /// The neighbour fetcher.
    pub fetch: &'a mut Fetcher<'b>,
    /// The state budget.
    pub budget: &'a mut StateBudget,
    /// The hop bound (`None` = unbounded).
    pub max_hops: Option<u32>,
    /// Only rows for this end.
    pub end_filter: Option<ObjectId>,
    /// The start node.
    pub start: ObjectId,
}

/// A row consumer; returns `false` to stop the search.
pub type Sink<'s> = &'s mut dyn FnMut(PathRow) -> Result<bool>;

impl Ctx<'_, '_> {
    pub(super) fn depth_ok(&self, depth: u32) -> bool {
        self.max_hops.is_none_or(|m| depth < m)
    }

    pub(super) fn wants(&self, end: i64) -> bool {
        self.end_filter.is_none_or(|e| e.raw() == end)
    }
}

/// Runs the search of `mode`, feeding `sink` layer by layer.
// @lat: [[query#Physical Planning#Path Engine]]
pub fn search(mode: PathMode, ctx: &mut Ctx<'_, '_>, sink: Sink<'_>) -> Result<()> {
    match mode {
        PathMode::Reachability => reach::run(ctx, sink),
        PathMode::Trail => trail::run(ctx, sink),
        PathMode::AnyShortest => shortest::run(ctx, sink, false),
        PathMode::AllShortest => shortest::run(ctx, sink, true),
    }
}

/// Expands a frontier: for each `(node, state)` entry, in order, its outgoing hops
/// with the DFA state reached, sorted by hop key. The fetch is grouped by letter, so
/// the number of statements depends on the alphabet and the chunk size, not on the
/// frontier width.
pub(super) fn expand(
    ctx: &mut Ctx<'_, '_>,
    frontier: &[(i64, u32)],
) -> Result<Vec<Vec<(Nb, u32)>>> {
    let n_letters = ctx.dfa.letters.len();
    let mut nodes: Vec<Vec<i64>> = vec![Vec::new(); n_letters];
    let mut seen: Vec<HashSet<i64>> = vec![HashSet::new(); n_letters];
    for (node, state) in frontier {
        for (li, _) in &ctx.dfa.trans[*state as usize] {
            if seen[*li].insert(*node) {
                nodes[*li].push(*node);
            }
        }
    }
    let mut fetched: Vec<HashMap<i64, Vec<Nb>>> = Vec::with_capacity(n_letters);
    for (li, ns) in nodes.iter().enumerate() {
        fetched.push(if ns.is_empty() {
            HashMap::new()
        } else {
            ctx.fetch.fetch(&ctx.res.letters[li], ns)?
        });
    }
    let mut out = Vec::with_capacity(frontier.len());
    for (node, state) in frontier {
        let mut list: Vec<(Nb, u32)> = Vec::new();
        for (li, target) in &ctx.dfa.trans[*state as usize] {
            if let Some(nbs) = fetched[*li].get(node) {
                list.extend(nbs.iter().map(|nb| (*nb, *target)));
            }
        }
        list.sort_by_key(|(nb, _)| nb.key());
        out.push(list);
    }
    Ok(out)
}

pub(super) fn hop_of(nb: &Nb) -> Hop {
    Hop {
        eid: ObjectId::from_raw(nb.eid),
        pred: ObjectId::from_raw(nb.p),
        dir: nb.dir,
        kind: nb.kind,
    }
}

/// A row for a path given as start node plus `(hop, node reached)` steps.
pub(super) fn row_of(start: ObjectId, steps: &[(Hop, i64)], with_path: bool) -> PathRow {
    let end = steps.last().map_or(start, |(_, n)| ObjectId::from_raw(*n));
    PathRow {
        start,
        end,
        hops: steps.len() as u32,
        path: with_path.then(|| Path {
            nodes: std::iter::once(start)
                .chain(steps.iter().map(|(_, n)| ObjectId::from_raw(*n)))
                .collect(),
            hops: steps.iter().map(|(h, _)| *h).collect(),
        }),
    }
}
