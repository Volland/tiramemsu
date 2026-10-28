//! Interval sets over the integer timeline and the DatalogMTL operators on them
//! (`lat.md/datalog#Interval Sets Through Joins`, `#DatalogMTL Horn Fragment`).
//!
//! A set is a sorted list of disjoint, non-adjacent half-open intervals
//! `[from, to)` of epoch milliseconds; `i64::MIN` as a start and `i64::MAX` as
//! an end are open ends. Arithmetic saturates, so an open end stays open.

use smallvec::SmallVec;
use tm_core::Valid;

use crate::syntax::{TempOp, Window};

/// Open start.
pub const NEG_INF: i64 = i64::MIN;
/// Open end.
pub const POS_INF: i64 = i64::MAX;

/// A coalesced set of half-open intervals.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
// @tg: implements:: [[openspec:datalog-temporal#Pointwise valid time with intervals]] {since: "2026-10-28"}
pub struct IvSet(SmallVec<[(i64, i64); 1]>);

fn add(x: i64, d: i64) -> i64 {
    if x == NEG_INF || x == POS_INF {
        x
    } else {
        x.saturating_add(d).clamp(NEG_INF + 1, POS_INF - 1)
    }
}

fn sub(x: i64, d: i64) -> i64 {
    if x == NEG_INF || x == POS_INF {
        x
    } else {
        x.saturating_sub(d).clamp(NEG_INF + 1, POS_INF - 1)
    }
}

/// `x + d` where `d` may be `inf` (`None`): an infinite shift opens the end.
fn add_w(x: i64, d: Option<i64>) -> i64 {
    match d {
        Some(d) => add(x, d),
        None if x == NEG_INF => NEG_INF,
        None => POS_INF,
    }
}

/// `x − d` where `d` may be `inf`.
fn sub_w(x: i64, d: Option<i64>) -> i64 {
    match d {
        Some(d) => sub(x, d),
        None if x == POS_INF => POS_INF,
        None => NEG_INF,
    }
}

impl IvSet {
    /// The empty set.
    pub fn empty() -> IvSet {
        IvSet(SmallVec::new())
    }

    /// Every instant.
    pub fn all() -> IvSet {
        IvSet(smallvec::smallvec![(NEG_INF, POS_INF)])
    }

    /// One interval; empty when `from >= to`.
    pub fn one(from: i64, to: i64) -> IvSet {
        if from < to {
            IvSet(smallvec::smallvec![(from, to)])
        } else {
            IvSet::empty()
        }
    }

    /// The set of a stored valid interval.
    pub fn of_valid(v: Valid) -> IvSet {
        IvSet::one(v.from.unwrap_or(NEG_INF), v.to.unwrap_or(POS_INF))
    }

    /// From any intervals (sorted and coalesced here).
    pub fn from_intervals(mut v: Vec<(i64, i64)>) -> IvSet {
        v.retain(|(a, b)| a < b);
        v.sort_unstable();
        let mut out: SmallVec<[(i64, i64); 1]> = SmallVec::new();
        for (a, b) in v {
            match out.last_mut() {
                Some(last) if a <= last.1 => last.1 = last.1.max(b),
                _ => out.push((a, b)),
            }
        }
        IvSet(out)
    }

    /// True when no instant is in the set.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// True for the set of every instant.
    pub fn is_all(&self) -> bool {
        self.0.len() == 1 && self.0[0] == (NEG_INF, POS_INF)
    }

    /// The intervals, ascending.
    pub fn intervals(&self) -> &[(i64, i64)] {
        &self.0
    }

    /// The intervals as stored valid times.
    pub fn valids(&self) -> Vec<Valid> {
        self.0
            .iter()
            .map(|&(a, b)| Valid {
                from: (a != NEG_INF).then_some(a),
                to: (b != POS_INF).then_some(b),
            })
            .collect()
    }

    /// True when `t` is in the set.
    pub fn contains(&self, t: i64) -> bool {
        self.0.iter().any(|&(a, b)| a <= t && t < b)
    }

    /// True when every instant of `other` is in `self`.
    pub fn covers(&self, other: &IvSet) -> bool {
        other.subtract(self).is_empty()
    }

    /// `self ∩ other`.
    pub fn intersect(&self, other: &IvSet) -> IvSet {
        if self.is_all() {
            return other.clone();
        }
        if other.is_all() {
            return self.clone();
        }
        let (mut i, mut j) = (0, 0);
        let mut out = SmallVec::new();
        while i < self.0.len() && j < other.0.len() {
            let (a1, b1) = self.0[i];
            let (a2, b2) = other.0[j];
            let lo = a1.max(a2);
            let hi = b1.min(b2);
            if lo < hi {
                out.push((lo, hi));
            }
            if b1 < b2 {
                i += 1;
            } else {
                j += 1;
            }
        }
        IvSet(out)
    }

    /// True when the sets share an instant.
    pub fn meets(&self, other: &IvSet) -> bool {
        !self.intersect(other).is_empty()
    }

    /// `self ∪ other`.
    pub fn union(&self, other: &IvSet) -> IvSet {
        if self.is_empty() {
            return other.clone();
        }
        if other.is_empty() {
            return self.clone();
        }
        let mut v: Vec<(i64, i64)> = self.0.iter().chain(other.0.iter()).copied().collect();
        v.sort_unstable();
        let mut out: SmallVec<[(i64, i64); 1]> = SmallVec::new();
        for (a, b) in v {
            match out.last_mut() {
                Some(last) if a <= last.1 => last.1 = last.1.max(b),
                _ => out.push((a, b)),
            }
        }
        IvSet(out)
    }

    /// `self ∖ other`.
    pub fn subtract(&self, other: &IvSet) -> IvSet {
        if other.is_empty() || self.is_empty() {
            return self.clone();
        }
        let mut out = SmallVec::new();
        let mut j = 0;
        for &(a, b) in &self.0 {
            let mut cur = a;
            while j < other.0.len() && other.0[j].1 <= cur {
                j += 1;
            }
            let mut k = j;
            while cur < b {
                match other.0.get(k) {
                    Some(&(c, d)) if c < b => {
                        if c > cur {
                            out.push((cur, c));
                        }
                        cur = cur.max(d);
                        k += 1;
                    }
                    _ => {
                        out.push((cur, b));
                        cur = b;
                    }
                }
            }
        }
        IvSet(out)
    }

    /// The complement within the whole timeline.
    pub fn complement(&self) -> IvSet {
        IvSet::all().subtract(self)
    }

    /// Restricted to `[from, to)`.
    pub fn clip(&self, from: i64, to: i64) -> IvSet {
        self.intersect(&IvSet::one(from, to))
    }

    /// The set after a unary temporal operator in a body.
    // @tg: implements:: [[openspec:datalog-temporal#Metric temporal operators]] {since: "2026-10-28"}
    pub fn apply(&self, op: TempOp, w: Window) -> IvSet {
        let (a, b) = (w.lo, w.hi);
        let mapped: Vec<(i64, i64)> = self
            .0
            .iter()
            .filter_map(|&(f, e)| {
                let (lo, hi) = match op {
                    // ∃s ∈ [t−b, t−a] ∩ [f, e): t ∈ [f+a, e+b)
                    TempOp::Past => (add(f, a), add_w(e, b)),
                    // ∃s ∈ [t+a, t+b] ∩ [f, e): t ∈ [f−b, e−a)
                    TempOp::Future => (sub_w(f, b), sub(e, a)),
                    // [t−b, t−a] ⊆ [f, e): t ∈ [f+b, e+a)
                    TempOp::AlwaysPast => (add_w_box_lo(f, b), add(e, a)),
                    // [t+a, t+b] ⊆ [f, e): t ∈ [f−a, e−b)
                    TempOp::AlwaysFuture => (sub(f, a), sub_w_box_hi(e, b)),
                };
                (lo < hi).then_some((lo, hi))
            })
            .collect();
        IvSet::from_intervals(mapped)
    }

    /// The set a head `op[a, b] H` gives H from the body's set: the window around
    /// every instant of the body.
    // @tg: implements:: [[openspec:datalog-temporal#Always operators in heads]] {since: "2026-10-28"}
    pub fn head(&self, op: TempOp, w: Window) -> IvSet {
        match op {
            // H throughout [t+a, t+b]: H on [f+a, e+b)
            TempOp::AlwaysFuture => self.apply(TempOp::Past, w),
            // H throughout [t−b, t−a]: H on [f−b, e−a)
            TempOp::AlwaysPast => self.apply(TempOp::Future, w),
            // sometime operators in heads are refused by analysis
            TempOp::Past | TempOp::Future => self.clone(),
        }
    }

    /// `A since[a, b] B`: B held at some `s ∈ [t−b, t−a]` and A at every `r` with
    /// `s < r ≤ t`. `self` is A's set, `b` B's.
    pub fn since(&self, bset: &IvSet, w: Window) -> IvSet {
        let a1 = w.lo.max(1);
        let mut out = Vec::new();
        for &(bf, bt) in &bset.0 {
            // t = s needs no A (only when the window starts at 0)
            if w.lo == 0 {
                out.push((bf, bt));
            }
            for &(af, at) in &self.0 {
                // s ∈ B ∩ [af − 1, at − 1)
                let sl = bf.max(sub(af, 1));
                let sh = bt.min(at);
                if sl >= sh {
                    continue;
                }
                let lo = add(sl, a1);
                let hi = match w.hi {
                    Some(h) => add(sub(sh, 1), h).min(sub(at, 1)),
                    None => sub(at, 1),
                };
                // half-open: t ≤ hi
                let hi_open = if hi == POS_INF || at == POS_INF {
                    if w.hi.is_none() || sh == POS_INF {
                        POS_INF
                    } else {
                        add(hi, 1)
                    }
                } else {
                    add(hi, 1)
                };
                if lo < hi_open {
                    out.push((lo, hi_open));
                }
            }
        }
        IvSet::from_intervals(out)
    }

    /// `A until[a, b] B`: B holds at some `s ∈ [t+a, t+b]` and A at every `r` with
    /// `t ≤ r < s`. `self` is A's set, `b` B's.
    pub fn until(&self, bset: &IvSet, w: Window) -> IvSet {
        // mirror the timeline: t ↦ −t maps [f, e) to [1−e, 1−f), and A until B
        // to A' since B' with A required on (s, t]
        let mirror = |s: &IvSet| {
            IvSet::from_intervals(
                s.0.iter()
                    .map(|&(f, e)| {
                        let nf = if e == POS_INF { NEG_INF } else { 1 - e };
                        let ne = if f == NEG_INF { POS_INF } else { 1 - f };
                        (nf, ne)
                    })
                    .collect(),
            )
        };
        mirror(&mirror(self).since(&mirror(bset), w))
    }
}

/// `f + b` for an always operator's lower end: an infinite window needs an open start.
fn add_w_box_lo(f: i64, b: Option<i64>) -> i64 {
    match b {
        Some(b) => add(f, b),
        None if f == NEG_INF => NEG_INF,
        None => POS_INF,
    }
}

/// `e − b` for an always operator's upper end.
fn sub_w_box_hi(e: i64, b: Option<i64>) -> i64 {
    match b {
        Some(b) => sub(e, b),
        None if e == POS_INF => POS_INF,
        None => NEG_INF,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const D: i64 = 86_400_000;

    fn w(lo: i64, hi: Option<i64>) -> Window {
        Window { lo, hi }
    }

    #[test]
    fn algebra() {
        let a = IvSet::from_intervals(vec![(0, 10), (20, 30)]);
        let b = IvSet::one(5, 25);
        assert_eq!(
            a.intersect(&b),
            IvSet::from_intervals(vec![(5, 10), (20, 25)])
        );
        assert_eq!(a.union(&b), IvSet::one(0, 30));
        assert_eq!(
            a.subtract(&b),
            IvSet::from_intervals(vec![(0, 5), (25, 30)])
        );
        assert_eq!(IvSet::one(0, 5).union(&IvSet::one(5, 9)), IvSet::one(0, 9));
        assert_eq!(
            IvSet::all().subtract(&IvSet::one(3, POS_INF)),
            IvSet::one(NEG_INF, 3)
        );
    }

    #[test]
    fn operators() {
        let fever = IvSet::one(10 * D, 11 * D);
        assert_eq!(
            fever.apply(TempOp::Past, w(0, Some(3 * D))),
            IvSet::one(10 * D, 14 * D)
        );
        let emp = IvSet::one(0, 100 * D);
        assert_eq!(
            emp.apply(TempOp::AlwaysPast, w(0, Some(30 * D))),
            IvSet::one(30 * D, 100 * D)
        );
        let booked = IvSet::one(D, 2 * D);
        assert_eq!(
            booked.head(TempOp::AlwaysFuture, w(0, Some(7 * D))),
            IvSet::one(D, 9 * D)
        );
        // since: alarm [5, 6), unacked [5, 20) ⇒ pending [5, 20)
        let unacked = IvSet::one(5, 20);
        let alarm = IvSet::one(5, 6);
        assert_eq!(unacked.since(&alarm, w(0, None)), IvSet::one(5, 20));
        // until mirrors since: unacked [5, 20) until an ack at [20, 21) ⇒ [5, 21)
        let ack = IvSet::one(20, 21);
        assert_eq!(unacked.until(&ack, w(0, None)), IvSet::one(5, 21));
    }

    #[test]
    fn since_matches_brute_force() {
        let sets = [
            IvSet::one(0, 4),
            IvSet::from_intervals(vec![(1, 3), (5, 9)]),
            IvSet::one(2, 12),
            IvSet::from_intervals(vec![(0, 1), (6, 7)]),
        ];
        for aset in &sets {
            for bset in &sets {
                for (lo, hi) in [
                    (0, Some(0)),
                    (0, Some(2)),
                    (1, Some(3)),
                    (2, None),
                    (0, None),
                ] {
                    let win = w(lo, hi);
                    let got = aset.since(bset, win);
                    let got_u = aset.until(bset, win);
                    for t in -2..16 {
                        let want = (t - hi.unwrap_or(100)..=t - lo)
                            .any(|s| bset.contains(s) && (s + 1..=t).all(|r| aset.contains(r)));
                        assert_eq!(
                            got.contains(t),
                            want,
                            "since {aset:?} {bset:?} {win:?} t={t}"
                        );
                        let want_u = (t + lo..=t + hi.unwrap_or(100))
                            .any(|s| bset.contains(s) && (t..s).all(|r| aset.contains(r)));
                        assert_eq!(
                            got_u.contains(t),
                            want_u,
                            "until {aset:?} {bset:?} {win:?} t={t}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn unary_operators_match_brute_force() {
        let sets = [
            IvSet::one(0, 4),
            IvSet::from_intervals(vec![(1, 3), (5, 9)]),
        ];
        for s in &sets {
            for (lo, hi) in [(0, Some(0)), (0, Some(2)), (1, Some(3))] {
                let win = w(lo, hi);
                let hi = hi.unwrap();
                for op in [
                    TempOp::Past,
                    TempOp::Future,
                    TempOp::AlwaysPast,
                    TempOp::AlwaysFuture,
                ] {
                    let got = s.apply(op, win);
                    for t in -6..16 {
                        let want = match op {
                            TempOp::Past => (t - hi..=t - lo).any(|x| s.contains(x)),
                            TempOp::Future => (t + lo..=t + hi).any(|x| s.contains(x)),
                            TempOp::AlwaysPast => (t - hi..=t - lo).all(|x| s.contains(x)),
                            TempOp::AlwaysFuture => (t + lo..=t + hi).all(|x| s.contains(x)),
                        };
                        assert_eq!(got.contains(t), want, "{op:?} {s:?} {win:?} t={t}");
                    }
                }
            }
        }
    }
}
