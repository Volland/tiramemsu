//! Basic graph patterns: the join engine over the store's permutations.
//!
//! The patterns are ordered greedily from estimated cardinalities: exact
//! predicate statistics where the store keeps them, bounded range probes
//! otherwise. An acyclic pattern set is joined pattern by pattern, each step an
//! index nested loop (a prefix scan per row) or a hash join (one scan, hashed on
//! the shared variables), whichever is estimated cheaper. A cyclic set is joined
//! with a worst-case-optimal generic join over hash tries built once per pattern
//! (Umbra / Free Join style), so its work is bounded by the worst-case output.
//!
//! The SQL route's semantics are kept: each pattern sees its own view, a
//! repeated variable is an equality, the canonical-eid predicate gives set
//! semantics, and under relationship isomorphism the eids of one match group
//! differ.

use std::collections::{BTreeSet, HashMap};
use std::hash::Hasher;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering as AtomicOrdering};
use std::sync::Mutex;

use rustc_hash::{FxHashMap, FxHasher};
use tm_core::{budget, ObjectId, Result, Value};
use tm_exec::plan::analyze::is_cyclic;
use tm_exec::plan::{PExpr, PTerm, PTriple};
use tm_exec::udf::{kind_of_value, K_NUM};
use tm_ir::CmpOp;
use tm_ir::{MatchMode, Var};
use tm_store::{Pattern, ReadStore, StmtRow, View};

use crate::exec::{store_view, Exec};
use crate::rel::{Col, Iso, Rel};
use crate::value::V;

const UNBOUND: i64 = i64::MIN;
/// The most keys a cardinality probe reads (a larger range estimates as this).
const PROBE_CAP: u64 = 4_096;

/// Join rows stored flat, `w` slots per row in one buffer, so that extending a
/// row allocates nothing. The count is kept apart because a pattern set without
/// variables still has one (empty) row per match.
#[derive(Clone, Debug, Default)]
pub struct Rows {
    w: usize,
    n: usize,
    data: Vec<i64>,
}

impl Rows {
    fn new(w: usize) -> Rows {
        Rows {
            w,
            n: 0,
            data: Vec::new(),
        }
    }

    /// Number of rows.
    pub fn len(&self) -> usize {
        self.n
    }

    /// True when there is no row.
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    fn row(&self, i: usize) -> &[i64] {
        &self.data[i * self.w..(i + 1) * self.w]
    }

    /// The rows in order.
    pub fn iter(&self) -> impl Iterator<Item = &[i64]> + '_ {
        (0..self.n).map(move |i| self.row(i))
    }

    fn push(&mut self, r: &[i64]) {
        debug_assert_eq!(r.len(), self.w);
        self.data.extend_from_slice(r);
        self.n += 1;
    }

    /// Appends a copy of `r` and returns it to be extended; [`Rows::pop`] takes
    /// it back when the extension fails.
    fn push_tail(&mut self, r: &[i64]) -> &mut [i64] {
        self.push(r);
        let at = self.data.len() - self.w;
        &mut self.data[at..]
    }

    fn pop(&mut self) {
        self.n -= 1;
        self.data.truncate(self.n * self.w);
    }

    fn extend(&mut self, other: Rows) {
        self.n += other.n;
        self.data.extend_from_slice(&other.data);
    }
}

/// The hash of the values of `slots` in `row`.
fn hash_slots(slots: &[usize], row: &[i64]) -> u64 {
    let mut h = FxHasher::default();
    for s in slots {
        h.write_i64(row[*s]);
    }
    h.finish()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pos {
    Const(i64),
    Slot(usize),
}

#[derive(Clone, Debug)]
struct Pat {
    pos: [Pos; 3],
    eid: Option<usize>,
    /// A hidden slot holding the eid for isomorphism checks.
    iso_eid: Option<usize>,
    view: View,
    canonical: bool,
    iso_group: Option<u32>,
    pred: Option<i64>,
    /// Matching statements with only the constants bound.
    card: u64,
    stats: Option<(u64, u64, u64)>,
}

impl Pat {
    fn slots(&self) -> Vec<usize> {
        let mut v: Vec<usize> = self
            .pos
            .iter()
            .filter_map(|p| match p {
                Pos::Slot(s) => Some(*s),
                Pos::Const(_) => None,
            })
            .collect();
        v.extend(self.eid);
        v.sort_unstable();
        v.dedup();
        v
    }

    fn pattern(&self, row: Option<&[i64]>) -> Pattern {
        let get = |p: Pos| match p {
            Pos::Const(c) => Some(c),
            Pos::Slot(s) => row.and_then(|r| (r[s] != UNBOUND).then_some(r[s])),
        };
        Pattern::new(get(self.pos[0]), get(self.pos[1]), get(self.pos[2]))
    }

    /// Estimated matches per probe when `bound` slots are bound.
    fn estimate(&self, bound: &[bool]) -> f64 {
        let mut est = self.card.max(1) as f64;
        for (i, p) in self.pos.iter().enumerate() {
            if let Pos::Slot(s) = p {
                if bound[*s] {
                    let distinct = match (i, self.stats) {
                        (0, Some((_, ds, _))) => ds.max(1) as f64,
                        (2, Some((_, _, d))) => d.max(1) as f64,
                        _ => (self.card.max(1) as f64).sqrt().max(1.0),
                    };
                    est /= distinct;
                }
            }
        }
        if self.eid.is_some_and(|e| bound[e]) {
            est = est.min(1.0);
        }
        est.max(0.01)
    }
}

/// Binds row `r` of a pattern into `row`; false when it does not match.
fn bind(p: &Pat, r: &StmtRow, row: &mut [i64]) -> bool {
    let vals = [r.s, r.p, r.o];
    for (pos, v) in p.pos.iter().zip(vals) {
        match pos {
            Pos::Const(c) => {
                if *c != v {
                    return false;
                }
            }
            Pos::Slot(s) => {
                if row[*s] == UNBOUND {
                    row[*s] = v;
                } else if row[*s] != v {
                    return false;
                }
            }
        }
    }
    for slot in [p.eid, p.iso_eid].into_iter().flatten() {
        if row[slot] == UNBOUND {
            row[slot] = r.eid;
        } else if row[slot] != r.eid {
            return false;
        }
    }
    true
}

/// The engine of one basic graph pattern.
pub struct Bgp<'e, 'a> {
    ex: &'e Exec<'a>,
    pats: Vec<Pat>,
    vars: Vec<Var>,
    nslots: usize,
    iso: bool,
    canonical: HashMap<(i64, i64, i64, usize), i64>,
    /// Explain lines.
    pub plan: Vec<String>,
    /// Conjuncts over the patterns' variables, each with the slots it reads and
    /// whether it was applied; checked as soon as those slots are bound.
    filters: Vec<(PExpr, Vec<usize>, bool)>,
    /// Slots a conjunct `?v = constant` fixes to the constant's id: bound
    /// before the join, so it seeks instead of filtering every row.
    pins: Vec<(usize, i64)>,
}

impl<'e, 'a> Bgp<'e, 'a> {
    /// The engine for `pats`, with `seed` variables already bound by the rows the
    /// caller passes to [`Bgp::run`].
    pub fn new(ex: &'e Exec<'a>, pats: &[&PTriple], seed: &[Var]) -> Result<Bgp<'e, 'a>> {
        let mut vars: Vec<Var> = seed.to_vec();
        let slot = |v: &Var, vars: &mut Vec<Var>| match vars.iter().position(|x| x == v) {
            Some(i) => i,
            None => {
                vars.push(v.clone());
                vars.len() - 1
            }
        };
        let iso = ex.sem.match_mode == MatchMode::RelIsomorphism;
        let mut compiled = Vec::with_capacity(pats.len());
        let mut hidden = Vec::new();
        for t in pats {
            let pos = [&t.s, &t.p, &t.o].map(|p| match p {
                PTerm::Id(id) => Pos::Const(id.raw()),
                PTerm::Var(v) => Pos::Slot(slot(v, &mut vars)),
            });
            let eid = t.eid.as_ref().map(|e| slot(e, &mut vars));
            let pred = match pos[1] {
                Pos::Const(c) => Some(c),
                Pos::Slot(_) => None,
            };
            compiled.push(Pat {
                pos,
                eid,
                iso_eid: None,
                view: store_view(&t.view),
                canonical: t.canonical,
                iso_group: if iso { t.iso_group } else { None },
                pred,
                card: 0,
                stats: None,
            });
            hidden.push(iso && t.iso_group.is_some() && eid.is_none());
        }
        let nvars = vars.len();
        let mut nslots = nvars;
        for (p, h) in compiled.iter_mut().zip(&hidden) {
            if *h {
                p.iso_eid = Some(nslots);
                nslots += 1;
            }
        }
        for p in &mut compiled {
            let pat = p.pattern(None);
            if let Some(pr) = p.pred {
                p.stats = ex.store.predicate_stats(pr)?;
            }
            // a pattern bound on its predicate only: the exact live count, also
            // as the estimate of a historical view (its lower bound); otherwise a
            // bounded range probe
            p.card = match (pat.s, pat.o, p.stats) {
                (None, None, Some((live, _, _))) => live,
                _ => ex.store.count(pat, p.view.scope(), PROBE_CAP)?,
            };
        }
        Ok(Bgp {
            ex,
            pats: compiled,
            vars,
            nslots,
            iso,
            canonical: HashMap::new(),
            plan: Vec::new(),
            filters: Vec::new(),
            pins: Vec::new(),
        })
    }

    /// Sets the conjuncts to check during the join, each as soon as the slots it
    /// reads are bound.
    pub fn set_filters(&mut self, filters: &[PExpr]) {
        self.pins = filters.iter().filter_map(|f| self.pin(f)).collect();
        let names: Vec<String> = self.vars.iter().map(|v| format!("{v:?}")).collect();
        self.filters = filters
            .iter()
            .map(|f| {
                let needs = (0..names.len())
                    .filter(|s| crate::exec::mentions_any(f, std::slice::from_ref(&names[*s])))
                    .collect();
                (f.clone(), needs, false)
            })
            .collect();
    }

    /// Keeps the rows on which every filter whose slots are bound holds (`all`:
    /// every filter not yet applied), as SQLite tests a `WHERE` term in the
    /// first loop that binds its columns.
    fn apply_filters(&mut self, rows: Rows, all: bool) -> Result<Rows> {
        if rows.is_empty() {
            return Ok(rows);
        }
        let first = rows.row(0);
        let ready: Vec<usize> = (0..self.filters.len())
            .filter(|i| {
                let (_, needs, done) = &self.filters[*i];
                !done
                    && (all
                        || needs
                            .iter()
                            .all(|s| first.get(*s).is_some_and(|v| *v != UNBOUND)))
            })
            .collect();
        if ready.is_empty() {
            return Ok(rows);
        }
        let nv = self.vars.len();
        let rel = Rel {
            vars: self.vars.clone(),
            cols: (0..nv).map(|_| Col::term(false)).collect(),
            rows: Vec::new(),
            iso: Vec::new(),
        };
        let mut out = Rows::new(rows.w);
        let mut vals: Vec<V> = Vec::with_capacity(nv);
        for r in rows.iter() {
            budget::poll()?;
            vals.clear();
            vals.extend(r[..nv].iter().map(|x| {
                if *x == UNBOUND {
                    V::Null
                } else {
                    V::Integer(*x)
                }
            }));
            let mut ok = true;
            for i in &ready {
                if crate::value::cond(&self.ex.cond(&self.filters[*i].0, &rel, &vals)?)
                    != Some(true)
                {
                    ok = false;
                    break;
                }
            }
            if ok {
                out.push(r);
            }
        }
        drop(rows);
        for i in &ready {
            self.filters[*i].2 = true;
        }
        Ok(out)
    }

    // @tg: implements:: [[openspec:native-query-execution#Constant equalities seek]] {since: "2026-10-27"}
    /// The slot and id a conjunct `?v = k` fixes. On a term the native
    /// evaluator compares such a constant by id (`eq_const`, for a
    /// constant that is neither a number nor a date), so binding `?v` to the id
    /// keeps exactly the rows the conjunct keeps; the conjunct is still checked.
    fn pin(&self, f: &PExpr) -> Option<(usize, i64)> {
        let PExpr::Cmp(CmpOp::Eq, a, b) = f else {
            return None;
        };
        let (v, k) = match (a.as_ref(), b.as_ref()) {
            (PExpr::Var(v), PExpr::Const(k)) | (PExpr::Const(k), PExpr::Var(v)) => (v, k),
            _ => return None,
        };
        if kind_of_value(&k.value) == K_NUM || matches!(k.value, Value::DateTime { .. }) {
            return None;
        }
        let slot = self.vars.iter().position(|x| x == v)?;
        let id = k.id?;
        self.pats
            .iter()
            .any(|p| p.slots().contains(&slot))
            .then_some((slot, id.raw()))
    }

    /// The output variables: the seed variables, then the patterns' in order.
    pub fn vars(&self) -> &[Var] {
        &self.vars
    }

    fn canonical_ok(&mut self, i: usize, r: &StmtRow) -> Result<bool> {
        if !self.pats[i].canonical {
            return Ok(true);
        }
        let key = (r.s, r.p, r.o, i);
        let min = match self.canonical.get(&key) {
            Some(m) => *m,
            None => {
                let view = self.pats[i].view;
                let m = view
                    .scan(self.ex.store, Pattern::new(Some(r.s), Some(r.p), Some(r.o)))?
                    .filter_map(|x| x.ok())
                    .map(|x| x.eid)
                    .min()
                    .unwrap_or(r.eid);
                self.canonical.insert(key, m);
                m
            }
        };
        Ok(r.eid == min)
    }

    /// The eids of the patterns before `upto` in `i`'s match group that `row`
    /// must differ from.
    fn iso_ok(&self, i: usize, row: &[i64]) -> bool {
        iso_ok_in(&self.pats, i, row)
    }

    fn scan(&self, i: usize, row: Option<&[i64]>) -> Result<Vec<StmtRow>> {
        let p = &self.pats[i];
        let rows: Vec<StmtRow> = p
            .view
            .scan(self.ex.store, p.pattern(row))?
            .collect::<Result<_>>()?;
        Ok(rows)
    }

    // @tg: implements:: [[openspec:native-query-execution#Cost-based join order]] {since: "2026-10-27"}
    /// The join order: from each possible first pattern, a greedy completion
    /// that takes the connected pattern with the fewest matches per row next;
    /// the order with the fewest estimated intermediate rows wins. Starting from
    /// the smallest pattern alone can grow a large intermediate result that a
    /// selective pattern taken first would have avoided (WatDiv F1).
    fn order(&self, seeded: &[bool], seed_rows: usize) -> Vec<usize> {
        let n = self.pats.len();
        let any_seeded = seeded.iter().any(|b| *b);
        let mut best: Option<(f64, Vec<usize>)> = None;
        for first in 0..n {
            // with seed variables, only a pattern connected to them may start
            if any_seeded && !self.pats[first].slots().iter().any(|s| seeded[*s]) {
                continue;
            }
            let (cost, order) = self.greedy(seeded, seed_rows, first);
            if best.as_ref().is_none_or(|(c, _)| cost < *c) {
                best = Some((cost, order));
            }
        }
        match best {
            Some((_, order)) => order,
            // no pattern touches the seed variables: a cross product
            None => self.greedy(seeded, seed_rows, usize::MAX).1,
        }
    }

    /// The greedy order starting with pattern `first` (any, when out of range)
    /// and its cost, the sum of the estimated rows after each step.
    fn greedy(&self, seeded: &[bool], seed_rows: usize, first: usize) -> (f64, Vec<usize>) {
        let n = self.pats.len();
        let mut bound = seeded.to_vec();
        bound.resize(self.nslots, false);
        let mut done = vec![false; n];
        let mut out = Vec::with_capacity(n);
        let mut rows = seed_rows.max(1) as f64;
        let mut cost = 0.0;
        for k in 0..n {
            let connected =
                |i: usize, bound: &[bool]| self.pats[i].slots().iter().any(|s| bound[*s]);
            let any_connected = (0..n).any(|i| !done[i] && connected(i, &bound));
            let next = if k == 0 && first < n {
                first
            } else {
                (0..n)
                    .filter(|i| !done[*i])
                    .filter(|i| !any_connected || connected(*i, &bound))
                    .min_by(|a, b| {
                        self.pats[*a]
                            .estimate(&bound)
                            .partial_cmp(&self.pats[*b].estimate(&bound))
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .expect("a pattern left")
            };
            rows *= self.pats[next].estimate(&bound);
            cost += rows;
            done[next] = true;
            for s in self.pats[next].slots() {
                bound[s] = true;
            }
            if let Some(h) = self.pats[next].iso_eid {
                bound[h] = true;
            }
            out.push(next);
        }
        (cost, out)
    }

    /// Joins every pattern, starting from `seed` rows (values of the seed
    /// variables, all terms); returns rows over [`Bgp::vars`].
    pub fn run(&mut self, seed: Vec<Vec<i64>>) -> Result<Rows> {
        let mut seeded: Vec<bool> = (0..self.nslots)
            .map(|s| seed.first().is_some_and(|r| s < r.len()))
            .collect();
        let edges: Vec<BTreeSet<Var>> = self
            .pats
            .iter()
            .map(|p| {
                p.slots()
                    .into_iter()
                    .map(|s| self.vars[s].clone())
                    .collect()
            })
            .collect();
        let rows =
            if self.pats.len() >= 3 && seed.len() == 1 && seed[0].is_empty() && is_cyclic(&edges) {
                self.generic_join()?
            } else {
                let pins: Vec<(usize, i64)> = self
                    .pins
                    .iter()
                    .copied()
                    .filter(|(s, _)| !seeded[*s])
                    .collect();
                for (s, id) in &pins {
                    seeded[*s] = true;
                    self.plan.push(format!(
                        "bound {} to constant {id} from an equality",
                        self.vars[*s].name()
                    ));
                }
                let mut rows = Rows::new(self.nslots);
                rows.data.reserve(seed.len() * self.nslots);
                for mut r in seed {
                    r.resize(self.nslots, UNBOUND);
                    for (s, id) in &pins {
                        r[*s] = *id;
                    }
                    rows.push(&r);
                }
                rows = self.apply_filters(rows, false)?;
                for i in self.order(&seeded, rows.len()) {
                    rows = self.step(i, rows)?;
                    rows = self.apply_filters(rows, false)?;
                    if rows.is_empty() {
                        break;
                    }
                }
                rows
            };
        // rows keep the hidden isomorphism slots after the variables
        self.apply_filters(rows, true)
    }

    // @tg: implements:: [[openspec:native-query-execution#Late decoding]] {since: "2026-10-27"}
    /// One join step: extends `rows` by pattern `i`.
    fn step(&mut self, i: usize, rows: Rows) -> Result<Rows> {
        if rows.is_empty() {
            return Ok(rows);
        }
        let bound: Vec<bool> = rows.row(0).iter().map(|v| *v != UNBOUND).collect();
        let probe = self.pats[i].estimate(&bound);
        let card = self.pats[i].card.max(1) as f64;
        let shared: Vec<usize> = self.pats[i]
            .slots()
            .into_iter()
            .chain(self.pats[i].iso_eid)
            .filter(|s| bound[*s])
            .collect();
        // a hash join reads the pattern once; a nested loop pays a seek per row
        let use_hash = !shared.is_empty()
            && (rows.len() as f64) * (1.0 + probe) > 4.0 * card
            && rows.len() > 64;
        let w = rows.w;
        let mut out = Rows::new(w);
        let par = Par {
            store: self.ex.store.as_sync(),
            threshold: self.ex.parallel_threshold,
        };
        if use_hash {
            self.plan.push(format!(
                "hash join {} ({} rows built, {} probes)",
                self.describe(i, &[]),
                self.pats[i].card,
                rows.len()
            ));
            // the build side sorted by the hash of its shared slots, each hash
            // mapped to its run; `bind` rejects the rows of a colliding hash
            let all = self.scan(i, None)?;
            let mut build: Vec<(u64, StmtRow)> = Vec::with_capacity(all.len());
            let mut probe_row = vec![UNBOUND; self.nslots];
            for r in all {
                if !self.canonical_ok(i, &r)? {
                    continue;
                }
                probe_row.fill(UNBOUND);
                if !bind(&self.pats[i], &r, &mut probe_row) {
                    continue;
                }
                build.push((hash_slots(&shared, &probe_row), r));
            }
            build.sort_unstable_by_key(|(h, _)| *h);
            let mut table: FxHashMap<u64, (usize, usize)> = FxHashMap::default();
            let mut a = 0;
            while a < build.len() {
                let h = build[a].0;
                let mut b = a + 1;
                while b < build.len() && build[b].0 == h {
                    b += 1;
                }
                table.insert(h, (a, b));
                a = b;
            }
            let pats = &self.pats;
            let probe = |chunk: &[i64]| -> Result<Rows> {
                let mut out = Rows::new(w);
                for (n, row) in chunk.chunks_exact(w).enumerate() {
                    if n % 1024 == 0 {
                        budget::check()?;
                    }
                    if let Some(&(a, b)) = table.get(&hash_slots(&shared, row)) {
                        for (_, r) in &build[a..b] {
                            budget::poll()?;
                            let next = out.push_tail(row);
                            if !(bind(&pats[i], r, next) && iso_ok_in(pats, i, next)) {
                                out.pop();
                            }
                        }
                    }
                }
                Ok(out)
            };
            // shared slots exist, so rows are at least one slot wide
            match par.run(&rows, &|_, chunk| probe(chunk))? {
                Some(rows) => {
                    self.plan
                        .push(format!("  in parallel morsels of {MORSEL} rows"));
                    out = rows;
                }
                // sequential: the store is not shared with other threads
                None => out = probe(&rows.data)?,
            }
        } else {
            self.plan.push(format!(
                "index nested loop {} (estimate {:.1} per row, {} rows)",
                self.describe(i, &bound),
                probe,
                rows.len()
            ));
            let (pats, pat) = (&self.pats, &self.pats[i]);
            if !pat.canonical {
                let probe_rows = |store: &(dyn ReadStore + Sync), chunk: &[i64]| {
                    let mut out = Rows::new(w);
                    for row in chunk.chunks_exact(w) {
                        for r in pat.view.scan(store, pat.pattern(Some(row)))? {
                            budget::poll()?;
                            let r = r?;
                            let next = out.push_tail(row);
                            if !(bind(pat, &r, next) && iso_ok_in(pats, i, next)) {
                                out.pop();
                            }
                        }
                    }
                    Ok(out)
                };
                if w > 0 {
                    if let Some(rows) = par.run(&rows, &probe_rows)? {
                        self.plan
                            .push(format!("  in parallel morsels of {MORSEL} rows"));
                        return Ok(rows);
                    }
                }
                for (n, row) in rows.iter().enumerate() {
                    if n % 1024 == 0 {
                        budget::check()?;
                    }
                    for r in pat.view.scan(self.ex.store, pat.pattern(Some(row)))? {
                        budget::poll()?;
                        let r = r?;
                        let next = out.push_tail(row);
                        if !(bind(pat, &r, next) && iso_ok_in(pats, i, next)) {
                            out.pop();
                        }
                    }
                }
                return Ok(out);
            }
            for (n, row) in rows.iter().enumerate() {
                if n % 1024 == 0 {
                    budget::check()?;
                }
                for r in self.scan(i, Some(row))? {
                    budget::poll()?;
                    if !self.canonical_ok(i, &r)? {
                        continue;
                    }
                    let next = out.push_tail(row);
                    if !(bind(&self.pats[i], &r, next) && iso_ok_in(&self.pats, i, next)) {
                        out.pop();
                    }
                }
            }
        }
        Ok(out)
    }

    /// Pattern `i` for explain, with the index a probe reads when the `bound`
    /// slots are bound.
    fn describe(&self, i: usize, bound: &[bool]) -> String {
        let p = &self.pats[i];
        let name = |pos: Pos| match pos {
            Pos::Const(c) => {
                let id = ObjectId::from_raw(c);
                let iri = (id.tag_bits() == tm_core::Tag::Iri as u8)
                    .then(|| {
                        tm_store::dict::load_term(self.ex.store, id.unsigned_payload() as i64)
                            .ok()
                            .flatten()
                    })
                    .flatten();
                iri.map_or_else(|| id.to_string(), |t| format!("<{}>", t.lex))
            }
            Pos::Slot(s) => format!("?{}", self.vars.get(s).map_or("_", |v| v.name())),
        };
        let access = self
            .ex
            .store
            .access_path(probe_pattern(p, bound), p.view.scope())
            .map_or_else(String::new, |a| format!(" {a} scan"));
        format!(
            "({} {} {}) [{:?}]{access}",
            name(p.pos[0]),
            name(p.pos[1]),
            name(p.pos[2]),
            p.view.tx
        )
    }

    // @tg: implements:: [[openspec:native-query-execution#Worst-case-optimal cyclic joins]] {since: "2026-10-27"}
    /// The worst-case-optimal join of a cyclic pattern set.
    fn generic_join(&mut self) -> Result<Rows> {
        // variable order: most patterns first, then the most selective
        let mut order: Vec<usize> = (0..self.nslots).collect();
        let uses = |s: usize, pats: &[Pat]| {
            pats.iter()
                .filter(|p| p.slots().contains(&s) || p.iso_eid == Some(s))
                .count()
        };
        order.retain(|s| uses(*s, &self.pats) > 0);
        order.sort_by_key(|s| std::cmp::Reverse(uses(*s, &self.pats)));
        // one trie per pattern over its slots in the global order; leaves are rows
        let mut tries: Vec<(Vec<usize>, Trie)> = Vec::new();
        for i in 0..self.pats.len() {
            let mut slots: Vec<usize> = self.pats[i].slots();
            if let Some(h) = self.pats[i].iso_eid {
                slots.push(h);
            }
            slots.sort_by_key(|s| order.iter().position(|o| o == s).unwrap_or(usize::MAX));
            slots.dedup();
            let mut trie = Trie::default();
            for r in self.scan(i, None)? {
                if !self.canonical_ok(i, &r)? {
                    continue;
                }
                let mut row = vec![UNBOUND; self.nslots];
                if !bind(&self.pats[i], &r, &mut row) {
                    continue;
                }
                let key: Vec<i64> = slots.iter().map(|s| row[*s]).collect();
                trie.insert(&key);
            }
            self.plan.push(format!(
                "generic join trie {} over {:?} ({} statements)",
                self.describe(i, &[]),
                slots
                    .iter()
                    .map(|s| self.vars.get(*s).map_or("_", |v| v.name()))
                    .collect::<Vec<_>>(),
                self.pats[i].card
            ));
            tries.push((slots, trie));
        }
        let mut out = Rows::new(self.nslots);
        let mut row = vec![UNBOUND; self.nslots];
        let mut cursors: Vec<&Trie> = tries.iter().map(|(_, t)| t).collect();
        let mut depth: Vec<usize> = vec![0; tries.len()];
        let mut visited = 0u64;
        self.descend(
            &order,
            0,
            &tries,
            &mut cursors,
            &mut depth,
            &mut row,
            &mut out,
            &mut visited,
        )?;
        self.plan.push(format!(
            "generic join visited {visited} keys, {} rows",
            out.len()
        ));
        Ok(out)
    }

    #[allow(clippy::too_many_arguments)]
    fn descend<'t>(
        &self,
        order: &[usize],
        k: usize,
        tries: &'t [(Vec<usize>, Trie)],
        cursors: &mut Vec<&'t Trie>,
        depth: &mut Vec<usize>,
        row: &mut Vec<i64>,
        out: &mut Rows,
        visited: &mut u64,
    ) -> Result<()> {
        if k == order.len() {
            // every pattern has bound all its slots: check isomorphism, then emit
            // one row per combination of statement multiplicities
            let mult: u64 = cursors.iter().map(|c| c.count.max(1)).product();
            if (0..self.pats.len()).all(|i| self.iso_ok(i, row)) {
                for _ in 0..mult {
                    budget::poll()?;
                    out.push(row);
                }
            }
            budget::poll()?;
            return Ok(());
        }
        let s = order[k];
        let parts: Vec<usize> = (0..tries.len())
            .filter(|i| tries[*i].0.get(depth[*i]) == Some(&s))
            .collect();
        if parts.is_empty() {
            return self.descend(order, k + 1, tries, cursors, depth, row, out, visited);
        }
        // iterate the smallest level, probe the others
        let smallest = *parts
            .iter()
            .min_by_key(|i| cursors[**i].children.len())
            .expect("a participant");
        let level: &'t Trie = cursors[smallest];
        *visited += level.children.len() as u64;
        // the cursors of this level, restored after each value
        let saved: Vec<&'t Trie> = parts.iter().map(|i| cursors[*i]).collect();
        let mut next: Vec<&'t Trie> = Vec::with_capacity(parts.len());
        for &v in level.children.keys() {
            next.clear();
            for c in &saved {
                match c.children.get(&v) {
                    Some(c) => next.push(c),
                    None => break,
                }
            }
            if next.len() < parts.len() {
                continue;
            }
            for (i, c) in parts.iter().zip(&next) {
                cursors[*i] = *c;
                depth[*i] += 1;
            }
            row[s] = v;
            self.descend(order, k + 1, tries, cursors, depth, row, out, visited)?;
            row[s] = UNBOUND;
            for (i, c) in parts.iter().zip(&saved) {
                cursors[*i] = *c;
                depth[*i] -= 1;
            }
        }
        Ok(())
    }
}

/// A hash trie: one level per variable, leaf counts for bag multiplicity.
#[derive(Debug, Default)]
struct Trie {
    children: FxHashMap<i64, Trie>,
    count: u64,
}

impl Trie {
    fn insert(&mut self, key: &[i64]) {
        match key.split_first() {
            None => self.count += 1,
            Some((k, rest)) => self.children.entry(*k).or_default().insert(rest),
        }
    }
}

/// Evaluates triple patterns into a relation of terms, optionally from `seed`.
pub fn eval(
    ex: &Exec<'_>,
    pats: &[&PTriple],
    seed: Option<(&[Var], Vec<Vec<i64>>)>,
    filters: &[PExpr],
) -> Result<(Rel, Vec<String>)> {
    let (seed_vars, seed_rows) = match seed {
        Some((v, r)) => (v.to_vec(), r),
        None => (Vec::new(), vec![Vec::new()]),
    };
    let mut bgp = Bgp::new(ex, pats, &seed_vars)?;
    bgp.set_filters(filters);
    let rows = bgp.run(seed_rows)?;
    let vars = bgp.vars().to_vec();
    let vars_len = vars.len();
    let mut rel = Rel {
        cols: vars.iter().map(|_| Col::term(false)).collect(),
        vars,
        rows: rows
            .iter()
            .map(|r| r[..vars_len].iter().map(|x| V::Integer(*x)).collect())
            .collect(),
        iso: Vec::new(),
    };
    if bgp.iso {
        for (i, t) in pats.iter().enumerate() {
            if let (Some(g), Some(e)) = (t.iso_group, &t.eid) {
                if let Some(col) = rel.idx(e) {
                    rel.iso.push(Iso {
                        group: g,
                        col,
                        pred: match t.p {
                            PTerm::Id(id) => Some(id),
                            PTerm::Var(_) => None,
                        },
                        optional: false,
                    });
                }
            }
            let _ = i;
        }
    }
    Ok((rel, bgp.plan))
}

/// Keeps `ReadStore` in scope for the trait methods used above.
#[allow(dead_code)]
fn _store(_: &dyn ReadStore) {}

/// The shape of the pattern a probe of `p` scans when the `bound` slots are
/// bound (their values do not matter, only which positions are fixed).
fn probe_pattern(p: &Pat, bound: &[bool]) -> Pattern {
    let get = |pos: Pos| match pos {
        Pos::Const(c) => Some(c),
        Pos::Slot(s) => bound.get(s).copied().unwrap_or(false).then_some(0),
    };
    Pattern::new(get(p.pos[0]), get(p.pos[1]), get(p.pos[2]))
}

/// Whether `row` keeps the eids of pattern `i`'s match group distinct.
fn iso_ok_in(pats: &[Pat], i: usize, row: &[i64]) -> bool {
    let p = &pats[i];
    let (Some(g), Some(me)) = (p.iso_group, p.eid.or(p.iso_eid)) else {
        return true;
    };
    for (j, q) in pats.iter().enumerate() {
        if j == i || q.iso_group != Some(g) {
            continue;
        }
        if let (Some(a), Some(b)) = (p.pred, q.pred) {
            if a != b {
                continue;
            }
        }
        if let Some(other) = q.eid.or(q.iso_eid) {
            if row[other] != UNBOUND && row[other] == row[me] && j != i {
                return false;
            }
        }
    }
    true
}

/// Rows per morsel of a parallel join step.
const MORSEL: usize = 1024;

/// Parallel morsels for one join step, when the store can be shared.
struct Par<'s> {
    store: Option<&'s (dyn ReadStore + Sync)>,
    threshold: usize,
}

/// The output of one morsel, once a worker has run it.
type Slot = Mutex<Option<Result<Rows>>>;

/// The work of one morsel: its rows, flat, extended into new rows.
type MorselWork<'w> = dyn Fn(&(dyn ReadStore + Sync), &[i64]) -> Result<Rows> + Sync + 'w;

// @tg: implements:: [[openspec:native-query-execution#Parallel scans]] {since: "2026-10-27", confidence: partial, note: "join steps only; scans and hash builds stay sequential"}
impl Par<'_> {
    /// Runs `work` over morsels of `rows` on several threads and concatenates
    /// the outputs in input order; `None` when the step should stay sequential
    /// (no shareable store, below the threshold, or one core). Each worker sees
    /// the caller's deadline and cancellation; the first error in morsel order
    /// is returned and stops the others.
    fn run(&self, rows: &Rows, work: &MorselWork<'_>) -> Result<Option<Rows>> {
        let Some(store) = self.store else {
            return Ok(None);
        };
        if rows.w == 0 || self.threshold == 0 || rows.len() < self.threshold.max(2 * MORSEL) {
            return Ok(None);
        }
        let n = rows.len().div_ceil(MORSEL);
        let threads = std::thread::available_parallelism()
            .map_or(1, |t| t.get())
            .min(n);
        if threads < 2 {
            return Ok(None);
        }
        let next = AtomicUsize::new(0);
        let failed = AtomicBool::new(false);
        let results: Vec<Slot> = (0..n).map(|_| Mutex::new(None)).collect();
        let interrupt = budget::interrupt();
        std::thread::scope(|sc| {
            for _ in 0..threads {
                sc.spawn(|| {
                    let drain = || loop {
                        if failed.load(AtomicOrdering::Relaxed) {
                            break;
                        }
                        let k = next.fetch_add(1, AtomicOrdering::Relaxed);
                        if k >= n {
                            break;
                        }
                        let chunk = &rows.data
                            [k * MORSEL * rows.w..((k + 1) * MORSEL).min(rows.len()) * rows.w];
                        let r = work(store, chunk);
                        if r.is_err() {
                            failed.store(true, AtomicOrdering::Relaxed);
                        }
                        *results[k].lock().unwrap_or_else(|p| p.into_inner()) = Some(r);
                    };
                    match &interrupt {
                        Some(it) => budget::scope(budget::Meter::new(it.clone()), drain),
                        None => drain(),
                    }
                });
            }
        });
        let mut out = Rows::new(rows.w);
        for r in results {
            match r.into_inner().unwrap_or_else(|p| p.into_inner()) {
                Some(Ok(rows)) => out.extend(rows),
                Some(Err(e)) => return Err(e),
                None => {}
            }
        }
        budget::check()?;
        Ok(Some(out))
    }
}
