//! Running a planned query: the operator tree, evaluated bottom-up into
//! materialised relations with the semantics of the SQL route.

use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use rustc_hash::FxHashMap;

use tm_core::store::ReadStore;
use tm_core::text::TextQuery;
use tm_core::{budget, Error, ObjectId, RecallQuery, Result, Value, VectorQuery};
use tm_exec::decode::{CacheMode, Decoder};
use tm_exec::plan::analyze::{Dom, VClass};
use tm_exec::plan::normalize::Planner;
use tm_exec::plan::{Cell, Node, PAgg, PExpr, PKey, PObj, PTerm, PTriple, PValues, PVirtual};
use tm_exec::result::{Explain, QueryResult, RegionInfo, RegionKind, ResultValue, RouteNote};
use tm_exec::scan::{ResolvedTx, ResolvedView};
use tm_exec::{Prepared, QueryEngine};
use tm_ir::{AggFunc, Missing, Semantics, Var};
use tm_store::read::ViewTx;
use tm_store::{dict::TermCache, Pattern, View};

use crate::expr::Val;
use crate::funcs::aggregate;
use crate::rel::{Col, Iso, Rel};
use crate::source::{StoreLoad, StoreSource, Terms};
use crate::value::{self as sv, Key, V};

/// The storage-trait view of a planner view.
pub fn store_view(v: &ResolvedView) -> View {
    View {
        tx: match v.tx {
            ResolvedTx::Now => ViewTx::Now,
            ResolvedTx::AsOf(t) => ViewTx::AsOf(t as i64),
            ResolvedTx::History => ViewTx::History,
        },
        valid_at: match v.valid {
            tm_core::ValidSel::Unfiltered => None,
            tm_core::ValidSel::At(d) => Some(d),
        },
    }
}

/// The evaluator of one query on one snapshot.
pub struct Exec<'a> {
    /// The snapshot.
    pub store: &'a dyn ReadStore,
    /// The semantic flags of the query.
    pub sem: Semantics,
    /// Term rows for value operations.
    pub terms: Terms<'a>,
    /// Plan-local terms by raw id.
    pub synthetic: HashMap<i64, Value>,
    exists: RefCell<HashMap<usize, std::rc::Rc<Rel>>>,
    /// Hash sets of `EXISTS` inner rows by (node, inner columns probed).
    exists_keys: RefCell<HashMap<(usize, Vec<usize>), IdSet>>,
    /// Explain lines, in order of evaluation.
    pub plan: RefCell<Vec<String>>,
    /// The path engine's hop cap.
    pub path_max_hops: u32,
    /// The path engine's state bound.
    pub path_max_states: usize,
    /// Input rows above which a join step runs in parallel morsels (`0`: never).
    pub parallel_threshold: usize,
    /// The Datalog provider of the engine, for goal calls.
    pub datalog: Option<std::sync::Arc<dyn tm_exec::datalog::DatalogProvider>>,
    /// The relations of the goal calls run so far, by spec: a call runs once.
    datalog_calls: RefCell<HashMap<String, std::rc::Rc<Rel>>>,
}

// @tg: implements:: [[openspec:native-query-execution#Same answers as the SQL runner]] {since: "2026-10-27"}
/// The inner rows of an `EXISTS`, by the ids of the columns it is probed on.
type IdSet = std::rc::Rc<HashSet<Vec<i64>>>;

/// The rows of a prepared query on `store`, decoded.
pub fn execute(
    engine: &QueryEngine,
    store: &dyn ReadStore,
    cache: CacheMode,
    p: &Prepared,
) -> Result<QueryResult> {
    let (node, synthetic) = plan(store, p)?;
    engine.run_hook();
    let mut ex = Exec::new(store, p.query().semantics, synthetic);
    ex.parallel_threshold = engine.parallel_threshold();
    ex.datalog = engine.datalog_provider().cloned();
    if let Some(pe) = engine.path_engine() {
        ex.path_max_hops = pe.options().max_hops;
        ex.path_max_states = pe.options().max_states;
    }
    let (rows, doms) = match &node {
        Node::Empty(_) => (Vec::new(), Vec::new()),
        n => ex.root(n, p.columns())?,
    };
    let mut dec = Decoder::new(engine.term_cache(), cache, &ex.synthetic);
    let metered = budget::active();
    let mut out = Vec::with_capacity(rows.len());
    let mut load = StoreLoad(store);
    for r in rows {
        if metered {
            budget::charge_rows(1)?;
        }
        let mut cells = Vec::with_capacity(r.len());
        for (v, d) in r.iter().zip(&doms) {
            cells.push(dec.cell(&mut load, v, d)?);
        }
        if metered {
            charge_row(&cells)?;
        }
        out.push(cells);
    }
    Ok(QueryResult {
        columns: p.columns().to_vec(),
        rows: out,
        stats: dec.stats,
    })
}

// @tg: implements:: [[openspec:native-query-execution#Native explain]] {since: "2026-10-27"}
/// The native plan of a prepared query, run to collect the steps it takes.
pub fn explain(engine: &QueryEngine, store: &dyn ReadStore, p: &Prepared) -> Result<Explain> {
    let (node, synthetic) = plan(store, p)?;
    let mut ex = Exec::new(store, p.query().semantics, synthetic);
    ex.parallel_threshold = engine.parallel_threshold();
    ex.datalog = engine.datalog_provider().cloned();
    if let Some(pe) = engine.path_engine() {
        ex.path_max_hops = pe.options().max_hops;
        ex.path_max_states = pe.options().max_states;
    }
    let short = matches!(node, Node::Empty(_));
    if !short {
        ex.root(&node, p.columns())?;
    }
    let steps = ex.plan.into_inner();
    Ok(Explain {
        regions: vec![RegionInfo {
            kind: RegionKind::Native,
            note: RouteNote::None,
            aliases: Vec::new(),
            query_plan: steps.clone(),
        }],
        short_circuit: short,
        sql: None,
        params: Vec::new(),
        query_plan: steps,
    })
}

fn plan(store: &dyn ReadStore, p: &Prepared) -> Result<(Node, HashMap<i64, Value>)> {
    let mut planner = Planner::with_source(Box::new(StoreSource(store)), p.query());
    let node = planner.plan(&p.query().root)?;
    Ok((node, std::mem::take(&mut planner.synthetic)))
}

fn charge_row(row: &[Option<ResultValue>]) -> Result<()> {
    fn bytes(c: &Option<ResultValue>) -> u64 {
        match c {
            None => 8,
            Some(ResultValue::Term(v)) => budget::value_bytes(v),
            Some(ResultValue::List(xs)) => 8 + xs.iter().map(bytes).sum::<u64>(),
            Some(_) => 8,
        }
    }
    budget::charge_bytes(row.iter().map(bytes).sum())
}

fn key_row(r: &[V]) -> Vec<Key> {
    r.iter().map(sv::key).collect()
}

fn ids(r: &[V]) -> Vec<i64> {
    r.iter().map(|v| v.as_i64().unwrap_or(0)).collect()
}

impl<'a> Exec<'a> {
    /// An evaluator on `store`.
    pub fn new(
        store: &'a dyn ReadStore,
        sem: Semantics,
        synthetic: HashMap<i64, Value>,
    ) -> Exec<'a> {
        Exec {
            store,
            sem,
            terms: Terms::new(store, &synthetic),
            synthetic,
            exists: RefCell::new(HashMap::new()),
            exists_keys: RefCell::new(HashMap::new()),
            plan: RefCell::new(Vec::new()),
            path_max_hops: 15,
            path_max_states: 1_000_000,
            parallel_threshold: 0,
            datalog: None,
            datalog_calls: RefCell::new(HashMap::new()),
        }
    }

    pub(crate) fn note(&self, s: impl Into<String>) {
        self.plan.borrow_mut().push(s.into());
    }

    /// `[NOT] EXISTS`, correlated on the variables the nested tree shares with
    /// the outer row.
    pub fn exists(&self, n: &Node, neg: bool, outer: &Rel, row: &[V]) -> Result<V> {
        let key = n as *const Node as usize;
        let inner = self.exists_inner(n)?;
        let shared: Vec<(usize, usize)> = inner
            .vars
            .iter()
            .enumerate()
            .filter_map(|(ii, v)| outer.idx(v).map(|oi| (oi, ii)))
            .filter(|(oi, _)| !outer.cols[*oi].null)
            .collect();
        // term columns on both sides compare by id: probe a hash set
        let by_id = !shared.is_empty()
            && shared.iter().all(|(oi, ii)| {
                let (a, b) = (&outer.cols[*oi], &inner.cols[*ii]);
                a.dom == Dom::Term && b.dom == Dom::Term && !a.mm && !b.mm
            });
        if by_id {
            let cols: Vec<usize> = shared.iter().map(|(_, ii)| *ii).collect();
            let set = self.exists_keys.borrow().get(&(key, cols.clone())).cloned();
            let set = match set {
                Some(s) => s,
                None => {
                    let s: HashSet<Vec<i64>> = inner
                        .rows
                        .iter()
                        .filter_map(|r| {
                            cols.iter()
                                .map(|i| r[*i].as_i64())
                                .collect::<Option<Vec<i64>>>()
                        })
                        .collect();
                    let s = std::rc::Rc::new(s);
                    self.exists_keys.borrow_mut().insert((key, cols), s.clone());
                    s
                }
            };
            let probe: Option<Vec<i64>> = shared.iter().map(|(oi, _)| row[*oi].as_i64()).collect();
            let found = probe.is_some_and(|k| set.contains(&k));
            return Ok(V::Integer((found != neg) as i64));
        }
        let mut found = false;
        for ir in &inner.rows {
            let mut ok = true;
            for (oi, ii) in &shared {
                if !self.join_eq(
                    &outer.cols[*oi],
                    &row[*oi],
                    &inner.cols[*ii],
                    &ir[*ii],
                    false,
                )? {
                    ok = false;
                    break;
                }
            }
            if ok {
                found = true;
                break;
            }
        }
        Ok(V::Integer((found != neg) as i64))
    }

    /// The rows of an `EXISTS` tree, evaluated once (uncorrelated) per query.
    fn exists_inner(&self, n: &Node) -> Result<std::rc::Rc<Rel>> {
        let key = n as *const Node as usize;
        let cached = self.exists.borrow().get(&key).cloned();
        Ok(match cached {
            Some(r) => r,
            None => {
                let r = std::rc::Rc::new(self.eval(n)?);
                self.exists.borrow_mut().insert(key, r.clone());
                r
            }
        })
    }

    /// The root: hoists an OrderLimit at the root or under a root Project /
    /// Extend, orders by the keys and then by the raw output values, and returns
    /// the output rows with the domain of each column (`Gen::root`).
    pub fn root(&self, node: &Node, columns: &[Var]) -> Result<(Vec<Vec<V>>, Vec<Dom>)> {
        let mut cur = node;
        let mut proj: Option<(&Vec<Var>, bool)> = None;
        if let Node::Project(i, vars, distinct) = cur {
            proj = Some((vars, *distinct));
            cur = i;
        }
        let mut exts = Vec::new();
        while let Node::Extend(i, v, e) = cur {
            exts.push((v, e));
            cur = i;
        }
        let mut ol = None;
        if let Node::OrderLimit(i, keys, skip, limit) = cur {
            ol = Some((keys, *skip, *limit));
            cur = i;
        }
        let distinct = proj.is_some_and(|p| p.1);
        let mut r = self.eval(cur)?;
        let keyed: Vec<(Vec<V>, Vec<V>)>;
        let mut slice = (None, None);
        let ordered = ol.is_some();
        if let Some((keys, skip, lim)) = ol {
            if distinct && (skip.is_some() || lim.is_some()) {
                // limit before distinct: order and slice inside, keep the keys
                let k = self.sort_keys(keys, &r)?;
                let mut rows: Vec<(Vec<V>, Vec<V>)> = r.rows.drain(..).zip(k).collect();
                self.sort_by_keys(keys, &mut rows);
                let rows = slice_rows(rows, skip, lim);
                keyed = rows.into_iter().map(|(row, k)| (k, row)).collect();
                r.rows = keyed.iter().map(|(_, row)| row.clone()).collect();
            } else {
                let k = self.sort_keys(keys, &r)?;
                keyed = k.into_iter().zip(r.rows.iter().cloned()).collect();
                slice = (skip, lim);
            }
        } else {
            keyed = r.rows.iter().map(|row| (Vec::new(), row.clone())).collect();
        }
        for (v, e) in exts.iter().rev() {
            self.extend(&mut r, v, e)?;
        }
        // the output rows, aligned with their keys
        let idxs: Vec<Option<usize>> = columns.iter().map(|v| r.idx(v)).collect();
        let doms: Vec<Dom> = idxs
            .iter()
            .map(|i| i.map_or(Dom::Term, |i| r.cols[i].dom.clone()))
            .collect();
        let mut out: Vec<(Vec<V>, Vec<V>)> = keyed
            .into_iter()
            .zip(r.rows.iter())
            .map(|((k, _), row)| {
                let o: Vec<V> = idxs
                    .iter()
                    .map(|i| i.map_or(V::Null, |i| row[i].clone()))
                    .collect();
                (k, o)
            })
            .collect();
        if ordered {
            let descs: Vec<bool> = ol
                .map(|(keys, _, _)| keys.iter().map(|k| k.desc).collect())
                .unwrap_or_default();
            let null_first = |desc: bool| match self.sem.missing {
                Missing::Unbound => !desc,
                Missing::Null3VL => desc,
            };
            out.sort_by(|a, b| {
                for (i, d) in descs.iter().enumerate() {
                    let o = order_key(&a.0[i], &b.0[i], *d, null_first(*d));
                    if o != Ordering::Equal {
                        return o;
                    }
                }
                // tie-breakers: the raw output values
                for (x, y) in a.1.iter().zip(&b.1) {
                    let o = sv::order_nulls_first(x, y);
                    if o != Ordering::Equal {
                        return o;
                    }
                }
                Ordering::Equal
            });
        }
        let mut rows: Vec<Vec<V>> = out.into_iter().map(|(_, o)| o).collect();
        if distinct {
            let mut seen = HashSet::new();
            rows.retain(|row| seen.insert(key_row(row)));
        }
        let rows = match slice {
            (None, None) => rows,
            (skip, lim) => {
                let skip = skip.unwrap_or(0) as usize;
                let rows: Vec<Vec<V>> = rows.into_iter().skip(skip).collect();
                match lim {
                    Some(l) => rows.into_iter().take(l as usize).collect(),
                    None => rows,
                }
            }
        };
        Ok((rows, doms))
    }

    fn sort_keys(&self, keys: &[PKey], r: &Rel) -> Result<Vec<Vec<V>>> {
        let mut out = Vec::with_capacity(r.rows.len());
        for row in &r.rows {
            let mut k = Vec::with_capacity(keys.len());
            for key in keys {
                let v = self.value(&key.expr, r, row)?;
                let x = self.val_sql(&v);
                k.push(self.key_of(&v.dom, &x)?);
            }
            out.push(k);
        }
        Ok(out)
    }

    fn sort_by_keys(&self, keys: &[PKey], rows: &mut [(Vec<V>, Vec<V>)]) {
        let null_first = |desc: bool| match self.sem.missing {
            Missing::Unbound => !desc,
            Missing::Null3VL => desc,
        };
        rows.sort_by(|a, b| {
            for (i, k) in keys.iter().enumerate() {
                let o = order_key(&a.1[i], &b.1[i], k.desc, null_first(k.desc));
                if o != Ordering::Equal {
                    return o;
                }
            }
            Ordering::Equal
        });
    }

    /// Evaluates a plan node.
    pub fn eval(&self, node: &Node) -> Result<Rel> {
        budget::check()?;
        match node {
            Node::Empty(vars) => Ok(Rel {
                vars: vars.clone(),
                cols: vars.iter().map(|_| Col::null()).collect(),
                rows: Vec::new(),
                iso: Vec::new(),
            }),
            Node::Triple(t) => self.bgp(&[t], None),
            Node::Virtual(v) => {
                let r = Rel::unit();
                self.virtual_pattern(r, v)
            }
            Node::Volatile(v) => self.volatile(v),
            Node::Path(p) => crate::path::eval(self, p, &Rel::unit()),
            Node::Text(t) => self.text(t),
            Node::Datalog(t) => self.datalog_call(t),
            Node::Vector(t) => self.vector(t),
            Node::Recall(t) => self.recall(t),
            Node::Values(v) => Ok(values(v)),
            Node::Unnest(input, list, var) => self.unnest(input, list, var),
            Node::Join(inputs, null_safe) => self.join(inputs, null_safe),
            Node::LeftJoin(l, r, c) => self.left_join(l, r, c.as_ref()),
            Node::Filter(input, cond) => {
                if let Node::Aggregate(i, group, aggs) = &**input {
                    return self.aggregate(i, group, aggs, Some(cond));
                }
                let parts = match cond {
                    PExpr::And(xs) => xs.clone(),
                    other => vec![other.clone()],
                };
                let (mut r, late) = match &**input {
                    Node::Join(inputs, ns) => self.join_with(inputs, ns, parts)?,
                    // a lone pattern under filters joins as a pattern too, so
                    // its conjuncts run inside the match (Cypher's `{name: …}`)
                    n if lift(n).is_some() => {
                        self.join_with(std::slice::from_ref(&**input), &[], parts)?
                    }
                    _ => (self.eval(input)?, parts),
                };
                self.filter_rows(&mut r, &late)?;
                Ok(r)
            }
            Node::Union(inputs) => self.union(inputs),
            Node::Extend(input, var, e) => {
                let mut r = self.eval(input)?;
                self.extend(&mut r, var, e)?;
                Ok(r)
            }
            Node::Aggregate(input, group, aggs) => self.aggregate(input, group, aggs, None),
            Node::Project(input, vars, distinct) => {
                let r = self.eval(input)?;
                let idxs: Vec<Option<usize>> = vars.iter().map(|v| r.idx(v)).collect();
                let cols = idxs
                    .iter()
                    .map(|i| i.map_or_else(Col::null, |i| r.cols[i].clone()))
                    .collect();
                let mut rows: Vec<Vec<V>> = r
                    .rows
                    .iter()
                    .map(|row| {
                        idxs.iter()
                            .map(|i| i.map_or(V::Null, |i| row[i].clone()))
                            .collect()
                    })
                    .collect();
                if *distinct {
                    let mut seen = HashSet::new();
                    rows.retain(|row| seen.insert(key_row(row)));
                }
                let iso = r
                    .iso
                    .iter()
                    .filter_map(|i| {
                        idxs.iter()
                            .position(|x| *x == Some(i.col))
                            .map(|col| Iso { col, ..i.clone() })
                    })
                    .collect();
                Ok(Rel {
                    vars: vars.clone(),
                    cols,
                    rows,
                    iso: if *distinct { Vec::new() } else { iso },
                })
            }
            Node::OrderLimit(input, keys, skip, limit) => {
                let r = self.eval(input)?;
                let k = self.sort_keys(keys, &r)?;
                let mut rows: Vec<(Vec<V>, Vec<V>)> = r.rows.into_iter().zip(k).collect();
                self.sort_by_keys(keys, &mut rows);
                let rows = slice_rows(rows, *skip, *limit);
                Ok(Rel {
                    rows: rows.into_iter().map(|(row, _)| row).collect(),
                    vars: r.vars,
                    cols: r.cols,
                    iso: Vec::new(),
                })
            }
            Node::RowNumber(input, partition, keys, var) => {
                let mut r = self.eval(input)?;
                let k = self.sort_keys(keys, &r)?;
                let parts: Vec<Option<usize>> = partition.iter().map(|v| r.idx(v)).collect();
                let mut rows: Vec<(Vec<V>, Vec<V>)> = r.rows.drain(..).zip(k).collect();
                // order within a partition; partitions grouped by their values
                rows.sort_by(|a, b| {
                    for p in &parts {
                        let (x, y) = match p {
                            Some(i) => (&a.0[*i], &b.0[*i]),
                            None => continue,
                        };
                        let o = sv::order_nulls_first(x, y);
                        if o != Ordering::Equal {
                            return o;
                        }
                    }
                    Ordering::Equal
                });
                let null_first = |desc: bool| match self.sem.missing {
                    Missing::Unbound => !desc,
                    Missing::Null3VL => desc,
                };
                rows.sort_by(|a, b| {
                    for i in parts.iter().flatten() {
                        let o = sv::order_nulls_first(&a.0[*i], &b.0[*i]);
                        if o != Ordering::Equal {
                            return o;
                        }
                    }
                    for (i, key) in keys.iter().enumerate() {
                        let o = order_key(&a.1[i], &b.1[i], key.desc, null_first(key.desc));
                        if o != Ordering::Equal {
                            return o;
                        }
                    }
                    Ordering::Equal
                });
                let mut n = 0i64;
                let mut prev: Option<Vec<Key>> = None;
                for (row, _) in rows {
                    let part: Vec<Key> = parts
                        .iter()
                        .map(|p| p.map_or(Key::Null, |i| sv::key(&row[i])))
                        .collect();
                    if prev.as_ref() != Some(&part) {
                        n = 0;
                        prev = Some(part);
                    }
                    n += 1;
                    let mut row = row;
                    row.push(V::Integer(n));
                    r.rows.push(row);
                }
                r.vars.push(var.clone());
                r.cols.push(Col {
                    dom: Dom::Computed(VClass::Int),
                    mm: false,
                    null: false,
                });
                r.iso.clear();
                Ok(r)
            }
            Node::PadMissing(input, vars) => {
                let mut r = self.eval(input)?;
                for v in vars {
                    if r.idx(v).is_none() {
                        r.push_col(v, Col::null(), V::Null);
                    }
                }
                Ok(r)
            }
        }
    }

    fn extend(&self, r: &mut Rel, var: &Var, e: &PExpr) -> Result<()> {
        let mut vals = Vec::with_capacity(r.rows.len());
        let mut col: Option<Col> = None;
        for row in &r.rows {
            let v = self.value(e, r, row)?;
            if col.is_none() {
                col = Some(Col {
                    dom: v.dom.clone(),
                    mm: v.mm,
                    null: false,
                });
            }
            vals.push(self.val_sql(&v));
        }
        let col = match col {
            Some(c) => c,
            None => {
                // no row: the static facts from a row of NULLs
                let dummy: Vec<V> = vec![V::Null; r.vars.len()];
                match self.value(e, r, &dummy) {
                    Ok(v) => Col {
                        dom: v.dom,
                        mm: v.mm,
                        null: false,
                    },
                    Err(_) => Col::term(true),
                }
            }
        };
        match r.idx(var) {
            Some(i) => {
                r.cols[i] = col;
                for (row, v) in r.rows.iter_mut().zip(vals) {
                    row[i] = v;
                }
            }
            None => {
                r.vars.push(var.clone());
                r.cols.push(col);
                for (row, v) in r.rows.iter_mut().zip(vals) {
                    row.push(v);
                }
            }
        }
        Ok(())
    }

    fn bgp(&self, pats: &[&PTriple], seed: Option<&Rel>) -> Result<Rel> {
        self.bgp_filtered(pats, seed, &[])
    }

    /// [`Exec::bgp`] checking `filters` (over the patterns' variables) during the join.
    fn bgp_filtered(
        &self,
        pats: &[&PTriple],
        seed: Option<&Rel>,
        filters: &[PExpr],
    ) -> Result<Rel> {
        let (rel, steps) = match seed {
            Some(s) => {
                let rows: Vec<Vec<i64>> = s.rows.iter().map(|r| ids(r)).collect();
                crate::bgp::eval(self, pats, Some((&s.vars, rows)), filters)?
            }
            None => crate::bgp::eval(self, pats, None, filters)?,
        };
        for s in steps {
            self.note(s);
        }
        Ok(rel)
    }

    // @tg: implements:: [[openspec:native-query-execution#Budgets on the native runner]] {since: "2026-10-27"}
    /// Keeps the rows of `r` on which every conjunct holds.
    fn filter_rows(&self, r: &mut Rel, parts: &[PExpr]) -> Result<()> {
        if parts.is_empty() {
            return Ok(());
        }
        let mut keep = Vec::with_capacity(r.rows.len());
        for row in &r.rows {
            budget::check()?;
            let mut ok = true;
            for p in parts {
                if sv::cond(&self.cond(p, r, row)?) != Some(true) {
                    ok = false;
                    break;
                }
            }
            keep.push(ok);
        }
        let mut it = keep.into_iter();
        r.rows.retain(|_| it.next().unwrap_or(false));
        Ok(())
    }

    fn join(&self, inputs: &[Node], null_safe: &[Var]) -> Result<Rel> {
        let (mut r, late) = self.join_with(inputs, null_safe, Vec::new())?;
        self.filter_rows(&mut r, &late)?;
        Ok(r)
    }

    /// A join that applies each conjunct of `conds` as early as its variables
    /// allow: to an input, to each connected component of the patterns, then to
    /// the joined rows before the paths run. Returns the conjuncts it could not
    /// place (they read what a path or a virtual pattern binds).
    fn join_with(
        &self,
        inputs: &[Node],
        null_safe: &[Var],
        mut conds: Vec<PExpr>,
    ) -> Result<(Rel, Vec<PExpr>)> {
        // patterns wrapped in filters and projections that keep every variable
        // join as patterns; their filters become conjuncts of the join
        let mut triples: Vec<&PTriple> = Vec::new();
        let mut lifted = vec![false; inputs.len()];
        for (i, n) in inputs.iter().enumerate() {
            if let Some((t, cs)) = lift(n) {
                triples.push(t);
                for c in cs {
                    match c {
                        PExpr::And(xs) => conds.extend(xs.iter().cloned()),
                        other => conds.push(other.clone()),
                    }
                }
                lifted[i] = true;
            }
        }
        let mut others: Vec<Rel> = Vec::new();
        for (i, n) in inputs.iter().enumerate() {
            if !lifted[i] && !matches!(n, Node::Virtual(_) | Node::Path(_)) {
                others.push(self.eval(n)?);
            }
        }
        // every variable an input binds; a conjunct placed on a relation must
        // read none outside it
        let mut bound: Vec<Var> = Vec::new();
        let mut add = |v: &Var| {
            if !bound.contains(v) {
                bound.push(v.clone());
            }
        };
        for t in &triples {
            for v in triple_vars(t) {
                add(v);
            }
        }
        for r in &others {
            r.vars.iter().for_each(&mut add);
        }
        for n in inputs {
            if let Node::Virtual(PVirtual {
                object: PObj::Var(v),
                ..
            }) = n
            {
                add(v);
            }
        }
        // what only a path binds (an endpoint another input binds is a filter
        // of the path, not its output)
        let late: Vec<String> = path_outputs(inputs)
            .into_iter()
            .filter(|v| !bound.iter().any(|b| &format!("{b:?}") == v))
            .collect();
        let place = |r: &mut Rel, conds: &mut Vec<PExpr>, extra: &[String]| -> Result<()> {
            if conds.is_empty() {
                return Ok(());
            }
            // a variable this relation may not hold its final value for: one it
            // does not bind, or a joined one the merge may coalesce or retype
            let mut outside: Vec<String> = extra.to_vec();
            for v in &bound {
                let unsafe_here = match r.idx(v) {
                    None => true,
                    Some(i) => {
                        let c = &r.cols[i];
                        null_safe.contains(v) || c.dom != Dom::Term || c.null || c.mm
                    }
                };
                if unsafe_here {
                    outside.push(format!("{v:?}"));
                }
            }
            let (now, later): (Vec<PExpr>, Vec<PExpr>) = std::mem::take(conds)
                .into_iter()
                .partition(|c| !mentions_any(c, &outside));
            *conds = later;
            self.filter_rows(r, &now)
        };
        for r in &mut others {
            place(r, &mut conds, &late)?;
        }
        // a small positive `EXISTS` conjunct is a semi-join: its distinct values
        // on the patterns' variables seed them (the conjunct is still checked)
        let all_triple_vars: HashSet<&Var> = triples.iter().flat_map(|t| triple_vars(t)).collect();
        let mut semi: Vec<Rel> = Vec::new();
        for c in &conds {
            let PExpr::Exists(n, false) = c else { continue };
            let inner = self.exists_inner(n)?;
            if inner.rows.len() > 10_000 {
                continue;
            }
            let cols: Vec<usize> = (0..inner.vars.len())
                .filter(|i| {
                    let col = &inner.cols[*i];
                    all_triple_vars.contains(&inner.vars[*i])
                        && !null_safe.contains(&inner.vars[*i])
                        && col.dom == Dom::Term
                        && !col.mm
                        && !col.null
                })
                .collect();
            if cols.is_empty() {
                continue;
            }
            let mut seen = HashSet::new();
            let rows: Vec<Vec<V>> = inner
                .rows
                .iter()
                .map(|r| cols.iter().map(|i| r[*i].clone()).collect::<Vec<V>>())
                .filter(|r: &Vec<V>| seen.insert(r.iter().map(sv::key).collect::<Vec<Key>>()))
                .collect();
            semi.push(Rel {
                vars: cols.iter().map(|i| inner.vars[*i].clone()).collect(),
                cols: cols.iter().map(|i| inner.cols[*i].clone()).collect(),
                rows,
                iso: Vec::new(),
            });
        }
        // each connected component of the patterns is matched on its own (a
        // small all-term input seeds the one it shares variables with), so
        // filters cut it before the components are crossed
        let mut rels: Vec<Rel> = Vec::new();
        for comp in components(&triples) {
            let comp_vars: HashSet<&Var> = comp.iter().flat_map(|t| triple_vars(t)).collect();
            let semi_seed = semi
                .iter()
                .enumerate()
                .filter(|(_, r)| r.vars.iter().all(|v| comp_vars.contains(v)))
                .min_by_key(|(_, r)| r.rows.len())
                .map(|(i, r)| (i, r.rows.len()));
            let seed = others
                .iter()
                .enumerate()
                .filter(|(_, r)| {
                    r.rows.len() <= 10_000
                        && !r.vars.is_empty()
                        && r.iso.is_empty()
                        && r.cols
                            .iter()
                            .all(|c| c.dom == Dom::Term && !c.mm && !c.null)
                        && r.vars.iter().any(|v| comp_vars.contains(v))
                        && r.vars.iter().all(|v| !null_safe.contains(v))
                })
                .min_by_key(|(_, r)| r.rows.len())
                .map(|(i, _)| i);
            let other_rows = seed.map(|i| others[i].rows.len());
            // conjuncts that read only this component's variables run inside the
            // join, each as soon as its variables are bound
            let mut outside: Vec<String> = late.clone();
            for v in &bound {
                if !comp_vars.contains(v)
                    || null_safe.contains(v)
                    || others.iter().any(|r| r.vars.contains(v))
                {
                    outside.push(format!("{v:?}"));
                }
            }
            let (early, rest): (Vec<PExpr>, Vec<PExpr>) = std::mem::take(&mut conds)
                .into_iter()
                .partition(|c| !mentions_any(c, &outside));
            conds = rest;
            let mut r = match (seed, semi_seed) {
                (Some(i), sm) if sm.is_none_or(|(_, n)| other_rows.unwrap_or(0) <= n) => {
                    let s = others.remove(i);
                    // the seed's own variables are not this component's
                    let (inner, after): (Vec<PExpr>, Vec<PExpr>) =
                        early.into_iter().partition(|c| {
                            !mentions_any(
                                c,
                                &s.vars
                                    .iter()
                                    .filter(|v| !comp_vars.contains(v))
                                    .map(|v| format!("{v:?}"))
                                    .collect::<Vec<_>>(),
                            )
                        });
                    conds.extend(after);
                    self.bgp_filtered(&comp, Some(&s), &inner)?
                }
                (_, Some((i, _))) => {
                    let s = semi.remove(i);
                    self.note(format!(
                        "semi-join seed from EXISTS ({} values)",
                        s.rows.len()
                    ));
                    self.bgp_filtered(&comp, Some(&s), &early)?
                }
                _ => self.bgp_filtered(&comp, None, &early)?,
            };
            place(&mut r, &mut conds, &late)?;
            rels.push(r);
        }
        rels.extend(others);
        let mut acc = Rel::unit();
        for r in rels {
            acc = self.merge(acc, r, null_safe)?;
        }
        for n in inputs {
            if let Node::Virtual(v) = n {
                acc = self.virtual_pattern(acc, v)?;
            }
        }
        // conjuncts that read nothing a path binds are checked before the paths
        // run, as SQLite tests them before calling `tm_path`
        let has_paths = inputs.iter().any(|n| matches!(n, Node::Path(_)));
        if has_paths {
            place(&mut acc, &mut conds, &late)?;
        }
        let mut pending: Vec<&tm_exec::plan::PPath> = inputs
            .iter()
            .filter_map(|n| match n {
                Node::Path(p) => Some(p),
                _ => None,
            })
            .collect();
        while !pending.is_empty() {
            let ready = pending.iter().position(|p| match &p.arg {
                PTerm::Id(_) => true,
                PTerm::Var(v) => acc.idx(v).is_some(),
            });
            let p = pending.remove(ready.unwrap_or(0));
            acc = crate::path::eval(self, p, &acc)?;
        }
        Ok((acc, conds))
    }

    /// The inner join of two relations (`Gen::merge`).
    pub fn merge(&self, acc: Rel, r: Rel, null_safe: &[Var]) -> Result<Rel> {
        // the unit relation is the identity of the join: no copy of the other side
        let unit = |x: &Rel| x.vars.is_empty() && x.iso.is_empty() && x.rows.len() == 1;
        if unit(&acc) {
            return Ok(r);
        }
        if unit(&r) {
            return Ok(acc);
        }
        let shared: Vec<(usize, usize, bool)> = r
            .vars
            .iter()
            .enumerate()
            .filter_map(|(ri, v)| acc.idx(v).map(|ai| (ai, ri, null_safe.contains(v))))
            .collect();
        // the merged columns
        let mut vars = acc.vars.clone();
        let mut cols = acc.cols.clone();
        let mut merged_from: Vec<(usize, Option<usize>, bool)> = Vec::new();
        for (ai, ri, ns) in &shared {
            let (a, b) = (&acc.cols[*ai], &r.cols[*ri]);
            let coalesce = *ns || (a.mm && self.sem.missing == Missing::Unbound);
            let col = if *ns {
                Col {
                    dom: a.dom.clone(),
                    mm: a.mm && b.mm,
                    null: false,
                }
            } else if (a.mm || b.mm) && self.sem.missing == Missing::Unbound {
                if a.mm {
                    Col {
                        dom: a.dom.clone(),
                        mm: a.mm && b.mm,
                        null: false,
                    }
                } else {
                    a.clone()
                }
            } else {
                Col {
                    mm: false,
                    ..a.clone()
                }
            };
            cols[*ai] = col;
            merged_from.push((*ai, Some(*ri), coalesce));
        }
        let right_only: Vec<usize> = (0..r.vars.len())
            .filter(|ri| !shared.iter().any(|(_, x, _)| x == ri))
            .collect();
        for ri in &right_only {
            vars.push(r.vars[*ri].clone());
            cols.push(r.cols[*ri].clone());
        }
        // isomorphism between the two sides
        let mut iso_pairs: Vec<(usize, usize, bool)> = Vec::new();
        for b in &r.iso {
            for a in &acc.iso {
                if a.group != b.group {
                    continue;
                }
                if let (Some(p), Some(q)) = (a.pred, b.pred) {
                    if p != q {
                        continue;
                    }
                }
                iso_pairs.push((a.col, b.col, a.optional || b.optional));
            }
        }
        let hashable = shared.iter().all(|(ai, ri, ns)| {
            let (a, b) = (&acc.cols[*ai], &r.cols[*ri]);
            !ns && a.dom == b.dom
                && !a.null
                && !b.null
                && !(self.sem.missing == Missing::Unbound && (a.mm || b.mm))
        });
        let mut rows = Vec::new();
        let emit = |lrow: &[V], rrow: &[V], rows: &mut Vec<Vec<V>>| {
            let mut out = lrow.to_vec();
            for (ai, ri, coalesce) in &merged_from {
                if let Some(ri) = ri {
                    if *coalesce && out[*ai].is_null() {
                        out[*ai] = rrow[*ri].clone();
                    }
                }
            }
            for ri in &right_only {
                out.push(rrow[*ri].clone());
            }
            rows.push(out);
        };
        let iso_ok = |lrow: &[V], rrow: &[V]| {
            iso_pairs.iter().all(|(a, b, opt)| {
                let (x, y) = (&lrow[*a], &rrow[*b]);
                (*opt && (x.is_null() || y.is_null())) || sv::cond(&sv::eq(x, y)) == Some(false)
            })
        };
        if hashable && !shared.is_empty() {
            let mut table: FxHashMap<Vec<Key>, Vec<usize>> = FxHashMap::default();
            for (i, rrow) in r.rows.iter().enumerate() {
                let k: Vec<Key> = shared
                    .iter()
                    .map(|(_, ri, _)| sv::key(&rrow[*ri]))
                    .collect();
                if k.contains(&Key::Null) {
                    continue;
                }
                table.entry(k).or_default().push(i);
            }
            for (n, lrow) in acc.rows.iter().enumerate() {
                if n % 4096 == 0 {
                    budget::check()?;
                }
                let k: Vec<Key> = shared
                    .iter()
                    .map(|(ai, _, _)| sv::key(&lrow[*ai]))
                    .collect();
                if let Some(ms) = table.get(&k) {
                    for i in ms {
                        budget::poll()?;
                        let rrow = &r.rows[*i];
                        // numeric keys collide across classes; confirm with SQL equality
                        let mut ok = true;
                        for (ai, ri, _) in &shared {
                            if sv::cond(&sv::eq(&lrow[*ai], &rrow[*ri])) != Some(true) {
                                ok = false;
                                break;
                            }
                        }
                        if ok && iso_ok(lrow, rrow) {
                            emit(lrow, rrow, &mut rows);
                        }
                    }
                }
            }
        } else {
            for (n, lrow) in acc.rows.iter().enumerate() {
                if n % 1024 == 0 {
                    budget::check()?;
                }
                for rrow in &r.rows {
                    budget::poll()?;
                    let mut ok = true;
                    for (ai, ri, ns) in &shared {
                        if !self.join_eq(
                            &acc.cols[*ai],
                            &lrow[*ai],
                            &r.cols[*ri],
                            &rrow[*ri],
                            *ns,
                        )? {
                            ok = false;
                            break;
                        }
                    }
                    if ok && iso_ok(lrow, rrow) {
                        emit(lrow, rrow, &mut rows);
                    }
                }
            }
        }
        let mut iso = acc.iso.clone();
        for b in &r.iso {
            let col = match shared.iter().find(|(_, ri, _)| *ri == b.col) {
                Some((ai, _, _)) => *ai,
                None => acc.vars.len() + right_only.iter().position(|x| *x == b.col).unwrap_or(0),
            };
            iso.push(Iso { col, ..b.clone() });
        }
        Ok(Rel {
            vars,
            cols,
            rows,
            iso,
        })
    }

    fn left_join(&self, left: &Node, right: &Node, cond: Option<&PExpr>) -> Result<Rel> {
        let l = self.eval(left)?;
        let r = self.eval(right)?;
        let shared: Vec<(usize, usize)> = r
            .vars
            .iter()
            .enumerate()
            .filter_map(|(ri, v)| l.idx(v).map(|li| (li, ri)))
            .collect();
        let right_only: Vec<usize> = (0..r.vars.len())
            .filter(|ri| !shared.iter().any(|(_, x)| x == ri))
            .collect();
        let mut vars = l.vars.clone();
        let mut cols = l.cols.clone();
        let mut coalesce = Vec::new();
        for (li, ri) in &shared {
            let prev = &l.cols[*li];
            if prev.mm && self.sem.missing == Missing::Unbound {
                // no match leaves the right side NULL; a missing left value takes the right one
                let b = &r.cols[*ri];
                cols[*li] = Col {
                    dom: prev.dom.clone(),
                    mm: true,
                    null: false,
                };
                let _ = b;
                coalesce.push((*li, *ri));
            }
        }
        for ri in &right_only {
            vars.push(r.vars[*ri].clone());
            cols.push(Col {
                mm: true,
                ..r.cols[*ri].clone()
            });
        }
        // the condition's scope: the merged columns plus the right-only ones
        let scope = Rel {
            vars: vars.clone(),
            cols: cols.clone(),
            rows: Vec::new(),
            iso: Vec::new(),
        };
        let mut iso_pairs: Vec<(usize, usize)> = Vec::new();
        for b in &r.iso {
            for a in &l.iso {
                if a.group == b.group && !matches!((a.pred, b.pred), (Some(p), Some(q)) if p != q) {
                    iso_pairs.push((a.col, b.col));
                }
            }
        }
        let mut rows = Vec::new();
        for (n, lrow) in l.rows.iter().enumerate() {
            if n % 1024 == 0 {
                budget::check()?;
            }
            let mut matched = false;
            for rrow in &r.rows {
                let mut ok = true;
                for (li, ri) in &shared {
                    if !self.join_eq(&l.cols[*li], &lrow[*li], &r.cols[*ri], &rrow[*ri], false)? {
                        ok = false;
                        break;
                    }
                }
                if !ok {
                    continue;
                }
                if !iso_pairs
                    .iter()
                    .all(|(a, b)| sv::cond(&sv::eq(&lrow[*a], &rrow[*b])) == Some(false))
                {
                    continue;
                }
                let mut out = lrow.clone();
                for (li, ri) in &coalesce {
                    if out[*li].is_null() {
                        out[*li] = rrow[*ri].clone();
                    }
                }
                for ri in &right_only {
                    out.push(rrow[*ri].clone());
                }
                if let Some(c) = cond {
                    // shared variables read the left value (merged), right-only the right one
                    if sv::cond(&self.cond(c, &scope, &out)?) != Some(true) {
                        continue;
                    }
                }
                matched = true;
                rows.push(out);
            }
            if !matched {
                let mut out = lrow.clone();
                for _ in &right_only {
                    out.push(V::Null);
                }
                rows.push(out);
            }
        }
        let mut iso = l.iso.clone();
        for b in &r.iso {
            let col = match shared.iter().find(|(_, ri)| *ri == b.col) {
                Some((li, _)) => *li,
                None => l.vars.len() + right_only.iter().position(|x| *x == b.col).unwrap_or(0),
            };
            iso.push(Iso {
                col,
                optional: true,
                ..b.clone()
            });
        }
        Ok(Rel {
            vars,
            cols,
            rows,
            iso,
        })
    }

    fn union(&self, inputs: &[Node]) -> Result<Rel> {
        let rels: Vec<Rel> = inputs.iter().map(|n| self.eval(n)).collect::<Result<_>>()?;
        let mut vars: Vec<Var> = Vec::new();
        for r in &rels {
            for v in &r.vars {
                if !vars.contains(v) {
                    vars.push(v.clone());
                }
            }
        }
        let mut cols = Vec::new();
        for v in &vars {
            let doms: Vec<&Dom> = rels
                .iter()
                .filter_map(|r| r.col(v))
                .filter(|c| !c.null)
                .map(|c| &c.dom)
                .collect();
            if doms.windows(2).any(|w| w[0] != w[1]) {
                return Err(tm_exec::error::unsupported(
                    "a Union column mixing stored terms and computed values".to_string(),
                ));
            }
            let mm = rels.iter().any(|r| r.col(v).is_none_or(|c| c.mm));
            cols.push(Col {
                dom: doms.first().map_or(Dom::Term, |d| (*d).clone()),
                mm,
                null: false,
            });
        }
        let mut rows = Vec::new();
        for r in &rels {
            let idx: Vec<Option<usize>> = vars.iter().map(|v| r.idx(v)).collect();
            for row in &r.rows {
                rows.push(
                    idx.iter()
                        .map(|i| i.map_or(V::Null, |i| row[i].clone()))
                        .collect(),
                );
            }
        }
        Ok(Rel {
            vars,
            cols,
            rows,
            iso: Vec::new(),
        })
    }

    fn unnest(&self, input: &Node, list: &PExpr, var: &Var) -> Result<Rel> {
        let r = self.eval(input)?;
        let mut out = Rel {
            vars: r.vars.clone(),
            cols: r.cols.clone(),
            rows: Vec::new(),
            iso: r.iso.clone(),
        };
        let mut elem_dom: Option<Dom> = None;
        let existing = r.idx(var);
        for row in &r.rows {
            let lv = self.value(list, &r, row)?;
            let elem = match &lv.dom {
                Dom::List(e) => (**e).clone(),
                Dom::TermOrList | Dom::Term => Dom::Term,
                Dom::Computed(c) => Dom::Computed(*c),
                Dom::PathJson { .. } => Dom::Computed(VClass::Dynamic),
            };
            let ecol = Col {
                dom: elem.clone(),
                mm: false,
                null: false,
            };
            elem_dom.get_or_insert(elem);
            for x in sv::json_each(&self.val_sql(&lv)) {
                match existing {
                    Some(i) => {
                        if self.join_eq(&r.cols[i], &row[i], &ecol, &x, false)? {
                            let mut o = row.clone();
                            if o[i].is_null() {
                                o[i] = x;
                            }
                            out.rows.push(o);
                        }
                    }
                    None => {
                        let mut o = row.clone();
                        o.push(x);
                        out.rows.push(o);
                    }
                }
            }
        }
        if existing.is_none() {
            out.vars.push(var.clone());
            out.cols.push(Col {
                dom: elem_dom.unwrap_or(Dom::Term),
                mm: false,
                null: false,
            });
        }
        Ok(out)
    }

    fn aggregate(
        &self,
        input: &Node,
        group: &[Var],
        aggs: &[PAgg],
        having: Option<&PExpr>,
    ) -> Result<Rel> {
        let r = self.eval(input)?;
        let gidx: Vec<Option<usize>> = group
            .iter()
            .map(|g| r.idx(g).filter(|i| !r.cols[*i].null))
            .collect();
        let keyed = gidx.iter().any(|g| g.is_some());
        let mut groups: Vec<(Vec<Key>, Vec<usize>)> = Vec::new();
        let mut index: FxHashMap<Vec<Key>, usize> = FxHashMap::default();
        for (i, row) in r.rows.iter().enumerate() {
            let k: Vec<Key> = gidx
                .iter()
                .filter_map(|g| g.map(|g| sv::key(&row[g])))
                .collect();
            match index.entry(k) {
                std::collections::hash_map::Entry::Occupied(e) => groups[*e.get()].1.push(i),
                std::collections::hash_map::Entry::Vacant(e) => {
                    groups.push((e.key().clone(), vec![i]));
                    e.insert(groups.len() - 1);
                }
            }
        }
        if groups.is_empty() && !keyed {
            groups.push((Vec::new(), Vec::new()));
        }
        let mut vars: Vec<Var> = Vec::new();
        let mut cols: Vec<Col> = Vec::new();
        for (g, gi) in group.iter().zip(&gidx) {
            vars.push(g.clone());
            cols.push(match gi {
                Some(i) => Col {
                    null: false,
                    ..r.cols[*i].clone()
                },
                None => Col::null(),
            });
        }
        let mut agg_cols: Vec<Option<Col>> = vec![None; aggs.len()];
        let mut rows = Vec::with_capacity(groups.len());
        for (_, members) in &groups {
            let mut out: Vec<V> = gidx
                .iter()
                .map(|g| match (g, members.first()) {
                    (Some(g), Some(m)) => r.rows[*m][*g].clone(),
                    _ => V::Null,
                })
                .collect();
            for (k, a) in aggs.iter().enumerate() {
                let (v, c) = self.agg(a, &r, members)?;
                if agg_cols[k].is_none() {
                    agg_cols[k] = Some(c);
                }
                out.push(v);
            }
            rows.push(out);
        }
        for (k, a) in aggs.iter().enumerate() {
            vars.push(a.var.clone());
            let c = match agg_cols[k].take() {
                Some(c) => c,
                None => self.agg(a, &r, &[])?.1,
            };
            cols.push(c);
        }
        let mut out = Rel {
            vars,
            cols,
            rows,
            iso: Vec::new(),
        };
        if let Some(h) = having {
            let mut keep = Vec::with_capacity(out.rows.len());
            for row in &out.rows {
                keep.push(sv::cond(&self.cond(h, &out, row)?) == Some(true));
            }
            let mut it = keep.into_iter();
            out.rows.retain(|_| it.next().unwrap_or(false));
        }
        Ok(out)
    }

    fn agg(&self, a: &PAgg, r: &Rel, members: &[usize]) -> Result<(V, Col)> {
        let computed = |c: VClass, mm: bool| Col {
            dom: Dom::Computed(c),
            mm,
            null: false,
        };
        let Some(arg) = &a.arg else {
            return Ok((
                V::Integer(members.len() as i64),
                computed(VClass::Int, false),
            ));
        };
        let mut vals: Vec<Val> = Vec::with_capacity(members.len());
        for m in members {
            vals.push(self.value(arg, r, &r.rows[*m])?);
        }
        let dom = match vals.first() {
            Some(v) => v.dom.clone(),
            None => {
                let dummy = vec![V::Null; r.vars.len()];
                self.value(arg, r, &dummy)
                    .map(|v| v.dom)
                    .unwrap_or(Dom::Term)
            }
        };
        let xs: Vec<V> = vals.iter().map(|v| self.val_sql(v)).collect();
        let distinct = |xs: Vec<V>| -> Vec<V> {
            if !a.distinct {
                return xs;
            }
            let mut seen = HashSet::new();
            xs.into_iter()
                .filter(|x| x.is_null() || seen.insert(sv::key(x)))
                .collect()
        };
        Ok(match &a.func {
            AggFunc::Count => {
                let xs = distinct(xs);
                (
                    V::Integer(xs.iter().filter(|x| !x.is_null()).count() as i64),
                    computed(VClass::Int, false),
                )
            }
            AggFunc::Sum | AggFunc::Avg => {
                let mut nums = Vec::with_capacity(xs.len());
                for x in &xs {
                    nums.push(self.num_of(&dom, x)?);
                }
                let nums = distinct(nums);
                let mut s = sv::Sum::default();
                for n in &nums {
                    s.add(n);
                }
                if a.func == AggFunc::Avg {
                    (s.avg(), computed(VClass::Double, true))
                } else {
                    let c = match dom {
                        Dom::Computed(VClass::Int) => VClass::Int,
                        Dom::Computed(VClass::Double) => VClass::Double,
                        _ => VClass::Dynamic,
                    };
                    (s.sum().map_err(crate::funcs::sql_error)?, computed(c, true))
                }
            }
            AggFunc::Min | AggFunc::Max => {
                let f = if a.func == AggFunc::Min {
                    "tm_min_by"
                } else {
                    "tm_max_by"
                };
                let mut st = aggregate(f)?;
                for x in &xs {
                    let k = self.key_of(&dom, x)?;
                    st.step(&[x.clone(), k]).map_err(crate::funcs::sql_error)?;
                }
                let v = st.finish().map_err(crate::funcs::sql_error)?;
                (
                    v,
                    Col {
                        dom: dom.clone(),
                        mm: true,
                        null: false,
                    },
                )
            }
            AggFunc::Sample => {
                let v = xs
                    .iter()
                    .filter(|x| !x.is_null())
                    .min_by(|x, y| sv::order(x, y))
                    .cloned()
                    .unwrap_or(V::Null);
                (
                    v,
                    Col {
                        dom: dom.clone(),
                        mm: true,
                        null: false,
                    },
                )
            }
            AggFunc::GroupConcat { sep } => {
                if a.distinct {
                    return Err(tm_exec::error::unsupported(
                        "group_concat(DISTINCT …) with a separator".to_string(),
                    ));
                }
                let mut parts = Vec::new();
                for x in &xs {
                    if let Some(t) = sv::text_of(&self.str_of(&dom, x)?) {
                        parts.push(t);
                    }
                }
                let v = if parts.is_empty() {
                    V::Null
                } else {
                    V::Text(parts.join(sep))
                };
                (v, computed(VClass::Str, true))
            }
            AggFunc::Collect => {
                let xs: Vec<V> = distinct(xs).into_iter().filter(|x| !x.is_null()).collect();
                (
                    sv::json_array(&xs),
                    Col {
                        dom: Dom::List(Box::new(dom.clone())),
                        mm: false,
                        null: false,
                    },
                )
            }
        })
    }

    fn virtual_pattern(&self, mut acc: Rel, v: &PVirtual) -> Result<Rel> {
        let view = store_view(&v.view);
        let subject_col = match &v.subject {
            PTerm::Var(s) => acc.idx(s),
            PTerm::Id(_) => None,
        };
        let obj_idx = match &v.object {
            PObj::Var(o) => acc.idx(o),
            PObj::Column(_) => None,
        };
        let mut rows = Vec::new();
        let mut new_cols: Vec<(Var, Col)> = Vec::new();
        if let (PTerm::Var(s), None) = (&v.subject, subject_col) {
            new_cols.push((s.clone(), Col::term(false)));
        }
        if let (PObj::Var(o), None) = (&v.object, obj_idx) {
            new_cols.push((o.clone(), Col::term(false)));
        }
        for row in &acc.rows {
            let candidates: Vec<tm_store::StmtRow> = match (&v.subject, subject_col) {
                (PTerm::Id(id), _) => self.store.statement(id.raw())?.into_iter().collect(),
                (PTerm::Var(_), Some(i)) => match &row[i] {
                    V::Integer(e) => self.store.statement(*e)?.into_iter().collect(),
                    _ => Vec::new(),
                },
                (PTerm::Var(_), None) => view.scan_eid(self.store, Pattern::ANY)?,
            };
            for st in candidates {
                if !view.selects(&st) {
                    continue;
                }
                if v.pred.nullable() && crate::expr::virtual_column(v.pred, &st).is_none() {
                    continue;
                }
                let mut out = row.clone();
                match &v.object {
                    PObj::Column(c) => {
                        let col = match v.pred {
                            tm_exec::virtual_pred::VirtualPred::AddedAt
                            | tm_exec::virtual_pred::VirtualPred::RetractedAt => {
                                let t = crate::expr::virtual_column(v.pred, &st);
                                t.and_then(|t| self.store.tx_instant(t).ok().flatten())
                            }
                            _ => crate::expr::virtual_column(v.pred, &st),
                        };
                        if col != Some(*c) {
                            continue;
                        }
                    }
                    PObj::Var(_) => {
                        let ov = self.virtual_object(v.pred, &st)?;
                        match obj_idx {
                            Some(i) => {
                                if sv::cond(&sv::eq(&ov, &row[i])) != Some(true) {
                                    continue;
                                }
                            }
                            None => {
                                if let (PTerm::Var(_), None) = (&v.subject, subject_col) {
                                    out.push(V::Integer(st.eid));
                                }
                                out.push(ov);
                                rows.push(out);
                                continue;
                            }
                        }
                    }
                }
                if let (PTerm::Var(_), None) = (&v.subject, subject_col) {
                    out.push(V::Integer(st.eid));
                }
                rows.push(out);
            }
        }
        for (var, c) in new_cols {
            acc.vars.push(var);
            acc.cols.push(c);
        }
        acc.rows = rows;
        Ok(acc)
    }

    fn volatile(&self, v: &tm_exec::plan::PVolatile) -> Result<Rel> {
        let pos = |p: &PTerm| match p {
            PTerm::Id(id) => Some(id.raw()),
            PTerm::Var(_) => None,
        };
        let (s, o) = (pos(&v.s), pos(&v.o));
        let now = View::NOW;
        let mut pairs: Vec<(i64, i64)> = Vec::new();
        for r in now.scan(self.store, Pattern::new(s, Some(v.p.raw()), o))? {
            let r = r?;
            pairs.push((r.s, r.o));
        }
        for (vs, key, row) in self.store.volatile_all()? {
            if key != v.p.raw() || s.is_some_and(|s| s != vs) || o.is_some_and(|o| o != row.value) {
                continue;
            }
            let shadowed = now
                .scan(self.store, Pattern::new(Some(vs), Some(key), None))?
                .next()
                .is_some();
            if !shadowed {
                pairs.push((vs, row.value));
            }
        }
        let mut rel = Rel::default();
        let mut sidx = None;
        if let PTerm::Var(var) = &v.s {
            sidx = Some(rel.vars.len());
            rel.vars.push(var.clone());
            rel.cols.push(Col::term(false));
        }
        let mut oidx = None;
        if let PTerm::Var(var) = &v.o {
            match rel.idx(var) {
                Some(_) => {}
                None => {
                    oidx = Some(rel.vars.len());
                    rel.vars.push(var.clone());
                    rel.cols.push(Col::term(false));
                }
            }
        }
        let same = matches!((&v.s, &v.o), (PTerm::Var(a), PTerm::Var(b)) if a == b);
        for (s, o) in pairs {
            if same && s != o {
                continue;
            }
            let mut row = Vec::new();
            if sidx.is_some() {
                row.push(V::Integer(s));
            }
            if oidx.is_some() {
                row.push(V::Integer(o));
            }
            rel.rows.push(row);
        }
        Ok(rel)
    }

    fn spec_of(text: &str) -> Result<tm_core::ViewSpec> {
        tm_exec::path::view::parse_view_arg(text)
            .map(|(s, _)| s)
            .map_err(|m| Error::invalid_query(format!("view: {m}")))
    }

    fn text(&self, t: &tm_exec::plan::PText) -> Result<Rel> {
        let spec = Self::spec_of(&t.view_text)?;
        let q = TextQuery {
            text: t.query.clone(),
            mode: tm_core::TextMode::from_name(t.mode).unwrap_or(tm_core::TextMode::All),
            graphs: t.graphs.clone(),
            limit: t.limit.map(|n| n as usize),
            ..TextQuery::default()
        };
        let hits =
            tm_store::search::text_search(self.store, &spec, &q, &TermCache::new(64), false)?;
        self.note(format!("text recall {:?} ({} hits)", t.query, hits.len()));
        let mut rel = recall_rel(
            &t.eid,
            [
                (t.score.as_ref(), VClass::Double, false),
                (t.rank.as_ref(), VClass::Int, false),
                (t.confidence.as_ref(), VClass::Double, true),
            ],
        );
        for h in hits {
            let mut row = vec![V::Integer(h.eid.oid().raw())];
            for (present, v) in [
                (t.score.is_some(), V::Real(h.lexical)),
                (t.rank.is_some(), V::Integer(h.rank as i64)),
                (
                    t.confidence.is_some(),
                    h.evidence.confidence.map_or(V::Null, V::Real),
                ),
            ] {
                if present {
                    row.push(v);
                }
            }
            rel.rows.push(row);
        }
        Ok(rel)
    }

    /// A Datalog goal call: evaluated once per execution through the engine's
    /// provider (a second evaluation of the same call reuses the relation), its
    /// rows a relation like the SQL route's `tm_datalog` rows.
    // @tg: implements:: [[openspec:datalog-query-calls#Evaluated once per query]] {since: "2026-10-28"}
    fn datalog_call(&self, t: &tm_exec::plan::PDatalog) -> Result<Rel> {
        use tm_exec::datalog::{run_call, spec_text, DlProgram};
        use tm_exec::plan::PDlProgram;
        let key = spec_text(t);
        if let Some(r) = self.datalog_calls.borrow().get(&key) {
            return Ok((**r).clone());
        }
        let provider = self
            .datalog
            .as_ref()
            .ok_or_else(tm_exec::datalog::no_provider)?;
        let spec = Self::spec_of(&t.view_text)?;
        let program = match &t.program {
            PDlProgram::Stored(i) => DlProgram::Stored(i.clone()),
            PDlProgram::Text(x) => DlProgram::Text(x.clone()),
        };
        let names: Vec<String> = t.outputs.iter().map(|(g, _)| g.clone()).collect();
        let rows = run_call(
            provider.as_ref(),
            self.store,
            &spec,
            &program,
            &t.goal,
            &names,
        )?;
        self.note(format!("datalog call {:?} ({} rows)", t.goal, rows.len()));
        let mut rel = Rel::default();
        for (_, v) in &t.outputs {
            rel.vars.push(v.clone());
            rel.cols.push(Col::term(false));
        }
        if let Some((f, to)) = &t.valid {
            for v in [f, to] {
                rel.vars.push(v.clone());
                rel.cols.push(Col::term(true));
            }
        }
        for r in rows {
            let mut row: Vec<V> = r.ids.iter().map(|id| V::Integer(id.raw())).collect();
            if t.valid.is_some() {
                for id in [r.from, r.to] {
                    row.push(id.map_or(V::Null, |i| V::Integer(i.raw())));
                }
            }
            rel.rows.push(row);
        }
        self.datalog_calls
            .borrow_mut()
            .insert(key, std::rc::Rc::new(rel.clone()));
        Ok(rel)
    }

    fn model_id(&self, iri: &str) -> Result<ObjectId> {
        tm_store::dict::encode(self.store, &Value::iri(iri))?.ok_or_else(|| {
            Error::invalid_query(format!("<{iri}> is not a defined embedding model"))
        })
    }

    fn vector(&self, t: &tm_exec::plan::PVector) -> Result<Rel> {
        let spec = Self::spec_of(&t.view_text)?;
        let vector = tm_core::vector::parse_vector(&t.vector)
            .map_err(|m| Error::invalid_query(format!("vector: {m}")))?;
        let q = VectorQuery {
            model: Some(self.model_id(&t.model)?),
            vector,
            graphs: t.graphs.clone(),
            limit: t.limit.map(|n| n as usize),
            predicates: t.predicates.clone(),
            ..VectorQuery::default()
        };
        let res =
            tm_store::search::vector_search(self.store, &spec, &q, &TermCache::new(64), false)?;
        self.note(format!(
            "vector recall <{}> ({} hits)",
            t.model,
            res.hits.len()
        ));
        let mut rel = recall_rel(
            &t.eid,
            [
                (t.similarity.as_ref(), VClass::Double, false),
                (t.rank.as_ref(), VClass::Int, false),
                (t.confidence.as_ref(), VClass::Double, true),
            ],
        );
        for h in res.hits {
            let mut row = vec![V::Integer(h.eid.oid().raw())];
            for (present, v) in [
                (t.similarity.is_some(), V::Real(h.similarity)),
                (t.rank.is_some(), V::Integer(h.rank as i64)),
                (
                    t.confidence.is_some(),
                    h.evidence.confidence.map_or(V::Null, V::Real),
                ),
            ] {
                if present {
                    row.push(v);
                }
            }
            rel.rows.push(row);
        }
        Ok(rel)
    }

    fn recall(&self, t: &tm_exec::plan::PRecall) -> Result<Rel> {
        let spec = Self::spec_of(&t.view_text)?;
        let mode = tm_core::TextMode::from_name(t.mode).unwrap_or(tm_core::TextMode::All);
        let text = t.text.as_ref().map(|q| TextQuery {
            text: q.clone(),
            mode,
            graphs: t.graphs.clone(),
            ..TextQuery::default()
        });
        let vector = match (&t.model, &t.vector) {
            (Some(m), Some(v)) => Some(VectorQuery {
                graphs: t.graphs.clone(),
                ..VectorQuery::new(
                    self.model_id(m)?,
                    tm_core::vector::parse_vector(v)
                        .map_err(|e| Error::invalid_query(format!("vector: {e}")))?,
                )
            }),
            (None, None) => None,
            _ => {
                return Err(Error::invalid_query(
                    "recall's vector channel needs both a model and a vector",
                ))
            }
        };
        let q = RecallQuery {
            text,
            vector,
            limit: t.limit.map(|n| n as usize),
            expand: t.expand as usize,
            ..RecallQuery::default()
        };
        let hits = tm_store::search::recall(self.store, &spec, &q, &TermCache::new(64), false)?;
        let mut rel = Rel {
            vars: vec![t.eid.clone()],
            cols: vec![Col::term(false)],
            ..Rel::default()
        };
        for (v, c) in [
            (
                t.score.as_ref(),
                Col {
                    dom: Dom::Computed(VClass::Double),
                    mm: false,
                    null: false,
                },
            ),
            (
                t.rank.as_ref(),
                Col {
                    dom: Dom::Computed(VClass::Int),
                    mm: false,
                    null: false,
                },
            ),
            (t.via.as_ref(), Col::term(true)),
        ] {
            if let Some(v) = v {
                rel.vars.push(v.clone());
                rel.cols.push(c);
            }
        }
        for h in hits {
            let mut row = vec![V::Integer(h.eid.oid().raw())];
            if t.score.is_some() {
                row.push(V::Real(h.score));
            }
            if t.rank.is_some() {
                row.push(V::Integer(h.rank as i64));
            }
            if t.via.is_some() {
                row.push(h.via.map_or(V::Null, |e| V::Integer(e.oid().raw())));
            }
            rel.rows.push(row);
        }
        Ok(rel)
    }
}

fn recall_rel(eid: &Var, extra: [(Option<&Var>, VClass, bool); 3]) -> Rel {
    let mut rel = Rel {
        vars: vec![eid.clone()],
        cols: vec![Col::term(false)],
        ..Rel::default()
    };
    for (v, c, mm) in extra {
        if let Some(v) = v {
            rel.vars.push(v.clone());
            rel.cols.push(Col {
                dom: Dom::Computed(c),
                mm,
                null: false,
            });
        }
    }
    rel
}

fn values(v: &PValues) -> Rel {
    if v.vars.is_empty() {
        return Rel {
            rows: vec![Vec::new(); v.rows.len().max(1)],
            ..Rel::default()
        };
    }
    let mut rel = Rel {
        vars: v.vars.clone(),
        ..Rel::default()
    };
    for i in 0..v.vars.len() {
        let cells: Vec<&Option<Cell>> = v.rows.iter().map(|r| &r[i]).collect();
        let dom = if cells.iter().any(|c| matches!(c, Some(Cell::Int(_)))) {
            Dom::Computed(VClass::Int)
        } else if cells.iter().any(|c| matches!(c, Some(Cell::EmptyList))) {
            Dom::List(Box::new(Dom::Term))
        } else {
            Dom::Term
        };
        rel.cols.push(Col {
            dom,
            mm: cells.iter().any(|c| c.is_none()),
            null: false,
        });
    }
    for r in &v.rows {
        rel.rows.push(
            r.iter()
                .map(|c| match c {
                    None => V::Null,
                    Some(Cell::Id(id)) => V::Integer(id.raw()),
                    Some(Cell::Int(i)) => V::Integer(*i),
                    Some(Cell::EmptyList) => V::Text("[]".into()),
                })
                .collect(),
        );
    }
    rel
}

/// Orders two sort keys with the NULL placement of the direction.
fn order_key(a: &V, b: &V, desc: bool, nulls_first: bool) -> Ordering {
    match (a.is_null(), b.is_null()) {
        (true, true) => Ordering::Equal,
        (true, false) => {
            if nulls_first {
                Ordering::Less
            } else {
                Ordering::Greater
            }
        }
        (false, true) => {
            if nulls_first {
                Ordering::Greater
            } else {
                Ordering::Less
            }
        }
        _ => {
            let o = sv::order(a, b);
            if desc {
                o.reverse()
            } else {
                o
            }
        }
    }
}

fn slice_rows<T>(rows: Vec<T>, skip: Option<u64>, limit: Option<u64>) -> Vec<T> {
    let it = rows.into_iter().skip(skip.unwrap_or(0) as usize);
    match limit {
        Some(l) => it.take(l as usize).collect(),
        None => it.collect(),
    }
}

/// The variables the paths among `inputs` bind.
fn path_outputs(inputs: &[Node]) -> Vec<String> {
    let mut out = Vec::new();
    for n in inputs {
        if let Node::Path(p) = n {
            if let PTerm::Var(v) = &p.other {
                out.push(format!("{v:?}"));
            }
            if let tm_exec::plan::PGraphs::Var(v) = &p.graphs {
                out.push(format!("{v:?}"));
            }
            out.extend(p.bind_path.iter().map(|v| format!("{v:?}")));
            out.extend(p.bind_arrival.iter().map(|v| format!("{v:?}")));
        }
    }
    out
}

/// Whether `e` mentions any of `vars` (as their debug forms), anywhere in it,
/// including inside existence tests. Conservative: a false positive only
/// keeps a conjunct where it was.
pub(crate) fn mentions_any(e: &PExpr, vars: &[String]) -> bool {
    if vars.is_empty() {
        return false;
    }
    let text = format!("{e:?}");
    vars.iter().any(|v| {
        text.match_indices(v.as_str()).any(|(i, _)| {
            !text[i + v.len()..]
                .chars()
                .next()
                .is_some_and(|c| c.is_alphanumeric() || c == '_')
        })
    })
}

/// The variables a pattern binds.
fn triple_vars(t: &PTriple) -> impl Iterator<Item = &Var> {
    [&t.s, &t.p, &t.o]
        .into_iter()
        .filter_map(PTerm::var)
        .chain(t.eid.as_ref())
}

/// The patterns grouped into connected components by shared variables, in the
/// order of their first pattern.
fn components<'t>(pats: &[&'t PTriple]) -> Vec<Vec<&'t PTriple>> {
    let mut comp: Vec<usize> = (0..pats.len()).collect();
    fn root(comp: &mut [usize], mut i: usize) -> usize {
        while comp[i] != i {
            comp[i] = comp[comp[i]];
            i = comp[i];
        }
        i
    }
    for i in 0..pats.len() {
        for j in i + 1..pats.len() {
            if triple_vars(pats[i]).any(|v| triple_vars(pats[j]).any(|w| w == v)) {
                let (a, b) = (root(&mut comp, i), root(&mut comp, j));
                comp[b.max(a)] = a.min(b);
            }
        }
    }
    let mut out: Vec<(usize, Vec<&PTriple>)> = Vec::new();
    for (i, p) in pats.iter().enumerate() {
        let r = root(&mut comp, i);
        match out.iter_mut().find(|(k, _)| *k == r) {
            Some((_, v)) => v.push(p),
            None => out.push((r, vec![p])),
        }
    }
    out.into_iter().map(|(_, v)| v).collect()
}

/// A pattern, possibly under filters and a projection that keeps all of its
/// variables, with those filters: it can join as a plain pattern.
fn lift(n: &Node) -> Option<(&PTriple, Vec<&PExpr>)> {
    match n {
        Node::Triple(t) => Some((t, Vec::new())),
        Node::Filter(i, c) => {
            let (t, mut cs) = lift(i)?;
            cs.push(c);
            Some((t, cs))
        }
        Node::Project(i, vars, false) => {
            let (t, cs) = lift(i)?;
            let tv: HashSet<&Var> = triple_vars(t).collect();
            let pv: HashSet<&Var> = vars.iter().collect();
            (tv == pv).then_some((t, cs))
        }
        _ => None,
    }
}
