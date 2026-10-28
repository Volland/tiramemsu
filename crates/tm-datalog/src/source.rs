//! Stored statements as the engine sees them: the view's statements, within
//! the run's scope, each with its valid time as an interval set
//! (`lat.md/datalog#Probe Or Load Per Atom`).
//!
//! A predicate is probed through the store's permutations until it has been
//! probed more than an eighth of its statement count; then (or at once, when
//! a scan would read it whole) it is loaded into memory with hash indexes per
//! bound-position mask. Either way the same rows come back in eid order.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use rustc_hash::{FxHashMap, FxHashSet};
use smallvec::SmallVec;
use tm_core::store::{Pattern, ReadStore, StmtRow};
use tm_core::{budget, Result, Valid};
use tm_store::read::View;

use crate::interval::IvSet;
use crate::terms::Id;

/// One stored statement: `(s, p, o, eid)`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// Subject.
    pub s: Id,
    /// Predicate.
    pub p: Id,
    /// Object.
    pub o: Id,
    /// Statement eid (raw).
    pub e: Id,
}

impl Row {
    fn get(&self, i: usize) -> Id {
        match i {
            0 => self.s,
            1 => self.p,
            2 => self.o,
            _ => self.e,
        }
    }
}

/// A pattern over `(s, p, o, e)`; `None` positions are free.
pub type Pat4 = [Option<Id>; 4];

fn mask(p: &Pat4) -> u8 {
    p.iter()
        .enumerate()
        .fold(0, |m, (i, x)| if x.is_some() { m | (1 << i) } else { m })
}

type Key = SmallVec<[Id; 4]>;
type Index = FxHashMap<Key, Vec<u32>>;

fn row_key(r: &Row, m: u8) -> Key {
    (0..4)
        .filter(|i| m & (1 << i) != 0)
        .map(|i| r.get(i))
        .collect()
}

/// The statements of one predicate (or of all, `p = None`), in eid order.
pub struct Loaded {
    rows: Vec<Row>,
    ivs: Vec<IvSet>,
    idx: RwLock<FxHashMap<u8, Arc<Index>>>,
}

impl Loaded {
    fn index(&self, m: u8) -> Arc<Index> {
        if let Some(ix) = self.idx.read().unwrap_or_else(|p| p.into_inner()).get(&m) {
            return ix.clone();
        }
        let mut ix: Index = FxHashMap::default();
        for (i, r) in self.rows.iter().enumerate() {
            ix.entry(row_key(r, m)).or_default().push(i as u32);
        }
        let ix = Arc::new(ix);
        self.idx
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .insert(m, ix.clone());
        ix
    }
}

/// How the run sees valid time.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TimeView {
    /// Statements carry their valid time.
    Valid,
    /// Valid time is ignored: every statement holds always.
    Atemporal,
    /// A `valid_at(t)` view: statements hold at `[t, t+1)`.
    At(i64),
}

/// What an explain reports about one predicate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Access {
    /// The predicate id (`None`: every predicate).
    pub p: Option<Id>,
    /// True when it was loaded into memory.
    pub loaded: bool,
    /// The number of probes made before (or instead of) loading.
    pub probes: u64,
    /// The number of statements loaded.
    pub rows: u64,
}

/// The stored side of one run.
// @tg: implements:: [[openspec:datalog-evaluation#Scope and pruning]] {since: "2026-10-28"}
// @tg: implements:: [[openspec:datalog-evaluation#Budgets and cancellation]] {since: "2026-10-28"}
pub struct Stored<'s> {
    view: View,
    scope: Option<FxHashSet<Id>>,
    exclude: FxHashSet<Id>,
    time: TimeView,
    loaded: RwLock<FxHashMap<Option<Id>, Arc<Loaded>>>,
    probes: Mutex<FxHashMap<Option<Id>, (u64, u64)>>,
    /// When false, never load (an explain or test may force probing).
    pub allow_load: bool,
    /// When true, load every predicate at first use.
    pub force_load: bool,
    read: AtomicU64,
    _store: std::marker::PhantomData<&'s ()>,
}

impl<'s> Stored<'s> {
    /// The stored side for `view`, restricted to the eids `scope` (when given),
    /// without the eids in `exclude`.
    pub fn new(
        view: View,
        scope: Option<FxHashSet<Id>>,
        exclude: FxHashSet<Id>,
        time: TimeView,
    ) -> Stored<'s> {
        Stored {
            view,
            scope,
            exclude,
            time,
            loaded: RwLock::new(FxHashMap::default()),
            probes: Mutex::new(FxHashMap::default()),
            allow_load: true,
            force_load: false,
            read: AtomicU64::new(0),
            _store: std::marker::PhantomData,
        }
    }

    /// The valid-time set of a row.
    pub fn iv(&self, r: &StmtRow) -> IvSet {
        match self.time {
            TimeView::Atemporal => IvSet::all(),
            TimeView::Valid => IvSet::of_valid(Valid {
                from: r.v_from,
                to: r.v_to,
            }),
            TimeView::At(t) => IvSet::of_valid(Valid {
                from: r.v_from,
                to: r.v_to,
            })
            .clip(t, t.saturating_add(1)),
        }
    }

    fn admits(&self, r: &StmtRow) -> bool {
        self.view.selects(r)
            && !self.exclude.contains(&r.eid)
            && self.scope.as_ref().is_none_or(|s| s.contains(&r.eid))
    }

    /// The number of statements read from the store so far.
    pub fn statements_read(&self) -> u64 {
        self.read.load(Ordering::Relaxed)
    }

    /// The access of every predicate touched, by predicate id.
    pub fn accesses(&self) -> Vec<Access> {
        let loaded = self.loaded.read().unwrap_or_else(|p| p.into_inner());
        let probes = self.probes.lock().unwrap_or_else(|p| p.into_inner());
        let mut keys: Vec<Option<Id>> = loaded.keys().chain(probes.keys()).copied().collect();
        keys.sort_unstable();
        keys.dedup();
        keys.into_iter()
            .map(|p| Access {
                p,
                loaded: loaded.contains_key(&p),
                probes: probes.get(&p).map_or(0, |x| x.0),
                rows: loaded.get(&p).map_or(0, |l| l.rows.len() as u64),
            })
            .collect()
    }

    fn scan(&self, store: &dyn ReadStore, pat: Pattern) -> Result<Vec<(Row, IvSet)>> {
        let mut out = Vec::new();
        for r in store.scan(pat, self.view.scope())? {
            let r = r?;
            budget::poll()?;
            self.read.fetch_add(1, Ordering::Relaxed);
            if self.admits(&r) {
                let iv = self.iv(&r);
                if !iv.is_empty() {
                    out.push((
                        Row {
                            s: r.s,
                            p: r.p,
                            o: r.o,
                            e: r.eid,
                        },
                        iv,
                    ));
                }
            }
        }
        out.sort_unstable_by_key(|(r, _)| r.e);
        Ok(out)
    }

    fn load(&self, store: &dyn ReadStore, p: Option<Id>) -> Result<Arc<Loaded>> {
        if let Some(l) = self
            .loaded
            .read()
            .unwrap_or_else(|x| x.into_inner())
            .get(&p)
        {
            return Ok(l.clone());
        }
        let rows = self.scan(
            store,
            Pattern {
                s: None,
                p,
                o: None,
            },
        )?;
        budget::charge_bytes(rows.len() as u64 * 48)?;
        let (rows, ivs) = rows.into_iter().unzip();
        let l = Arc::new(Loaded {
            rows,
            ivs,
            idx: RwLock::new(FxHashMap::default()),
        });
        self.loaded
            .write()
            .unwrap_or_else(|x| x.into_inner())
            .insert(p, l.clone());
        Ok(l)
    }

    /// An estimate of the statements of predicate `p` (exact where the store
    /// keeps statistics, otherwise counted up to 4 096).
    pub fn estimate(&self, store: &dyn ReadStore, p: Option<Id>) -> Result<u64> {
        if let Some(p) = p {
            if let Some((live, _, _)) = store.predicate_stats(p)? {
                return Ok(live);
            }
        }
        store.count(
            Pattern {
                s: None,
                p,
                o: None,
            },
            self.view.scope(),
            4096,
        )
    }

    /// An estimate of the matches of a pattern with some positions bound.
    pub fn estimate_bound(&self, store: &dyn ReadStore, pat: &Pat4) -> Result<f64> {
        let total = self.estimate(store, pat[1])? as f64;
        if pat[3].is_some() {
            return Ok(1.0);
        }
        let stats = match pat[1] {
            Some(p) => store.predicate_stats(p)?,
            None => None,
        };
        Ok(match (pat[0].is_some(), pat[2].is_some(), stats) {
            (false, false, _) => total,
            (true, true, _) => 1.0,
            (true, false, Some((live, ds, _))) => live as f64 / ds.max(1) as f64,
            (false, true, Some((live, _, dob))) => live as f64 / dob.max(1) as f64,
            (_, _, None) => total.sqrt().max(1.0),
        })
    }

    /// The statements matching `pat`, with their interval sets, in eid order.
    pub fn matches(&self, store: &dyn ReadStore, pat: &Pat4) -> Result<Vec<(Row, IvSet)>> {
        let mut out = Vec::new();
        self.for_matches(store, pat, &mut |r, iv| {
            out.push((r, iv.clone()));
            Ok(())
        })?;
        Ok(out)
    }

    /// [`Stored::matches`] as a callback: a loaded predicate is read in place.
    pub fn for_matches(
        &self,
        store: &dyn ReadStore,
        pat: &Pat4,
        f: &mut dyn FnMut(Row, &IvSet) -> Result<()>,
    ) -> Result<()> {
        // a bound eid: one statement
        if let Some(e) = pat[3] {
            let Some(r) = store.statement(e)? else {
                return Ok(());
            };
            self.read.fetch_add(1, Ordering::Relaxed);
            let row = Row {
                s: r.s,
                p: r.p,
                o: r.o,
                e: r.eid,
            };
            let ok = (0..3).all(|i| pat[i].is_none_or(|x| x == row.get(i)));
            let iv = self.iv(&r);
            if ok && self.admits(&r) && !iv.is_empty() {
                f(row, &iv)?;
            }
            return Ok(());
        }
        let p = pat[1];
        let loaded = self
            .loaded
            .read()
            .unwrap_or_else(|x| x.into_inner())
            .get(&p)
            .cloned();
        let loaded = match loaded {
            Some(l) => Some(l),
            None if !self.allow_load => None,
            None => {
                let unbound = pat[0].is_none() && pat[2].is_none();
                let count = {
                    let mut probes = self.probes.lock().unwrap_or_else(|x| x.into_inner());
                    let entry = probes.entry(p).or_insert((0, u64::MAX));
                    entry.0 += 1;
                    if entry.1 == u64::MAX {
                        entry.1 = self.estimate(store, p)?;
                    }
                    *entry
                };
                if self.force_load || unbound || count.0.saturating_mul(8) > count.1 {
                    Some(self.load(store, p)?)
                } else {
                    None
                }
            }
        };
        match loaded {
            Some(l) => {
                let m = mask(pat) & !0b0010;
                if m == 0 {
                    for (r, iv) in l.rows.iter().zip(l.ivs.iter()) {
                        f(*r, iv)?;
                    }
                    return Ok(());
                }
                let ix = l.index(m);
                let k: Key = (0..4)
                    .filter(|i| m & (1 << i) != 0)
                    .map(|i| pat[i].unwrap_or_default())
                    .collect();
                if let Some(rows) = ix.get(&k) {
                    for &i in rows {
                        f(l.rows[i as usize], &l.ivs[i as usize])?;
                    }
                }
                Ok(())
            }
            None => {
                for (r, iv) in self.scan(
                    store,
                    Pattern {
                        s: pat[0],
                        p: pat[1],
                        o: pat[2],
                    },
                )? {
                    f(r, &iv)?;
                }
                Ok(())
            }
        }
    }
}
