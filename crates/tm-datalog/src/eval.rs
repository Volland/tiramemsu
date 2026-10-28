//! Semi-naive evaluation over interval sets (`lat.md/datalog#Interval Sets
//! Through Joins`, `#Sorted Columnar Relations`, `#Provenance On Demand`).
//!
//! Strata run in order. A recursive stratum first runs every rule over full
//! relations, then repeats rule variants in which one in-stratum atom reads
//! the delta of the last iteration, until no tuple's set grows. Each variant
//! is planned greedily (the delta first, then the cheapest atom with the
//! filters, negations, builtins and aggregates placed as soon as their
//! variables are bound) and executed as nested index lookups that intersect
//! interval sets as they go.

use std::collections::BTreeSet;

use rustc_hash::{FxHashMap, FxHashSet};
use smallvec::SmallVec;
use tm_core::store::ReadStore;
use tm_core::{budget, Error, Result, TxId, Valid, Value};

use crate::interval::{IvSet, NEG_INF, POS_INF};
use crate::program::{
    ready, Agg, BKind, CAtom, CExpr, Elem, Program, Rule, RuleKind, Slot, Source, Var,
};
use crate::rel::Rel;
use crate::source::{Pat4, Stored};
use crate::syntax::{AggFn, CmpOp, TempOp, Window};
use crate::terms::{self, Id, Terms};

const UNBOUND: Id = i64::MIN;
const MORSEL: usize = 1024;

/// Limits of a run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Iterations of one stratum (default 100 000).
    pub max_iterations: u64,
    /// Facts derived in total.
    pub max_facts: Option<u64>,
    /// Interval fragments held in total.
    pub max_fragments: Option<u64>,
    /// Facts of strata that recurse through value invention.
    pub max_new_values: Option<u64>,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_iterations: 100_000,
            max_facts: None,
            max_fragments: None,
            max_new_values: None,
        }
    }
}

/// A limit that stopped a run.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Limit {
    /// `max_iterations`.
    Iterations,
    /// `max_facts`.
    Facts,
    /// `max_fragments`.
    Fragments,
    /// `max_new_values`.
    NewValues,
}

impl Limit {
    /// Its name.
    pub fn name(self) -> &'static str {
        match self {
            Limit::Iterations => "Iterations",
            Limit::Facts => "Facts",
            Limit::Fragments => "Fragments",
            Limit::NewValues => "NewValues",
        }
    }
}

/// Engine options.
#[derive(Clone, Debug)]
pub struct EvalOptions {
    /// True when valid time is ignored.
    pub atemporal: bool,
    /// The window `[from, to)` that recursive temporal strata are clipped to.
    pub window: Option<(i64, i64)>,
    /// Limits.
    pub limits: Limits,
    /// Record derivations.
    pub proof: bool,
    /// Deltas larger than this run as parallel morsels (0: never).
    pub parallel_threshold: usize,
    /// The pinned clock (`now()`).
    pub now: i64,
}

/// One premise of a derivation.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Prem {
    /// A stored statement.
    Stmt(Id),
    /// A row of a derived relation.
    Row(u32, u32),
}

/// A derivation: the rule and its premises.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Deriv {
    /// The rule (index into `Program::rules`).
    pub rule: u32,
    /// The premises.
    pub premises: Vec<Prem>,
}

/// A match of an atom: values per slot, interval set, premise.
type Match = (SmallVec<[Id; 4]>, IvSet, Prem);
/// A group of matches: values, the union of their sets, every premise.
type Group = (SmallVec<[Id; 4]>, IvSet, Vec<Prem>);

struct Out {
    rel: u32,
    tuple: SmallVec<[Id; 4]>,
    iv: IvSet,
    deriv: Option<Deriv>,
}

/// The part of a run that worker threads share.
pub struct Shared<'p, 's> {
    /// The program.
    pub prog: &'p Program,
    /// The stored side.
    pub stored: Stored<'s>,
    /// The term table.
    pub terms: &'s Terms,
    /// Constant ids.
    pub consts: Vec<Id>,
    /// Derived relations.
    pub rels: Vec<Rel>,
    /// Options.
    pub opts: EvalOptions,
}

/// One run of a program.
pub struct Engine<'p, 's> {
    /// What threads share.
    pub sh: Shared<'p, 's>,
    store: &'s dyn ReadStore,
    sync: Option<&'s (dyn ReadStore + Sync)>,
    /// Recorded derivations.
    pub derivs: Vec<Deriv>,
    /// The limits reached.
    pub limits_hit: BTreeSet<Limit>,
    /// Explain lines.
    pub plan: Vec<String>,
    /// Iterations over all strata.
    pub iterations: u64,
    facts: u64,
    new_values: u64,
    estimates: FxHashMap<(Option<Id>, u8), f64>,
}

/// Interval set of a derived row under an atom's operators (innermost first).
fn apply_ops(iv: &IvSet, ops: &[(TempOp, Window)]) -> IvSet {
    let mut out = iv.clone();
    for (op, w) in ops.iter().rev() {
        out = out.apply(*op, *w);
    }
    out
}

impl<'p, 's> Engine<'p, 's> {
    /// A run of `prog` over `store`.
    pub fn new(
        prog: &'p Program,
        store: &'s dyn ReadStore,
        stored: Stored<'s>,
        terms: &'s Terms,
        opts: EvalOptions,
    ) -> Result<Engine<'p, 's>> {
        let mut consts = Vec::with_capacity(prog.consts.len());
        for v in &prog.consts {
            consts.push(terms.intern(store, v)?);
        }
        Ok(Engine {
            sh: Shared {
                prog,
                stored,
                terms,
                consts,
                rels: prog.rels.iter().map(|r| Rel::new(r.arity)).collect(),
                opts,
            },
            store,
            sync: store.as_sync(),
            derivs: Vec::new(),
            limits_hit: BTreeSet::new(),
            plan: Vec::new(),
            iterations: 0,
            facts: 0,
            new_values: 0,
            estimates: FxHashMap::default(),
        })
    }

    /// The number of facts derived (new tuples or new parts of their sets).
    pub fn derived(&self) -> u64 {
        self.facts
    }

    /// The store.
    pub fn store(&self) -> &'s dyn ReadStore {
        self.store
    }

    /// Runs every stratum; stops early when a limit is reached.
    pub fn run(&mut self) -> Result<()> {
        for si in 0..self.sh.prog.strata.len() {
            if !self.limits_hit.is_empty() {
                break;
            }
            self.stratum(si)?;
        }
        Ok(())
    }

    fn stratum(&mut self, si: usize) -> Result<()> {
        let st = &self.sh.prog.strata[si];
        let in_stratum: FxHashSet<u32> = st.rels.iter().copied().collect();
        let names: Vec<String> = st
            .rels
            .iter()
            .map(|&r| self.sh.prog.rels[r as usize].name.to_string())
            .collect();
        // iteration 0: every rule over full relations
        let mut outs = Vec::new();
        for &ri in &st.rules {
            outs.extend(self.eval_rule(ri, None, si)?);
        }
        self.merge(outs, si)?;
        let mut iters = 1u64;
        if st.recursive {
            loop {
                let grew = st
                    .rels
                    .iter()
                    .any(|&r| !self.sh.rels[r as usize].delta.is_empty());
                if !grew || !self.limits_hit.is_empty() {
                    break;
                }
                if iters >= self.sh.opts.limits.max_iterations {
                    self.limits_hit.insert(Limit::Iterations);
                    break;
                }
                iters += 1;
                budget::check()?;
                let mut outs = Vec::new();
                for &ri in &st.rules {
                    let rule = &self.sh.prog.rules[ri];
                    let naive = rule.body.iter().any(|e| match e {
                        Elem::Binary { lhs, rhs, .. } => [lhs, rhs]
                            .iter()
                            .any(|a| a.rel().is_some_and(|r| in_stratum.contains(&r))),
                        _ => false,
                    });
                    if naive {
                        outs.extend(self.eval_rule(ri, None, si)?);
                        continue;
                    }
                    for (k, e) in rule.body.iter().enumerate() {
                        if let Elem::Pos(a) = e {
                            if let Some(r) = a.rel() {
                                if in_stratum.contains(&r)
                                    && !self.sh.rels[r as usize].delta.is_empty()
                                {
                                    outs.extend(self.eval_rule(ri, Some(k), si)?);
                                }
                            }
                        }
                    }
                }
                self.merge(outs, si)?;
            }
        }
        self.iterations += iters;
        let sizes: Vec<String> = st
            .rels
            .iter()
            .map(|&r| {
                format!(
                    "{} = {}",
                    self.sh.prog.rels[r as usize].name,
                    self.sh.rels[r as usize].len()
                )
            })
            .collect();
        self.plan.push(format!(
            "stratum {si} [{}]{}: {iters} iteration(s), {}",
            names.join(", "),
            if st.recursive { " recursive" } else { "" },
            sizes.join(", ")
        ));
        for &r in &st.rels {
            self.sh.rels[r as usize].delta.clear();
        }
        Ok(())
    }

    // @tg: implements:: [[openspec:datalog-temporal#Windows and recursion through time]] {since: "2026-10-28"}
    // @tg: implements:: [[openspec:datalog-evaluation#Limits and completion]] {since: "2026-10-28"}
    // @tg: implements:: [[openspec:datalog-language#Value invention]] {since: "2026-10-28"}
    fn merge(&mut self, outs: Vec<Out>, si: usize) -> Result<()> {
        let st = &self.sh.prog.strata[si];
        for &r in &st.rels {
            self.sh.rels[r as usize].delta.clear();
        }
        let clip = if st.metric_recursion {
            self.sh.opts.window
        } else {
            None
        };
        for o in outs {
            let mut iv = o.iv;
            if let Some((a, b)) = clip {
                iv = iv.clip(a, b);
            }
            if self.sh.prog.rels[o.rel as usize].magic {
                iv = IvSet::all();
            }
            let seed = o
                .deriv
                .as_ref()
                .is_some_and(|d| self.sh.prog.rules[d.rule as usize].kind == RuleKind::Seed);
            let rel = &mut self.sh.rels[o.rel as usize];
            let Some((row, new)) = rel.insert(&o.tuple, &iv, self.sh.opts.proof) else {
                continue;
            };
            if seed {
                let r = row as usize;
                rel.stored[r] = rel.stored[r].union(&new);
            }
            if self.sh.opts.proof {
                if let Some(d) = o.deriv {
                    let ix = self.derivs.len() as u32;
                    self.derivs.push(d);
                    rel.prov[row as usize].push((new.clone(), ix));
                }
            }
            match rel.delta.last_mut() {
                Some((r, part)) if *r == row => *part = part.union(&new),
                _ => rel.delta.push((row, new)),
            }
            self.facts += 1;
            if st.invention_recursion {
                self.new_values += 1;
                if self
                    .sh
                    .opts
                    .limits
                    .max_new_values
                    .is_some_and(|m| self.new_values >= m)
                {
                    self.limits_hit.insert(Limit::NewValues);
                    break;
                }
            }
            if self
                .sh
                .opts
                .limits
                .max_facts
                .is_some_and(|m| self.facts >= m)
            {
                self.limits_hit.insert(Limit::Facts);
                break;
            }
        }
        if let Some(m) = self.sh.opts.limits.max_fragments {
            if self.sh.rels.iter().map(|r| r.fragments).sum::<u64>() > m {
                self.limits_hit.insert(Limit::Fragments);
            }
        }
        // a delta row may have been pushed twice in one merge; keep one entry each
        for &r in &st.rels {
            let d = &mut self.sh.rels[r as usize].delta;
            if d.len() > 1 {
                let mut seen: FxHashMap<u32, usize> = FxHashMap::default();
                let mut out: Vec<(u32, IvSet)> = Vec::with_capacity(d.len());
                for (row, part) in d.drain(..) {
                    match seen.get(&row) {
                        Some(&i) => out[i].1 = out[i].1.union(&part),
                        None => {
                            seen.insert(row, out.len());
                            out.push((row, part));
                        }
                    }
                }
                *d = out;
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------ planning

    fn slot_bound(s: &Slot, bound: &BTreeSet<Var>) -> bool {
        match s {
            Slot::Const(_) => true,
            Slot::Var(v) => bound.contains(v),
        }
    }

    fn atom_estimate(&mut self, a: &CAtom, bound: &BTreeSet<Var>) -> Result<f64> {
        match &a.src {
            Source::Derived { rel, args } => {
                let n = self.sh.rels[*rel as usize].len().max(1) as f64;
                let k = args.iter().filter(|s| Self::slot_bound(s, bound)).count();
                if k == args.len() {
                    return Ok(1.0);
                }
                Ok((n / 10f64.powi(k as i32)).max(1.0))
            }
            Source::Stored(p) => {
                let pat: Pat4 = [
                    Self::slot_bound(&p.s, bound).then_some(0),
                    match p.p {
                        Slot::Const(c) => Some(self.sh.consts[c as usize]),
                        Slot::Var(v) if bound.contains(&v) => Some(0),
                        _ => None,
                    },
                    Self::slot_bound(&p.o, bound).then_some(0),
                    p.e.as_ref()
                        .filter(|s| Self::slot_bound(s, bound))
                        .map(|_| 0),
                ];
                let m =
                    pat.iter()
                        .enumerate()
                        .fold(0u8, |m, (i, x)| if x.is_some() { m | (1 << i) } else { m });
                let key = (pat[1].filter(|_| matches!(p.p, Slot::Const(_))), m);
                if let Some(e) = self.estimates.get(&key) {
                    return Ok(*e);
                }
                let probe: Pat4 = [
                    pat[0].map(|_| 1),
                    key.0,
                    pat[2].map(|_| 1),
                    pat[3].map(|_| 1),
                ];
                let e = self.sh.stored.estimate_bound(self.store, &probe)?;
                self.estimates.insert(key, e);
                Ok(e)
            }
        }
    }

    /// The order of `rule`'s body for a variant whose element `delta` reads the
    /// delta, or `None` for a run over full relations.
    fn plan_rule(
        &mut self,
        rule: &Rule,
        delta: Option<usize>,
        start: &BTreeSet<Var>,
    ) -> Result<Vec<usize>> {
        let mut bound = start.clone();
        let mut left: Vec<usize> = (0..rule.body.len()).collect();
        let mut order = Vec::new();
        if let Some(d) = delta {
            left.retain(|&k| k != d);
            order.push(d);
            if let Some(b) = ready(&rule.body[d], &bound, rule) {
                bound.extend(b);
            }
        }
        while !left.is_empty() {
            // filters first, in written order
            let cheap = left.iter().position(|&k| {
                !matches!(rule.body[k], Elem::Pos(_) | Elem::Binary { .. })
                    && ready(&rule.body[k], &bound, rule).is_some()
            });
            if let Some(i) = cheap {
                let k = left.remove(i);
                bound.extend(ready(&rule.body[k], &bound, rule).unwrap_or_default());
                order.push(k);
                continue;
            }
            let mut best: Option<(f64, usize)> = None;
            for (i, &k) in left.iter().enumerate() {
                let est = match &rule.body[k] {
                    Elem::Pos(a) => self.atom_estimate(a, &bound)?,
                    Elem::Binary { rhs, .. } => self.atom_estimate(rhs, &bound)?,
                    _ => continue,
                };
                if best.is_none_or(|(b, _)| est < b) {
                    best = Some((est, i));
                }
            }
            let Some((_, i)) = best else {
                return Err(Error::DatalogUnsafe {
                    rule: 0,
                    message: "no element of the body can run".into(),
                });
            };
            let k = left.remove(i);
            bound.extend(ready(&rule.body[k], &bound, rule).unwrap_or_default());
            order.push(k);
        }
        Ok(order)
    }

    fn eval_rule(&mut self, ri: usize, delta: Option<usize>, si: usize) -> Result<Vec<Out>> {
        let rule = &self.sh.prog.rules[ri];
        let order = self.plan_rule(rule, delta, &BTreeSet::new())?;
        if self.iterations == 0 || delta.is_none() {
            let desc: Vec<String> = order
                .iter()
                .map(|&k| describe(&rule.body[k], delta == Some(k)))
                .collect();
            let line = format!(
                "  rule {} -> {}: {}",
                ri,
                self.sh.prog.rels[rule.head as usize].name,
                desc.join(", ")
            );
            if !self.plan.contains(&line) {
                self.plan.push(line);
            }
        }
        let _ = si;
        let none = Vec::new();
        let deltas: &[(u32, IvSet)] = match delta.map(|k| &rule.body[k]) {
            Some(Elem::Pos(a)) => &self.sh.rels[a.rel().expect("delta atom") as usize].delta,
            _ => &none,
        };
        let cx = Cx {
            sh: &self.sh,
            store: self.store,
            rule,
            ri: ri as u32,
            order: &order,
            delta,
        };
        #[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
        if let (Some(sync), Some(_)) = (self.sync, delta) {
            let th = self.sh.opts.parallel_threshold;
            if th > 0 && deltas.len() > th {
                return cx.parallel(sync, deltas);
            }
        }
        let mut out = Vec::new();
        let mut b = vec![UNBOUND; rule.vars.len()];
        let mut prem = Vec::new();
        cx.step(0, &mut b, &IvSet::all(), &mut prem, deltas, &mut out)?;
        Ok(out)
    }
}

fn describe(e: &Elem, delta: bool) -> String {
    let d = if delta { "Δ" } else { "" };
    match e {
        Elem::Pos(a) | Elem::Neg(a) => {
            let neg = if matches!(e, Elem::Neg(_)) {
                "not "
            } else {
                ""
            };
            let ops: String = a
                .ops
                .iter()
                .map(|(o, _)| format!("{} ", o.name()))
                .collect();
            match &a.src {
                Source::Derived { rel, .. } => format!("{neg}{ops}{d}rel{rel}"),
                Source::Stored(_) => format!("{neg}{ops}stored"),
            }
        }
        Elem::Cmp(..) => "filter".into(),
        Elem::Builtin(k, _) => format!("{k:?}").to_lowercase(),
        Elem::Agg(a) => format!("{:?}", a.func).to_lowercase(),
        Elem::Binary { until, .. } => if *until { "until" } else { "since" }.into(),
    }
}

/// A computed value: an id, or a value not yet interned.
enum Val {
    Id(Id),
    V(Value),
}

/// The execution of one rule variant.
struct Cx<'e, 'p, 's> {
    sh: &'e Shared<'p, 's>,
    store: &'e dyn ReadStore,
    rule: &'e Rule,
    ri: u32,
    order: &'e [usize],
    delta: Option<usize>,
}

impl Cx<'_, '_, '_> {
    #[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
    // @tg: implements:: [[openspec:datalog-evaluation#Deterministic results]] {since: "2026-10-28"}
    fn parallel(&self, sync: &(dyn ReadStore + Sync), deltas: &[(u32, IvSet)]) -> Result<Vec<Out>> {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use std::sync::Mutex;
        let n = deltas.len().div_ceil(MORSEL);
        let threads = std::thread::available_parallelism()
            .map_or(1, |t| t.get())
            .min(n);
        let next = AtomicUsize::new(0);
        let failed = AtomicBool::new(false);
        let results: Vec<Mutex<Option<Result<Vec<Out>>>>> =
            (0..n).map(|_| Mutex::new(None)).collect();
        let interrupt = budget::interrupt();
        let (sh, rule, ri, order, delta) = (self.sh, self.rule, self.ri, self.order, self.delta);
        std::thread::scope(|sc| {
            for _ in 0..threads {
                sc.spawn(|| {
                    let cx = Cx {
                        sh,
                        store: sync,
                        rule,
                        ri,
                        order,
                        delta,
                    };
                    let drain = || loop {
                        if failed.load(Ordering::Relaxed) {
                            break;
                        }
                        let k = next.fetch_add(1, Ordering::Relaxed);
                        if k >= n {
                            break;
                        }
                        let chunk = &deltas[k * MORSEL..((k + 1) * MORSEL).min(deltas.len())];
                        let mut out = Vec::new();
                        let mut b = vec![UNBOUND; cx.rule.vars.len()];
                        let mut prem = Vec::new();
                        let r = cx
                            .step(0, &mut b, &IvSet::all(), &mut prem, chunk, &mut out)
                            .map(|_| out);
                        if r.is_err() {
                            failed.store(true, Ordering::Relaxed);
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
        let mut out = Vec::new();
        for r in results {
            match r.into_inner().unwrap_or_else(|p| p.into_inner()) {
                Some(Ok(o)) => out.extend(o),
                Some(Err(e)) => return Err(e),
                None => {}
            }
        }
        budget::check()?;
        Ok(out)
    }

    fn val(&self, s: &Slot, b: &[Id]) -> Id {
        match s {
            Slot::Const(c) => self.sh.consts[*c as usize],
            Slot::Var(v) => b[*v as usize],
        }
    }

    fn value(&self, id: Id) -> Result<Value> {
        self.sh.terms.value(self.store, id)
    }

    fn intern(&self, v: &Value) -> Result<Id> {
        self.sh.terms.intern(self.store, v)
    }

    fn eval(&self, e: &CExpr, b: &[Id]) -> Result<Option<Val>> {
        Ok(match e {
            CExpr::Slot(s) => {
                let id = self.val(s, b);
                (id != UNBOUND).then_some(Val::Id(id))
            }
            CExpr::Bin(l, op, r) => {
                let (Some(l), Some(r)) = (self.eval(l, b)?, self.eval(r, b)?) else {
                    return Ok(None);
                };
                let (l, r) = (self.as_value(l)?, self.as_value(r)?);
                terms::arith(&l, *op, &r).map(Val::V)
            }
            CExpr::Call(name, args) => {
                let mut vs = Vec::with_capacity(args.len());
                for a in args {
                    match self.eval(a, b)? {
                        Some(v) => vs.push(self.as_value(v)?),
                        None => return Ok(None),
                    }
                }
                terms::call(name, &vs, self.sh.opts.now).map(Val::V)
            }
        })
    }

    fn as_value(&self, v: Val) -> Result<Value> {
        match v {
            Val::Id(id) => self.value(id),
            Val::V(v) => Ok(v),
        }
    }

    fn as_id(&self, v: Val) -> Result<Id> {
        match v {
            Val::Id(id) => Ok(id),
            Val::V(v) => self.intern(&v),
        }
    }

    /// The valid interval of a statement id, or the interval an operand value stands for.
    fn interval_of(&self, id: Id) -> Result<Option<Valid>> {
        if let Value::Stmt(e) = self.value(id)? {
            return Ok(self.store.statement(e.oid().raw())?.map(|r| Valid {
                from: r.v_from,
                to: r.v_to,
            }));
        }
        Ok(terms::as_interval(&self.value(id)?))
    }

    /// Binds `s` to `id`, or checks it; returns whether it was newly bound.
    fn bind(s: &Slot, id: Id, b: &mut [Id], consts: &[Id]) -> Option<bool> {
        match s {
            Slot::Const(c) => (consts[*c as usize] == id).then_some(false),
            Slot::Var(v) => {
                let cur = b[*v as usize];
                if cur == UNBOUND {
                    b[*v as usize] = id;
                    Some(true)
                } else {
                    (cur == id).then_some(false)
                }
            }
        }
    }

    fn unbind(slots: &[Slot], newly: &[bool], b: &mut [Id]) {
        for (s, n) in slots.iter().zip(newly) {
            if let (true, Slot::Var(v)) = (n, s) {
                b[*v as usize] = UNBOUND;
            }
        }
    }

    /// The matches of an atom under the current bindings: `(values per slot,
    /// interval set, premise)`, without its operators applied.
    fn matches(&self, a: &CAtom, b: &[Id]) -> Result<Vec<Match>> {
        match &a.src {
            Source::Derived { rel, args } => {
                let r = &self.sh.rels[*rel as usize];
                let mut mask = 0u32;
                let mut key: SmallVec<[Id; 4]> = SmallVec::new();
                for (i, s) in args.iter().enumerate() {
                    let v = self.val(s, b);
                    if v != UNBOUND {
                        mask |= 1 << i;
                        key.push(v);
                    }
                }
                Ok(r.lookup(mask, &key)
                    .into_iter()
                    .map(|row| {
                        (
                            r.tuple(row).iter().copied().collect(),
                            r.ivs[row as usize].clone(),
                            Prem::Row(*rel, row),
                        )
                    })
                    .collect())
            }
            Source::Stored(p) => {
                let g = |s: &Slot| {
                    let v = self.val(s, b);
                    (v != UNBOUND).then_some(v)
                };
                let pat: Pat4 = [g(&p.s), g(&p.p), g(&p.o), p.e.as_ref().and_then(g)];
                Ok(self
                    .sh
                    .stored
                    .matches(self.store, &pat)?
                    .into_iter()
                    .map(|(r, iv)| {
                        let mut vals: SmallVec<[Id; 4]> = smallvec::smallvec![r.s, r.p, r.o];
                        if p.e.is_some() {
                            vals.push(r.e);
                        }
                        let iv = if p.record { IvSet::all() } else { iv };
                        (vals, iv, Prem::Stmt(r.e))
                    })
                    .collect())
            }
        }
    }

    /// The matches grouped by the values of the atom's free variables, each
    /// with the union of its sets and every premise; operators applied after
    /// the union (needed for the always operators and for negation).
    fn groups(&self, a: &CAtom, b: &[Id], skip: &dyn Fn(Var) -> bool) -> Result<Vec<Group>> {
        let slots = a.slots();
        let mut ix: FxHashMap<SmallVec<[Id; 4]>, usize> = FxHashMap::default();
        let mut out: Vec<Group> = Vec::new();
        'm: for (vals, iv, prem) in self.matches(a, b)? {
            // consistency of repeated free variables, and the group key
            let mut seen: SmallVec<[(Var, Id); 4]> = SmallVec::new();
            let mut key: SmallVec<[Id; 4]> = SmallVec::new();
            for (s, &x) in slots.iter().zip(vals.iter()) {
                match s {
                    Slot::Const(c) => {
                        if self.sh.consts[*c as usize] != x {
                            continue 'm;
                        }
                    }
                    Slot::Var(v) => {
                        let cur = b[*v as usize];
                        if cur != UNBOUND {
                            if cur != x {
                                continue 'm;
                            }
                        } else if let Some(&(_, y)) = seen.iter().find(|(w, _)| w == v) {
                            if y != x {
                                continue 'm;
                            }
                        } else {
                            seen.push((*v, x));
                            if !skip(*v) {
                                key.push(x);
                            }
                        }
                    }
                }
            }
            let k = key;
            match ix.get(&k) {
                Some(&i) => {
                    out[i].1 = out[i].1.union(&iv);
                    out[i].2.push(prem);
                }
                None => {
                    ix.insert(k.clone(), out.len());
                    out.push((vals, iv, vec![prem]));
                }
            }
        }
        for g in &mut out {
            g.1 = apply_ops(&g.1, &a.ops);
        }
        Ok(out)
    }

    #[allow(clippy::too_many_arguments)]
    // @tg: implements:: [[openspec:datalog-temporal#Negation over time]] {since: "2026-10-28"}
    fn step(
        &self,
        k: usize,
        b: &mut Vec<Id>,
        iv: &IvSet,
        prem: &mut Vec<Prem>,
        deltas: &[(u32, IvSet)],
        out: &mut Vec<Out>,
    ) -> Result<()> {
        if k == self.order.len() {
            return self.emit(b, iv, prem, out);
        }
        let ek = self.order[k];
        let e = &self.rule.body[ek];
        let consts = &self.sh.consts;
        match e {
            Elem::Pos(a) if self.delta == Some(ek) => {
                let Source::Derived { rel, args } = &a.src else {
                    unreachable!("delta atoms are derived")
                };
                let r = &self.sh.rels[*rel as usize];
                for (row, part) in deltas {
                    budget::poll()?;
                    let tuple = r.tuple(*row);
                    let mut newly = SmallVec::<[bool; 4]>::new();
                    let mut ok = true;
                    for (s, &x) in args.iter().zip(tuple) {
                        match Self::bind(s, x, b, consts) {
                            Some(n) => newly.push(n),
                            None => {
                                newly.push(false);
                                ok = false;
                                break;
                            }
                        }
                    }
                    if ok {
                        let contrib = if a.ops.is_empty() {
                            part.clone()
                        } else {
                            let full = &r.ivs[*row as usize];
                            apply_ops(full, &a.ops)
                                .subtract(&apply_ops(&full.subtract(part), &a.ops))
                        };
                        let iv2 = iv.intersect(&contrib);
                        if !iv2.is_empty() {
                            if self.sh.prog.rels[*rel as usize].magic {
                                self.step(k + 1, b, &iv2, prem, deltas, out)?;
                            } else {
                                prem.push(Prem::Row(*rel, *row));
                                self.step(k + 1, b, &iv2, prem, deltas, out)?;
                                prem.pop();
                            }
                        }
                    }
                    Self::unbind(&args[..newly.len()], &newly, b);
                }
            }
            Elem::Pos(a) if a.ops.is_empty() => {
                let slots = a.slots();
                // read in place: no copy of the matches
                let visit = |vals: &[Id],
                             miv: &IvSet,
                             p: Option<Prem>,
                             b: &mut Vec<Id>,
                             prem: &mut Vec<Prem>,
                             out: &mut Vec<Out>|
                 -> Result<()> {
                    budget::poll()?;
                    let mut newly = SmallVec::<[bool; 4]>::new();
                    let mut ok = true;
                    for (s, &x) in slots.iter().zip(vals.iter()) {
                        match Self::bind(s, x, b, consts) {
                            Some(n) => newly.push(n),
                            None => {
                                newly.push(false);
                                ok = false;
                                break;
                            }
                        }
                    }
                    if ok {
                        let iv2 = iv.intersect(miv);
                        if !iv2.is_empty() {
                            match p {
                                Some(p) => {
                                    prem.push(p);
                                    self.step(k + 1, b, &iv2, prem, deltas, out)?;
                                    prem.pop();
                                }
                                None => self.step(k + 1, b, &iv2, prem, deltas, out)?,
                            }
                        }
                    }
                    Self::unbind(&slots[..newly.len()], &newly, b);
                    Ok(())
                };
                match &a.src {
                    Source::Derived { rel, args } => {
                        let r = &self.sh.rels[*rel as usize];
                        let magic = self.sh.prog.rels[*rel as usize].magic;
                        let mut mask = 0u32;
                        let mut key: SmallVec<[Id; 4]> = SmallVec::new();
                        for (i, s) in args.iter().enumerate() {
                            let x = self.val(s, b);
                            if x != UNBOUND {
                                mask |= 1 << i;
                                key.push(x);
                            }
                        }
                        r.for_rows(mask, &key, |row| {
                            let p = (!magic).then_some(Prem::Row(*rel, row));
                            visit(r.tuple(row), &r.ivs[row as usize], p, b, prem, out)
                        })?;
                    }
                    Source::Stored(sp) => {
                        let g = |s: &Slot| {
                            let x = self.val(s, b);
                            (x != UNBOUND).then_some(x)
                        };
                        let pat: Pat4 = [g(&sp.s), g(&sp.p), g(&sp.o), sp.e.as_ref().and_then(g)];
                        let all = IvSet::all();
                        self.sh
                            .stored
                            .for_matches(self.store, &pat, &mut |r, siv| {
                                let vals = [r.s, r.p, r.o, r.e];
                                let n = if sp.e.is_some() { 4 } else { 3 };
                                let siv = if sp.record { &all } else { siv };
                                visit(&vals[..n], siv, Some(Prem::Stmt(r.e)), b, prem, out)
                            })?;
                    }
                }
            }
            Elem::Pos(a) => {
                let slots = a.slots();
                // stored atoms under operators: union per tuple first
                let grouped = !a.ops.is_empty() && matches!(a.src, Source::Stored(_));
                if grouped {
                    for (vals, giv, prems) in self.groups(a, b, &|_| false)? {
                        budget::poll()?;
                        let mut newly = SmallVec::<[bool; 4]>::new();
                        let mut ok = true;
                        for (s, &x) in slots.iter().zip(vals.iter()) {
                            match Self::bind(s, x, b, consts) {
                                Some(n) => newly.push(n),
                                None => {
                                    newly.push(false);
                                    ok = false;
                                    break;
                                }
                            }
                        }
                        if ok {
                            let iv2 = iv.intersect(&giv);
                            if !iv2.is_empty() {
                                let n = prem.len();
                                prem.extend(prems);
                                self.step(k + 1, b, &iv2, prem, deltas, out)?;
                                prem.truncate(n);
                            }
                        }
                        Self::unbind(&slots[..newly.len()], &newly, b);
                    }
                    return Ok(());
                }
                for (vals, miv, p) in self.matches(a, b)? {
                    budget::poll()?;
                    let mut newly = SmallVec::<[bool; 4]>::new();
                    let mut ok = true;
                    for (s, &x) in slots.iter().zip(vals.iter()) {
                        match Self::bind(s, x, b, consts) {
                            Some(n) => newly.push(n),
                            None => {
                                newly.push(false);
                                ok = false;
                                break;
                            }
                        }
                    }
                    if ok {
                        let aiv = if a.ops.is_empty() {
                            miv
                        } else {
                            apply_ops(&miv, &a.ops)
                        };
                        let iv2 = iv.intersect(&aiv);
                        if !iv2.is_empty() {
                            let demand =
                                matches!(p, Prem::Row(r, _) if self.sh.prog.rels[r as usize].magic);
                            if demand {
                                self.step(k + 1, b, &iv2, prem, deltas, out)?;
                            } else {
                                prem.push(p);
                                self.step(k + 1, b, &iv2, prem, deltas, out)?;
                                prem.pop();
                            }
                        }
                    }
                    Self::unbind(&slots[..newly.len()], &newly, b);
                }
            }
            Elem::Neg(a) => {
                let rule = self.rule;
                let mut union = IvSet::empty();
                for (_, giv, _) in self.groups(a, b, &|v| rule.anon(v))? {
                    union = union.union(&giv);
                }
                let iv2 = iv.subtract(&union);
                if !iv2.is_empty() {
                    self.step(k + 1, b, &iv2, prem, deltas, out)?;
                }
            }
            Elem::Cmp(l, op, r) => self.cmp(l, *op, r, k, b, iv, prem, deltas, out)?,
            Elem::Builtin(kind, args) => self.builtin(kind, args, k, b, iv, prem, deltas, out)?,
            Elem::Agg(agg) => self.aggregate(agg, k, b, iv, prem, deltas, out)?,
            Elem::Binary {
                lhs,
                until,
                window,
                rhs,
            } => {
                let rs = rhs.slots();
                for (rvals, riv, rprem) in self.groups(rhs, b, &|_| false)? {
                    let mut rnew = SmallVec::<[bool; 4]>::new();
                    let mut ok = true;
                    for (s, &x) in rs.iter().zip(rvals.iter()) {
                        match Self::bind(s, x, b, consts) {
                            Some(n) => rnew.push(n),
                            None => {
                                rnew.push(false);
                                ok = false;
                                break;
                            }
                        }
                    }
                    if ok {
                        let ls = lhs.slots();
                        for (lvals, liv, lprem) in self.groups(lhs, b, &|_| false)? {
                            let mut lnew = SmallVec::<[bool; 4]>::new();
                            let mut ok = true;
                            for (s, &x) in ls.iter().zip(lvals.iter()) {
                                match Self::bind(s, x, b, consts) {
                                    Some(n) => lnew.push(n),
                                    None => {
                                        lnew.push(false);
                                        ok = false;
                                        break;
                                    }
                                }
                            }
                            if ok {
                                let t = if *until {
                                    liv.until(&riv, *window)
                                } else {
                                    liv.since(&riv, *window)
                                };
                                let iv2 = iv.intersect(&t);
                                if !iv2.is_empty() {
                                    let n = prem.len();
                                    prem.extend(rprem.iter().copied());
                                    prem.extend(lprem);
                                    self.step(k + 1, b, &iv2, prem, deltas, out)?;
                                    prem.truncate(n);
                                }
                            }
                            Self::unbind(&ls[..lnew.len()], &lnew, b);
                        }
                    }
                    Self::unbind(&rs[..rnew.len()], &rnew, b);
                }
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn cmp(
        &self,
        l: &CExpr,
        op: CmpOp,
        r: &CExpr,
        k: usize,
        b: &mut Vec<Id>,
        iv: &IvSet,
        prem: &mut Vec<Prem>,
        deltas: &[(u32, IvSet)],
        out: &mut Vec<Out>,
    ) -> Result<()> {
        // assignment: `V = expr` with V unbound
        if op == CmpOp::Eq {
            for (target, src) in [(l, r), (r, l)] {
                if let CExpr::Slot(Slot::Var(v)) = target {
                    if b[*v as usize] == UNBOUND {
                        let Some(x) = self.eval(src, b)? else {
                            return Ok(());
                        };
                        let id = self.as_id(x)?;
                        b[*v as usize] = id;
                        let r = self.step(k + 1, b, iv, prem, deltas, out);
                        b[*v as usize] = UNBOUND;
                        return r;
                    }
                }
            }
        }
        let (Some(x), Some(y)) = (self.eval(l, b)?, self.eval(r, b)?) else {
            return Ok(());
        };
        let pass = match (op, &x, &y) {
            (CmpOp::Eq, Val::Id(a), Val::Id(c)) => a == c,
            (CmpOp::Ne, Val::Id(a), Val::Id(c)) => a != c,
            _ => {
                let (xv, yv) = (self.as_value(x)?, self.as_value(y)?);
                match (op, terms::compare(&xv, &yv)) {
                    (CmpOp::Eq, None) => self.intern(&xv)? == self.intern(&yv)?,
                    (CmpOp::Ne, None) => self.intern(&xv)? != self.intern(&yv)?,
                    _ => terms::cmp_values(&xv, op, &yv),
                }
            }
        };
        if pass {
            self.step(k + 1, b, iv, prem, deltas, out)?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn builtin(
        &self,
        kind: &BKind,
        args: &[Slot],
        k: usize,
        b: &mut Vec<Id>,
        iv: &IvSet,
        prem: &mut Vec<Prem>,
        deltas: &[(u32, IvSet)],
        out: &mut Vec<Out>,
    ) -> Result<()> {
        let input = |i: usize| self.val(&args[i], b);
        let outputs: Option<Vec<Value>> = match kind {
            BKind::Valid => match self.value(input(0))? {
                Value::Stmt(e) => self
                    .store
                    .statement(e.oid().raw())?
                    .map(|r| vec![terms::end_value(r.v_from), terms::end_value(r.v_to)]),
                _ => None,
            },
            BKind::Added => match self.value(input(0))? {
                Value::Stmt(e) => self
                    .store
                    .statement(e.oid().raw())?
                    .map(|r| vec![Value::Tx(TxId(r.t_add as u64))]),
                _ => None,
            },
            BKind::TxInstant => match self.value(input(0))? {
                Value::Tx(t) => self
                    .store
                    .tx_instant(t.0 as i64)?
                    .map(|ms| vec![Value::DateTime { ms, tz: Some(0) }]),
                _ => None,
            },
            BKind::Interval => match self.value(input(0))? {
                Value::Stmt(_) => self
                    .interval_of(input(0))?
                    .map(|v| vec![terms::interval_value(v)]),
                _ => None,
            },
            BKind::Start | BKind::End | BKind::Duration => self.interval_of(input(0))?.map(|v| {
                vec![match kind {
                    BKind::Start => terms::end_value(v.from),
                    BKind::End => terms::end_value(v.to),
                    _ => match (v.from, v.to) {
                        (Some(a), Some(c)) => Value::Int(c - a),
                        _ => Value::Iri(tm_core::vocab::DL_INF.into()),
                    },
                }]
            }),
            BKind::Allen(name) => {
                let (Some(x), Some(y)) = (self.interval_of(input(0))?, self.interval_of(input(1))?)
                else {
                    return Ok(());
                };
                if terms::allen(name, x, y) == Some(true) {
                    Some(Vec::new())
                } else {
                    None
                }
            }
            BKind::Contains | BKind::StartsWith => {
                let (x, y) = (self.value(input(0))?, self.value(input(1))?);
                let ok = match (terms::string(&x), terms::string(&y)) {
                    (Some(s), Some(t)) => {
                        if *kind == BKind::Contains {
                            s.contains(t)
                        } else {
                            s.starts_with(t)
                        }
                    }
                    _ => false,
                };
                ok.then(Vec::new)
            }
        };
        let Some(vals) = outputs else {
            return Ok(());
        };
        let outs = &args[kind.inputs()..];
        let mut newly = Vec::new();
        let mut ok = true;
        for (s, v) in outs.iter().zip(&vals) {
            let id = self.intern(v)?;
            match Self::bind(s, id, b, &self.sh.consts) {
                Some(n) => newly.push(n),
                None => {
                    newly.push(false);
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            self.step(k + 1, b, iv, prem, deltas, out)?;
        }
        Self::unbind(&outs[..newly.len()], &newly, b);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    // @tg: implements:: [[openspec:datalog-temporal#Aggregates over time]] {since: "2026-10-28"}
    fn aggregate(
        &self,
        agg: &Agg,
        k: usize,
        b: &mut Vec<Id>,
        iv: &IvSet,
        prem: &mut Vec<Prem>,
        deltas: &[(u32, IvSet)],
        out: &mut Vec<Out>,
    ) -> Result<()> {
        // the aggregate body as its own rule over the outer bindings
        let inner_rule = Rule {
            head: 0,
            args: Vec::new(),
            head_op: None,
            exists: Vec::new(),
            body: agg.body.clone(),
            vars: self.rule.vars.clone(),
            kind: RuleKind::Rule(0),
        };
        let bound: BTreeSet<Var> = (0..b.len() as Var)
            .filter(|v| b[*v as usize] != UNBOUND)
            .collect();
        let order = plan_static(&inner_rule, &bound);
        let sub = Cx {
            sh: self.sh,
            store: self.store,
            rule: &inner_rule,
            ri: u32::MAX,
            order: &order,
            delta: None,
        };
        let mut items: FxHashMap<Vec<Id>, (Option<Value>, IvSet)> = FxHashMap::default();
        let mut keys: Vec<Vec<Id>> = Vec::new();
        let mut rows = Vec::new();
        sub.step(
            0,
            &mut b.clone(),
            &IvSet::all(),
            &mut Vec::new(),
            &[],
            &mut rows,
        )?;
        for o in rows {
            let bb: &[Id] = &o.tuple;
            let over = match &agg.over {
                Some(e) => match sub.eval(e, bb)? {
                    Some(v) => Some(sub.as_value(v)?),
                    None => continue,
                },
                None => None,
            };
            let key: Vec<Id> = agg.inner.iter().map(|v| bb[*v as usize]).collect();
            match items.get_mut(&key) {
                Some(e) => e.1 = e.1.union(&o.iv),
                None => {
                    keys.push(key.clone());
                    items.insert(key, (over, o.iv));
                }
            }
        }
        // sweep the endpoints of the contributing sets inside the outer set
        let mut points: Vec<i64> = vec![NEG_INF];
        for key in &keys {
            for &(a, c) in items[key].1.intervals() {
                points.push(a);
                points.push(c);
            }
        }
        for &(a, c) in iv.intervals() {
            points.push(a);
            points.push(c);
        }
        points.sort_unstable();
        points.dedup();
        let mut pieces: Vec<(Value, (i64, i64))> = Vec::new();
        for w in points.windows(2) {
            let (lo, hi) = (w[0], w[1]);
            if !iv.contains(lo) {
                continue;
            }
            let live: Vec<&(Option<Value>, IvSet)> = keys
                .iter()
                .map(|key| &items[key])
                .filter(|(_, s)| s.contains(lo))
                .collect();
            let v = match agg.func {
                AggFn::Count => Some(Value::Int(live.len() as i64)),
                AggFn::Sum => {
                    let mut acc = Value::Int(0);
                    let mut ok = true;
                    for (x, _) in &live {
                        match x
                            .as_ref()
                            .and_then(|x| terms::arith(&acc, crate::syntax::BinOp::Add, x))
                        {
                            Some(s) => acc = s,
                            None => ok = false,
                        }
                    }
                    ok.then_some(acc)
                }
                AggFn::Min | AggFn::Max => {
                    let mut best: Option<Value> = None;
                    for (x, _) in &live {
                        let Some(x) = x else { continue };
                        best = match best {
                            None => Some(x.clone()),
                            Some(cur) => {
                                let better = match terms::compare(x, &cur) {
                                    Some(o) => {
                                        if agg.func == AggFn::Min {
                                            o.is_lt()
                                        } else {
                                            o.is_gt()
                                        }
                                    }
                                    None => false,
                                };
                                Some(if better { x.clone() } else { cur })
                            }
                        };
                    }
                    best
                }
            };
            if let Some(v) = v {
                match pieces.last_mut() {
                    Some((pv, (_, phi))) if *pv == v && *phi == lo => *phi = hi,
                    _ => pieces.push((v, (lo, hi))),
                }
            }
        }
        let mut by_value: Vec<(Value, Vec<(i64, i64)>)> = Vec::new();
        for (v, ivl) in pieces {
            match by_value.iter_mut().find(|(x, _)| *x == v) {
                Some((_, l)) => l.push(ivl),
                None => by_value.push((v, vec![ivl])),
            }
        }
        for (v, ivls) in by_value {
            let id = self.intern(&v)?;
            let piv = IvSet::from_intervals(ivls);
            let iv2 = iv.intersect(&piv);
            if iv2.is_empty() {
                continue;
            }
            let cur = b[agg.out as usize];
            if cur != UNBOUND && cur != id {
                continue;
            }
            b[agg.out as usize] = id;
            self.step(k + 1, b, &iv2, prem, deltas, out)?;
            if cur == UNBOUND {
                b[agg.out as usize] = UNBOUND;
            }
        }
        let _ = POS_INF;
        Ok(())
    }

    // @tg: implements:: [[openspec:datalog-evaluation#Provenance on request]] {since: "2026-10-28"}
    fn emit(&self, b: &mut [Id], iv: &IvSet, prem: &[Prem], out: &mut Vec<Out>) -> Result<()> {
        budget::poll()?;
        if self.ri == u32::MAX {
            // inside an aggregate: the whole binding
            out.push(Out {
                rel: 0,
                tuple: b.iter().copied().collect(),
                iv: iv.clone(),
                deriv: None,
            });
            return Ok(());
        }
        let rule = self.rule;
        let mut tuple: SmallVec<[Id; 4]> = SmallVec::new();
        let frontier: SmallVec<[Id; 4]> = rule
            .args
            .iter()
            .filter(|s| !matches!(s, Slot::Var(v) if rule.exists.contains(v)))
            .map(|s| self.val(s, b))
            .collect();
        for s in &rule.args {
            match s {
                Slot::Var(v) if rule.exists.contains(v) => {
                    let pos = rule.exists.iter().position(|x| x == v).unwrap_or(0) as u32;
                    let mut key: Vec<Id> = frontier.to_vec();
                    key.push(pos as Id);
                    tuple.push(self.sh.terms.invent(self.ri, &key));
                }
                s => tuple.push(self.val(s, b)),
            }
        }
        let mut hiv = match rule.head_op {
            Some((op, w)) => iv.head(op, w),
            None => iv.clone(),
        };
        if self.sh.opts.atemporal {
            hiv = IvSet::all();
        }
        if hiv.is_empty() {
            return Ok(());
        }
        out.push(Out {
            rel: rule.head,
            tuple,
            iv: hiv,
            deriv: self.sh.opts.proof.then(|| Deriv {
                rule: self.ri,
                premises: prem.to_vec(),
            }),
        });
        Ok(())
    }
}

/// A body order without estimates (aggregate bodies): filters as soon as
/// ready, atoms in written order.
fn plan_static(rule: &Rule, start: &BTreeSet<Var>) -> Vec<usize> {
    let mut bound = start.clone();
    let mut left: Vec<usize> = (0..rule.body.len()).collect();
    let mut order = Vec::new();
    while !left.is_empty() {
        let pick = left
            .iter()
            .position(|&k| {
                !matches!(rule.body[k], Elem::Pos(_) | Elem::Binary { .. })
                    && ready(&rule.body[k], &bound, rule).is_some()
            })
            .or_else(|| {
                left.iter()
                    .position(|&k| ready(&rule.body[k], &bound, rule).is_some())
            });
        let Some(i) = pick else {
            break;
        };
        let k = left.remove(i);
        bound.extend(ready(&rule.body[k], &bound, rule).unwrap_or_default());
        order.push(k);
    }
    order
}
