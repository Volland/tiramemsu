//! Fixtures for the path-engine tests: graphs written as `(s, p, o)` names.
#![allow(dead_code)]

use tiramemsu::*;

use super::{v, TestDb};

pub const REACH: PathMode = PathMode::Reachability;
pub const TRAIL: PathMode = PathMode::Trail;
pub const ANY: PathMode = PathMode::AnyShortest;
pub const ALL: PathMode = PathMode::AllShortest;

/// The local name of a `v:` IRI value.
pub fn local(val: &Value) -> String {
    match val {
        Value::Iri(i) => i
            .strip_prefix("urn:tiramemsu:v:")
            .unwrap_or(i.as_str())
            .to_string(),
        other => format!("{other:?}"),
    }
}

/// A test database plus name-based helpers.
pub struct G {
    pub t: TestDb,
}

impl G {
    pub fn new() -> G {
        G { t: TestDb::new() }
    }

    pub fn with(opts: OpenOptions) -> G {
        G {
            t: TestDb::open(opts),
        }
    }

    /// Asserts `(s p o)` for each triple in one transaction; returns the eids.
    pub fn edges(&self, edges: &[(&str, &str, &str)]) -> Vec<Eid> {
        let mut out = Vec::new();
        self.t.tx(|tx| {
            for (s, p, o) in edges {
                out.push(tx.assert(v(s), v(p), v(o), Valid::ALWAYS)?.eid());
            }
            Ok(())
        });
        out
    }

    pub fn edge(&self, s: &str, p: &str, o: &str) -> Eid {
        self.edges(&[(s, p, o)])[0]
    }

    /// The id of `v:name`.
    pub fn id(&self, name: &str) -> ObjectId {
        self.t.id(&v(name))
    }

    pub fn name(&self, id: ObjectId) -> String {
        local(&self.t.db.now().decode(id).unwrap())
    }

    /// `path` from `start` under `view`, as `(end, hops)`.
    pub fn ends(
        &self,
        view: View<'_>,
        start: &str,
        path: &str,
        mode: PathMode,
        max: u32,
    ) -> Vec<(String, u32)> {
        let rows = view.path(self.id(start), path, mode, max).unwrap();
        rows.iter()
            .map(|r| (local(&view.decode(r.end).unwrap()), r.hops))
            .collect()
    }

    /// Like [`G::ends`] under `Now`.
    pub fn now(&self, start: &str, path: &str, mode: PathMode) -> Vec<(String, u32)> {
        self.ends(self.t.db.now(), start, path, mode, u32::MAX)
    }

    /// Node names of every row's path under `Now`, in row order.
    pub fn paths(&self, start: &str, path: &str, mode: PathMode, max: u32) -> Vec<Vec<String>> {
        let view = self.t.db.now();
        view.path(self.id(start), path, mode, max)
            .unwrap()
            .iter()
            .map(|r| {
                r.path
                    .as_ref()
                    .expect("path value")
                    .nodes
                    .iter()
                    .map(|n| local(&view.decode(*n).unwrap()))
                    .collect()
            })
            .collect()
    }
}

/// `(name, hops)` pairs from literals.
pub fn eh(items: &[(&str, u32)]) -> Vec<(String, u32)> {
    items.iter().map(|(n, h)| (n.to_string(), *h)).collect()
}

/// Sorted copy (for set comparisons).
pub fn sorted_vec<T: Ord + Clone>(x: &[T]) -> Vec<T> {
    let mut v = x.to_vec();
    v.sort();
    v
}

pub fn chain(g: &G, pred: &str, names: &[&str]) {
    let edges: Vec<(&str, &str, &str)> = names.windows(2).map(|w| (w[0], pred, w[1])).collect();
    g.edges(&edges);
}
