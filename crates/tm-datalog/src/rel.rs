//! Derived relations: tuples of ids in one flat buffer, each with its coalesced
//! interval set, hash indexes per bound-position mask built on first use and
//! kept current on insert, the delta of the running iteration, and the
//! derivations that first produced each portion of a tuple's interval set
//! (`lat.md/datalog#Sorted Columnar Relations`, `#Provenance On Demand`).

use std::sync::{Arc, RwLock};

use rustc_hash::FxHashMap;
use smallvec::SmallVec;

use crate::interval::IvSet;
use crate::terms::Id;

/// A tuple key.
pub type Key = SmallVec<[Id; 4]>;

type Index = FxHashMap<Key, Vec<u32>>;

/// One derived relation.
// @tg: -contradicts:: [[datalog#Sorted Columnar Relations]] {reason: "hash indexes and index nested loops; the worst-case-optimal join for cyclic bodies is not built yet", until: "a cyclic workload in the Datalog benchmark"}
pub struct Rel {
    /// The arity.
    pub arity: usize,
    data: Vec<Id>,
    /// The interval set of each row.
    pub ivs: Vec<IvSet>,
    /// The part of each row's set that came from stored statements (seed rules).
    pub stored: Vec<IvSet>,
    map: FxHashMap<Key, u32>,
    idx: RwLock<FxHashMap<u32, Arc<Index>>>,
    /// Per row: the portions of its set and the derivation of each (provenance on).
    pub prov: Vec<SmallVec<[(IvSet, u32); 1]>>,
    /// The rows that grew in the last merge, with the new part.
    pub delta: Vec<(u32, IvSet)>,
    /// Total intervals over all rows.
    pub fragments: u64,
}

impl Rel {
    /// An empty relation.
    pub fn new(arity: usize) -> Rel {
        Rel {
            arity,
            data: Vec::new(),
            ivs: Vec::new(),
            stored: Vec::new(),
            map: FxHashMap::default(),
            idx: RwLock::new(FxHashMap::default()),
            prov: Vec::new(),
            delta: Vec::new(),
            fragments: 0,
        }
    }

    /// The number of tuples.
    pub fn len(&self) -> usize {
        self.ivs.len()
    }

    /// True when empty.
    pub fn is_empty(&self) -> bool {
        self.ivs.is_empty()
    }

    /// The tuple of `row`.
    pub fn tuple(&self, row: u32) -> &[Id] {
        let a = self.arity;
        &self.data[row as usize * a..(row as usize + 1) * a]
    }

    /// The row of a tuple.
    pub fn find(&self, tuple: &[Id]) -> Option<u32> {
        self.map.get(tuple).copied()
    }

    fn index(&self, mask: u32) -> Arc<Index> {
        if let Some(ix) = self
            .idx
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .get(&mask)
        {
            return ix.clone();
        }
        let mut ix: Index = FxHashMap::default();
        for row in 0..self.len() as u32 {
            ix.entry(key_of(self.tuple(row), mask))
                .or_default()
                .push(row);
        }
        let ix = Arc::new(ix);
        self.idx
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .insert(mask, ix.clone());
        ix
    }

    /// The rows whose positions in `mask` hold `key` (in position order).
    pub fn lookup(&self, mask: u32, key: &[Id]) -> Vec<u32> {
        if mask == 0 {
            return (0..self.len() as u32).collect();
        }
        let full = (1u32 << self.arity) - 1;
        if mask == full {
            return self.find(key).into_iter().collect();
        }
        self.index(mask).get(key).cloned().unwrap_or_default()
    }

    /// Calls `f` with each row whose positions in `mask` hold `key`, without
    /// copying the index bucket.
    pub fn for_rows(
        &self,
        mask: u32,
        key: &[Id],
        mut f: impl FnMut(u32) -> tm_core::Result<()>,
    ) -> tm_core::Result<()> {
        if mask == 0 {
            for row in 0..self.len() as u32 {
                f(row)?;
            }
            return Ok(());
        }
        if mask == (1u32 << self.arity) - 1 {
            if let Some(row) = self.find(key) {
                f(row)?;
            }
            return Ok(());
        }
        let ix = self.index(mask);
        if let Some(rows) = ix.get(key) {
            for &row in rows {
                f(row)?;
            }
        }
        Ok(())
    }

    /// Adds `iv` to `tuple`'s set; returns the row and the part that is new.
    pub fn insert(&mut self, tuple: &[Id], iv: &IvSet, prov: bool) -> Option<(u32, IvSet)> {
        if iv.is_empty() {
            return None;
        }
        match self.map.get(tuple).copied() {
            Some(row) => {
                let r = row as usize;
                let new = iv.subtract(&self.ivs[r]);
                if new.is_empty() {
                    return None;
                }
                let before = self.ivs[r].intervals().len() as u64;
                self.ivs[r] = self.ivs[r].union(&new);
                self.fragments = self.fragments + self.ivs[r].intervals().len() as u64 - before;
                Some((row, new))
            }
            None => {
                let row = self.len() as u32;
                self.data.extend_from_slice(tuple);
                self.ivs.push(iv.clone());
                self.stored.push(IvSet::empty());
                if prov {
                    self.prov.push(SmallVec::new());
                }
                self.fragments += iv.intervals().len() as u64;
                self.map.insert(tuple.iter().copied().collect(), row);
                let idx = self.idx.get_mut().unwrap_or_else(|p| p.into_inner());
                for (&mask, ix) in idx.iter_mut() {
                    Arc::make_mut(ix)
                        .entry(key_of(tuple, mask))
                        .or_default()
                        .push(row);
                }
                Some((row, iv.clone()))
            }
        }
    }
}

/// The positions of `mask` of a tuple, in order.
pub fn key_of(tuple: &[Id], mask: u32) -> Key {
    tuple
        .iter()
        .enumerate()
        .filter(|(i, _)| mask & (1 << i) != 0)
        .map(|(_, &x)| x)
        .collect()
}
