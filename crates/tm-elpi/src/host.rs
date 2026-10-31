//! The host builtins `tm.triple`, `tm.added`, `tm.retracted` and `tm.valid`
//! over a `ReadStore` (`lat.md/elpi#Host Callback Builtins`). The engine calls
//! [`Host::call`] for each call of a declared predicate; every eid a builtin
//! returns joins the run's seen set, which support is checked against.

use std::collections::{BTreeMap, HashSet};

use tm_core::store::{Pattern, ReadStore, StmtRow};
use tm_core::{Eid, ObjectId, Valid};
use tm_store::dict::{self, TermCache};
use tm_store::read::View;

use crate::term::{bounds, eid, from_value, to_eid, to_value, Tree};
use crate::valid::covers;

/// Which statements a run reads, by valid time.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Reading {
    /// Those valid at an instant.
    Instant(i64),
    /// Every statement of the view.
    All,
    /// Those valid throughout a segment.
    Segment(Valid),
}

impl Reading {
    fn admits(&self, r: &StmtRow) -> bool {
        let v = Valid {
            from: r.v_from,
            to: r.v_to,
        };
        match self {
            Reading::Instant(d) => v.contains(*d),
            Reading::All => true,
            Reading::Segment(seg) => covers(&v, seg),
        }
    }
}

/// A transaction's changes, for `tm.added` and `tm.retracted`.
#[derive(Clone, Debug, Default)]
pub struct Seeds {
    /// Statements the transaction added.
    pub added: Vec<StmtRow>,
    /// Statements the transaction retracted.
    pub retracted: Vec<StmtRow>,
}

/// The state of one run's builtins.
pub struct Host<'a> {
    store: &'a dyn ReadStore,
    view: View,
    reading: Reading,
    seeds: Option<&'a Seeds>,
    exclude: &'a HashSet<i64>,
    terms: TermCache,
    /// Every statement a builtin returned, with its interval.
    pub seen: BTreeMap<Eid, Valid>,
    /// Rows returned so far.
    pub rows: u64,
    max_rows: u64,
    /// True once the row budget ran out.
    pub over_budget: bool,
}

/// The ELPI signatures of the host builtins.
pub const SIGNATURES: [(&str, &str); 4] = [
    (
        "tm.triple",
        "pred tm.triple o:term, o:term, o:term, o:stmt.",
    ),
    ("tm.added", "pred tm.added o:term, o:term, o:term, o:stmt."),
    (
        "tm.retracted",
        "pred tm.retracted o:term, o:term, o:term, o:stmt.",
    ),
    ("tm.valid", "pred tm.valid o:stmt, o:bound, o:bound."),
];

fn valid_of(r: &StmtRow) -> Valid {
    Valid {
        from: r.v_from,
        to: r.v_to,
    }
}

impl<'a> Host<'a> {
    /// Builtins reading `view` of `store` by `reading`; `exclude` hides raw
    /// eids (a program's own conclusions), `seeds` serve `tm.added` and
    /// `tm.retracted`.
    pub fn new(
        store: &'a dyn ReadStore,
        view: View,
        reading: Reading,
        seeds: Option<&'a Seeds>,
        exclude: &'a HashSet<i64>,
        max_rows: u64,
    ) -> Host<'a> {
        Host {
            store,
            view,
            reading,
            seeds,
            exclude,
            terms: TermCache::new(4096),
            seen: BTreeMap::new(),
            rows: 0,
            max_rows,
            over_budget: false,
        }
    }

    /// One call of builtin `name`: a row of terms per alternative, one per
    /// argument.
    pub fn call(&mut self, name: &str, args: &[Tree]) -> Result<Vec<Vec<Tree>>, String> {
        let r = match name {
            "tm.triple" => self.triple(args),
            "tm.added" => self.seeded(args, true),
            "tm.retracted" => self.seeded(args, false),
            "tm.valid" => self.valid(args),
            other => Err(format!("unknown host builtin {other}")),
        };
        r.map_err(|e| e.to_string())
    }

    fn count(&mut self, n: usize) -> Result<(), String> {
        self.rows += n as u64;
        if self.rows > self.max_rows {
            self.over_budget = true;
            return Err(format!("row budget of {} exceeded", self.max_rows));
        }
        Ok(())
    }

    /// `Ok(None)`: the argument is free; `Ok(Some(None))`: a value with no
    /// dictionary id, so nothing matches.
    fn bound(&self, t: &Tree) -> Result<Option<Option<i64>>, String> {
        match to_value(t) {
            None => Ok(None),
            Some(v) => dict::encode(self.store, &v)
                .map(|id| Some(id.map(ObjectId::raw)))
                .map_err(|e| e.to_string()),
        }
    }

    fn row(&mut self, r: &StmtRow) -> Result<Option<Vec<Tree>>, String> {
        let Some(e) = Eid::from_oid(ObjectId::from_raw(r.eid)) else {
            return Ok(None);
        };
        let dec = |id: i64, h: &Host| {
            h.terms
                .decode(h.store, ObjectId::from_raw(id), true)
                .map(|v| from_value(&v))
                .map_err(|e| e.to_string())
        };
        let row = vec![dec(r.s, self)?, dec(r.p, self)?, dec(r.o, self)?, eid(e)];
        self.seen.insert(e, valid_of(r));
        Ok(Some(row))
    }

    fn triple(&mut self, args: &[Tree]) -> Result<Vec<Vec<Tree>>, String> {
        if args.len() != 4 {
            return Err("tm.triple takes four arguments".into());
        }
        let rows: Vec<StmtRow> = if let Some(e) = to_eid(&args[3]) {
            match self
                .store
                .statement(e.oid().raw())
                .map_err(|e| e.to_string())?
            {
                Some(r) => vec![r],
                None => Vec::new(),
            }
        } else {
            let mut pat = [None; 3];
            for (i, slot) in pat.iter_mut().enumerate() {
                match self.bound(&args[i])? {
                    None => {}
                    Some(None) => return Ok(Vec::new()),
                    Some(Some(id)) => *slot = Some(id),
                }
            }
            let mut out = Vec::new();
            let it = self
                .view
                .scan(self.store, Pattern::new(pat[0], pat[1], pat[2]))
                .map_err(|e| e.to_string())?;
            for r in it {
                let r = r.map_err(|e| e.to_string())?;
                if self.reading.admits(&r) && !self.exclude.contains(&r.eid) {
                    out.push(r);
                }
            }
            out
        };
        let rows: Vec<StmtRow> = rows
            .into_iter()
            .filter(|r| {
                self.view.selects(r) && self.reading.admits(r) && !self.exclude.contains(&r.eid)
            })
            .collect();
        self.count(rows.len())?;
        let mut out = Vec::with_capacity(rows.len());
        for r in &rows {
            if let Some(row) = self.row(r)? {
                out.push(row);
            }
        }
        Ok(out)
    }

    fn seeded(&mut self, args: &[Tree], added: bool) -> Result<Vec<Vec<Tree>>, String> {
        if args.len() != 4 {
            return Err("tm.added and tm.retracted take four arguments".into());
        }
        let Some(seeds) = self.seeds else {
            return Ok(Vec::new());
        };
        let mut want = [None; 3];
        for (i, slot) in want.iter_mut().enumerate() {
            match self.bound(&args[i])? {
                None => {}
                Some(None) => return Ok(Vec::new()),
                Some(Some(id)) => *slot = Some(id),
            }
        }
        let e = to_eid(&args[3]).map(|e| e.oid().raw());
        let src = if added {
            &seeds.added
        } else {
            &seeds.retracted
        };
        let rows: Vec<StmtRow> = src
            .iter()
            .filter(|r| {
                want[0].is_none_or(|x| x == r.s)
                    && want[1].is_none_or(|x| x == r.p)
                    && want[2].is_none_or(|x| x == r.o)
                    && e.is_none_or(|x| x == r.eid)
            })
            .copied()
            .collect();
        self.count(rows.len())?;
        let mut out = Vec::with_capacity(rows.len());
        for r in &rows {
            if let Some(row) = self.row(r)? {
                out.push(row);
            }
        }
        Ok(out)
    }

    fn valid(&mut self, args: &[Tree]) -> Result<Vec<Vec<Tree>>, String> {
        let Some(e) = args.first().and_then(to_eid) else {
            return Err("tm.valid needs a bound statement (eid N)".into());
        };
        let v = match self.seen.get(&e) {
            Some(v) => *v,
            None => match self
                .store
                .statement(e.oid().raw())
                .map_err(|e| e.to_string())?
            {
                Some(r) if self.view.selects(&r) => valid_of(&r),
                _ => return Ok(Vec::new()),
            },
        };
        self.count(1)?;
        let (from, to) = bounds(v);
        Ok(vec![vec![eid(e), from, to]])
    }
}
