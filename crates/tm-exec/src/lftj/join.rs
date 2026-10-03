//! Leapfrog triejoin (Veldhuizen 2014) over sorted, in-memory access paths.
//!
//! Each pattern is a relation over its variables, sorted in the global variable
//! order, so it is a trie: depth `d` holds the values of its `d`-th variable. The
//! join binds one variable at a time; at each one the iterators of the patterns
//! that contain it leapfrog over their keys with galloping seeks, so the work per
//! variable is bounded by the smallest participant, not the product.

use tm_core::{budget, Result};

/// One access path: `rows × cols.len()` values, sorted lexicographically.
#[derive(Clone, Debug, Default)]
pub struct Relation {
    /// The global variables of the columns, ascending.
    pub cols: Vec<usize>,
    /// Row-major values.
    pub data: Vec<i64>,
}

impl Relation {
    fn rows(&self) -> usize {
        if self.cols.is_empty() {
            0
        } else {
            self.data.len() / self.cols.len()
        }
    }

    fn at(&self, row: usize, depth: usize) -> i64 {
        self.data[row * self.cols.len() + depth]
    }

    /// First row in `[lo, hi)` whose value at `depth` satisfies `pred`, for a
    /// predicate that is monotone over the range (exponential, then binary search).
    fn gallop(&self, lo: usize, hi: usize, depth: usize, pred: impl Fn(i64) -> bool) -> usize {
        if lo >= hi || pred(self.at(lo, depth)) {
            return lo;
        }
        let (mut prev, mut step) = (lo, 1usize);
        let bound = loop {
            let cur = prev.saturating_add(step);
            if cur >= hi {
                break hi;
            }
            if pred(self.at(cur, depth)) {
                break cur;
            }
            prev = cur;
            step = step.saturating_mul(2);
        };
        let (mut a, mut b) = (prev + 1, bound);
        while a < b {
            let m = a + (b - a) / 2;
            if pred(self.at(m, depth)) {
                b = m;
            } else {
                a = m + 1;
            }
        }
        a
    }
}

/// The open range of one trie level: the current row and the end of the range.
#[derive(Copy, Clone, Debug)]
struct Level {
    pos: usize,
    end: usize,
}

/// A trie iterator: one [`Level`] per opened depth.
#[derive(Clone, Debug, Default)]
struct TrieIter {
    levels: Vec<Level>,
}

impl TrieIter {
    fn depth(&self) -> usize {
        self.levels.len() - 1
    }

    fn top(&self) -> Level {
        self.levels[self.levels.len() - 1]
    }

    fn open(&mut self, r: &Relation) {
        let lvl = match self.levels.last() {
            None => Level {
                pos: 0,
                end: r.rows(),
            },
            Some(l) => {
                // the rows that share the current key of the parent level
                let d = self.levels.len() - 1;
                let key = r.at(l.pos, d);
                Level {
                    pos: l.pos,
                    end: r.gallop(l.pos, l.end, d, |x| x > key),
                }
            }
        };
        self.levels.push(lvl);
    }

    fn up(&mut self) {
        self.levels.pop();
    }

    fn at_end(&self) -> bool {
        let l = self.top();
        l.pos >= l.end
    }

    fn key(&self, r: &Relation) -> i64 {
        r.at(self.top().pos, self.depth())
    }

    fn next(&mut self, r: &Relation) {
        let key = self.key(r);
        self.seek_where(r, |x| x > key);
    }

    fn seek(&mut self, r: &Relation, target: i64) {
        self.seek_where(r, |x| x >= target);
    }

    fn seek_where(&mut self, r: &Relation, pred: impl Fn(i64) -> bool) {
        let d = self.depth();
        let n = self.levels.len() - 1;
        let l = self.levels[n];
        self.levels[n].pos = r.gallop(l.pos, l.end, d, pred);
    }
}

/// The join of `rels` over `vars` variables. `emit` receives every binding (one
/// value per variable, indexed by variable); it returns `Err` to stop. The
/// operation budget is polled while the join runs.
pub fn leapfrog(
    vars: usize,
    rels: &[Relation],
    emit: &mut dyn FnMut(&[i64]) -> Result<()>,
) -> Result<()> {
    let mut by_var: Vec<Vec<usize>> = vec![Vec::new(); vars];
    for (i, r) in rels.iter().enumerate() {
        for &v in &r.cols {
            by_var[v].push(i);
        }
    }
    if by_var.iter().any(Vec::is_empty) || rels.iter().any(|r| r.rows() == 0) {
        return Ok(());
    }
    let mut j = Join {
        rels,
        iters: vec![TrieIter::default(); rels.len()],
        by_var,
        bound: vec![0; vars],
        emit,
    };
    j.var(0)
}

struct Join<'a, 'e> {
    rels: &'a [Relation],
    iters: Vec<TrieIter>,
    by_var: Vec<Vec<usize>>,
    bound: Vec<i64>,
    emit: &'e mut dyn FnMut(&[i64]) -> Result<()>,
}

impl Join<'_, '_> {
    fn var(&mut self, v: usize) -> Result<()> {
        if v == self.by_var.len() {
            return (self.emit)(&self.bound);
        }
        let parts = self.by_var[v].clone();
        for &p in &parts {
            self.iters[p].open(&self.rels[p]);
        }
        let r = self.intersect(v, &parts);
        for &p in &parts {
            self.iters[p].up();
        }
        r
    }

    /// Leapfrog over the iterators of the patterns containing variable `v`.
    fn intersect(&mut self, v: usize, parts: &[usize]) -> Result<()> {
        if parts.iter().any(|&p| self.iters[p].at_end()) {
            return Ok(());
        }
        let mut order = parts.to_vec();
        order.sort_by_key(|&p| self.iters[p].key(&self.rels[p]));
        let m = order.len();
        let mut i = 0;
        let mut max = self.iters[order[m - 1]].key(&self.rels[order[m - 1]]);
        loop {
            budget::poll()?;
            let p = order[i];
            let x = self.iters[p].key(&self.rels[p]);
            if x == max {
                // every iterator is at `x`
                self.bound[v] = x;
                self.var(v + 1)?;
                self.iters[p].next(&self.rels[p]);
            } else {
                self.iters[p].seek(&self.rels[p], max);
            }
            if self.iters[p].at_end() {
                return Ok(());
            }
            max = self.iters[p].key(&self.rels[p]);
            i = (i + 1) % m;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(cols: &[usize], mut rows: Vec<Vec<i64>>) -> Relation {
        rows.sort();
        Relation {
            cols: cols.to_vec(),
            data: rows.into_iter().flatten().collect(),
        }
    }

    fn run(vars: usize, rels: &[Relation]) -> Vec<Vec<i64>> {
        let mut out = Vec::new();
        leapfrog(vars, rels, &mut |b| {
            out.push(b.to_vec());
            Ok(())
        })
        .unwrap();
        out.sort();
        out
    }

    #[test]
    fn triangles_match_brute_force() {
        // edges over 0..12, three relations (a,b), (b,c), (a,c) = c→a reversed
        let mut edges = Vec::new();
        for i in 0..12i64 {
            for k in [1, 2, 5, 7] {
                edges.push((i, (i * 3 + k) % 12));
            }
        }
        edges.sort();
        edges.dedup();
        let ab = rel(&[0, 1], edges.iter().map(|&(a, b)| vec![a, b]).collect());
        let bc = rel(&[1, 2], edges.iter().map(|&(a, b)| vec![a, b]).collect());
        let ac = rel(&[0, 2], edges.iter().map(|&(c, a)| vec![a, c]).collect());
        let got = run(3, &[ab, bc, ac]);
        let mut want = Vec::new();
        for &(a, b) in &edges {
            for &(b2, c) in &edges {
                if b2 == b && edges.contains(&(c, a)) {
                    want.push(vec![a, b, c]);
                }
            }
        }
        want.sort();
        assert!(!want.is_empty());
        assert_eq!(got, want);
    }

    #[test]
    fn empty_and_single() {
        let a = rel(&[0], vec![vec![1], vec![3], vec![5]]);
        let b = rel(&[0], vec![vec![2], vec![3], vec![4], vec![5]]);
        assert_eq!(run(1, &[a.clone(), b]), vec![vec![3], vec![5]]);
        assert_eq!(
            run(1, &[a.clone(), rel(&[0], vec![])]),
            Vec::<Vec<i64>>::new()
        );
        assert_eq!(run(1, &[a]).len(), 3);
    }
}
