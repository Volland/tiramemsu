//! Compilation of the syntax tree into rules over numbered variables, and the
//! analysis every program passes before it runs: name resolution, safety,
//! stratification, monotonicity, and recursion through temporal operators or
//! value invention (`lat.md/datalog#Integer Timeline`,
//! `#Stratified Negation In The Interval Engine`, `#Guarded Value Invention`).

use std::collections::{BTreeMap, BTreeSet};

use rustc_hash::FxHashMap;
use tm_core::vocab::{DL_INF, SYS_IN_GRAPH};
use tm_core::{Error, Result, Value};

use crate::syntax::{
    self, AggFn, Atom, BinOp, CmpOp, Expr, Item, Literal, Pred, Severity, TAtom, TempOp, Window,
};
use crate::terms;

/// A variable number within one rule.
pub type Var = u16;

/// An argument position: a variable or a constant (an index into
/// [`Program::consts`]).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Slot {
    /// A variable.
    Var(Var),
    /// A constant.
    Const(u32),
}

/// An expression over slots.
#[derive(Clone, Debug, PartialEq)]
pub enum CExpr {
    /// A slot.
    Slot(Slot),
    /// Arithmetic.
    Bin(Box<CExpr>, BinOp, Box<CExpr>),
    /// A function builtin.
    Call(String, Vec<CExpr>),
}

/// A pattern over stored statements; `e` is the eid position (`None`: hidden).
#[derive(Clone, Debug, PartialEq)]
pub struct StoredPat {
    /// Subject.
    pub s: Slot,
    /// Predicate.
    pub p: Slot,
    /// Object.
    pub o: Slot,
    /// Statement eid, when bound by `@ E` or `stmt`.
    pub e: Option<Slot>,
    /// True for `stmt`: the statement's record, which holds at every instant.
    pub record: bool,
}

/// What an atom reads.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    /// Stored statements.
    Stored(StoredPat),
    /// A derived relation.
    Derived {
        /// Relation number.
        rel: u32,
        /// Arguments.
        args: Vec<Slot>,
    },
}

/// An atom under temporal operators (outermost first).
#[derive(Clone, Debug, PartialEq)]
pub struct CAtom {
    /// What it reads.
    pub src: Source,
    /// Operators, outermost first.
    pub ops: Vec<(TempOp, Window)>,
}

impl CAtom {
    /// The derived relation it reads, if any.
    pub fn rel(&self) -> Option<u32> {
        match &self.src {
            Source::Derived { rel, .. } => Some(*rel),
            Source::Stored(_) => None,
        }
    }

    /// Its slots.
    pub fn slots(&self) -> Vec<Slot> {
        match &self.src {
            Source::Stored(p) => {
                let mut v = vec![p.s, p.p, p.o];
                v.extend(p.e);
                v
            }
            Source::Derived { args, .. } => args.clone(),
        }
    }
}

/// Builtin relations with input and output positions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BKind {
    /// `valid(E, From, To)`: inputs E.
    Valid,
    /// `added(E, T)`: inputs E.
    Added,
    /// `tx_instant(T, Ms)`: inputs T.
    TxInstant,
    /// `interval(E, I)`: inputs E.
    Interval,
    /// `start(I, T)`.
    Start,
    /// `end(I, T)`.
    End,
    /// `duration(I, D)`.
    Duration,
    /// An Allen relation or `intersects`: both inputs.
    Allen(String),
    /// `contains(S, Sub)`: both inputs.
    Contains,
    /// `starts_with(S, P)`: both inputs.
    StartsWith,
}

impl BKind {
    /// The number of leading input arguments.
    pub fn inputs(&self) -> usize {
        match self {
            BKind::Allen(_) | BKind::Contains | BKind::StartsWith => 2,
            _ => 1,
        }
    }

    /// The arity.
    pub fn arity(&self) -> usize {
        match self {
            BKind::Valid => 3,
            _ => 2,
        }
    }

    fn of(name: &str) -> Option<BKind> {
        Some(match name {
            "valid" => BKind::Valid,
            "added" => BKind::Added,
            "tx_instant" => BKind::TxInstant,
            "interval" => BKind::Interval,
            "start" => BKind::Start,
            "end" => BKind::End,
            "duration" => BKind::Duration,
            "contains" => BKind::Contains,
            "starts_with" => BKind::StartsWith,
            n if terms::is_allen(n) => BKind::Allen(n.to_string()),
            _ => return None,
        })
    }
}

/// An aggregate.
#[derive(Clone, Debug, PartialEq)]
pub struct Agg {
    /// The result variable.
    pub out: Var,
    /// The function.
    pub func: AggFn,
    /// The aggregated expression.
    pub over: Option<CExpr>,
    /// The body.
    pub body: Vec<Elem>,
    /// Variables of the body that also occur outside it (bound before).
    pub group: Vec<Var>,
    /// Variables only the body binds.
    pub inner: Vec<Var>,
}

/// One body element.
#[derive(Clone, Debug, PartialEq)]
pub enum Elem {
    /// A positive atom.
    Pos(CAtom),
    /// A negated atom; its anonymous variables are wildcards.
    Neg(CAtom),
    /// A comparison; `=` with one unbound variable side assigns it.
    Cmp(CExpr, CmpOp, CExpr),
    /// A builtin relation.
    Builtin(BKind, Vec<Slot>),
    /// An aggregate.
    Agg(Box<Agg>),
    /// `A since[w] B` or `A until[w] B`.
    Binary {
        /// A.
        lhs: CAtom,
        /// True for `until`.
        until: bool,
        /// The window.
        window: Window,
        /// B.
        rhs: CAtom,
    },
}

/// Where a rule came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuleKind {
    /// The `i`-th rule (or fact) of the text.
    Rule(usize),
    /// The implicit rule that holds a derived IRI predicate's stored statements.
    Seed,
    /// The `i`-th goal.
    Goal(usize),
    /// The `i`-th constraint.
    Constraint(usize),
    /// A rule written by demand rewriting for rule `i` (or a goal).
    Magic(Option<usize>),
}

/// A compiled rule.
#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    /// The head relation.
    pub head: u32,
    /// The head arguments.
    pub args: Vec<Slot>,
    /// A head operator.
    pub head_op: Option<(TempOp, Window)>,
    /// Variables introduced by `exists`.
    pub exists: Vec<Var>,
    /// The body.
    pub body: Vec<Elem>,
    /// The variable names (index = number); anonymous ones are `_`.
    pub vars: Vec<String>,
    /// Where it came from.
    pub kind: RuleKind,
}

impl Rule {
    /// True when variable `v` is anonymous.
    pub fn anon(&self, v: Var) -> bool {
        self.vars[v as usize].starts_with('_')
    }
}

/// A relation the program derives.
#[derive(Clone, Debug, PartialEq)]
pub struct Relation {
    /// Its name.
    pub name: Pred,
    /// Its arity.
    pub arity: usize,
    /// True for goal, constraint and demand relations, never reported as facts.
    pub hidden: bool,
    /// True for demand relations: their facts hold always.
    pub magic: bool,
}

/// A stratum: relations evaluated together, in order.
#[derive(Clone, Debug, PartialEq)]
pub struct Stratum {
    /// Its relations.
    pub rels: Vec<u32>,
    /// The rules whose head is one of them.
    pub rules: Vec<usize>,
    /// True when a relation depends on itself.
    pub recursive: bool,
    /// True when no rule uses negation or aggregation.
    pub monotonic: bool,
    /// True when recursion passes through a metric temporal operator.
    pub metric_recursion: bool,
    /// True when recursion passes through value invention.
    pub invention_recursion: bool,
}

/// A goal.
#[derive(Clone, Debug, PartialEq)]
pub struct Goal {
    /// Its relation.
    pub rel: u32,
    /// The named variables, in first-occurrence order.
    pub columns: Vec<String>,
}

/// A constraint.
#[derive(Clone, Debug, PartialEq)]
pub struct Constraint {
    /// Its name.
    pub name: String,
    /// Its severity.
    pub severity: Severity,
    /// Its relation (one row per violation).
    pub rel: u32,
    /// The named variables.
    pub columns: Vec<String>,
    /// The rule.
    pub rule: usize,
}

/// A compiled and analysed program.
#[derive(Clone, Debug, PartialEq)]
pub struct Program {
    /// Derived relations.
    pub rels: Vec<Relation>,
    /// Rules.
    pub rules: Vec<Rule>,
    /// Constants.
    pub consts: Vec<Value>,
    /// Strata in evaluation order.
    pub strata: Vec<Stratum>,
    /// Goals.
    pub goals: Vec<Goal>,
    /// Constraints.
    pub constraints: Vec<Constraint>,
    /// The number of rules (and facts) of the text.
    pub text_rules: usize,
    /// The stored predicate IRIs the bodies read.
    pub predicates: Vec<String>,
    /// True when a body reads a variable predicate.
    pub variable_predicate: bool,
}

/// What `program_info` reports about a stratum.
#[derive(Clone, Debug, PartialEq)]
pub struct StratumInfo {
    /// Its relations, by name.
    pub relations: Vec<String>,
    /// True when recursive.
    pub recursive: bool,
    /// True when monotonic.
    pub monotonic: bool,
}

/// What analysis reports about a program.
#[derive(Clone, Debug, PartialEq)]
pub struct Analysis {
    /// The number of rules and facts.
    pub rules: usize,
    /// The number of goals.
    pub goals: usize,
    /// The constraints with their severity.
    pub constraints: Vec<(String, Severity)>,
    /// The strata of the text's relations, in evaluation order.
    pub strata: Vec<StratumInfo>,
    /// The stored predicates the bodies read.
    pub predicates: Vec<String>,
    /// True when a body reads a variable predicate.
    pub variable_predicate: bool,
    /// True when some relation depends on itself through a metric operator.
    pub metric_recursion: bool,
    /// True when some relation depends on itself through value invention.
    pub invention_recursion: bool,
}

/// Names that are reserved atoms or builtins.
pub fn reserved(name: &str) -> bool {
    matches!(name, "triple" | "stmt" | "in_graph") || BKind::of(name).is_some()
}

// ---------------------------------------------------------------- compile

struct Compiler {
    rels: Vec<Relation>,
    rel_ix: FxHashMap<Pred, u32>,
    consts: Vec<Value>,
    const_ix: FxHashMap<String, u32>,
    rules: Vec<Rule>,
    predicates: BTreeSet<String>,
    variable_predicate: bool,
}

struct RuleCx {
    vars: Vec<String>,
    names: FxHashMap<String, Var>,
    extra: Vec<Elem>,
    index: usize,
}

impl RuleCx {
    fn var(&mut self, name: &str) -> Var {
        if let Some(&v) = self.names.get(name) {
            return v;
        }
        let v = self.vars.len() as Var;
        self.vars.push(name.to_string());
        self.names.insert(name.to_string(), v);
        v
    }

    fn fresh(&mut self, base: &str) -> Var {
        let v = self.vars.len() as Var;
        self.vars.push(format!("{base}{v}"));
        v
    }
}

fn unsafe_err(rule: usize, msg: impl Into<String>) -> Error {
    Error::DatalogUnsafe {
        rule,
        message: msg.into(),
    }
}

impl Compiler {
    fn konst(&mut self, v: Value) -> Slot {
        let key = format!("{v:?}");
        if let Some(&i) = self.const_ix.get(&key) {
            return Slot::Const(i);
        }
        let i = self.consts.len() as u32;
        self.consts.push(v);
        self.const_ix.insert(key, i);
        Slot::Const(i)
    }

    fn rel(&mut self, name: Pred, arity: usize, hidden: bool) -> Result<u32> {
        if let Some(&i) = self.rel_ix.get(&name) {
            let r = &self.rels[i as usize];
            if r.arity != arity {
                return Err(unsafe_err(
                    0,
                    format!("{} is used with {} and {} arguments", r.name, r.arity, arity),
                ));
            }
            return Ok(i);
        }
        let i = self.rels.len() as u32;
        self.rels.push(Relation {
            name: name.clone(),
            arity,
            hidden,
            magic: false,
        });
        self.rel_ix.insert(name, i);
        Ok(i)
    }

    /// A slot for a term; expressions become a fresh variable with an assignment.
    fn slot(&mut self, cx: &mut RuleCx, e: &Expr) -> Result<Slot> {
        Ok(match e {
            Expr::Var(v) => Slot::Var(cx.var(v)),
            Expr::Anon => Slot::Var(cx.fresh("_")),
            Expr::Const(v) => self.konst(v.clone()),
            Expr::Inf => self.konst(Value::Iri(DL_INF.into())),
            Expr::Bin(..) | Expr::Call(..) => {
                let v = cx.fresh("$e");
                let ce = self.expr(cx, e)?;
                cx.extra
                    .push(Elem::Cmp(CExpr::Slot(Slot::Var(v)), CmpOp::Eq, ce));
                Slot::Var(v)
            }
        })
    }

    fn expr(&mut self, cx: &mut RuleCx, e: &Expr) -> Result<CExpr> {
        Ok(match e {
            Expr::Bin(a, op, b) => CExpr::Bin(
                Box::new(self.expr(cx, a)?),
                *op,
                Box::new(self.expr(cx, b)?),
            ),
            Expr::Call(name, args) => {
                if !terms::FUNCTIONS.contains(&name.as_str()) {
                    return Err(unsafe_err(cx.index, format!("unknown function `{name}`")));
                }
                let mut out = Vec::new();
                for a in args {
                    out.push(self.expr(cx, a)?);
                }
                CExpr::Call(name.clone(), out)
            }
            Expr::Anon => return Err(unsafe_err(cx.index, "`_` in an expression")),
            other => CExpr::Slot(self.slot(cx, other)?),
        })
    }

    fn atom(&mut self, cx: &mut RuleCx, a: &Atom, derived: &BTreeSet<Pred>) -> Result<Elem> {
        let mut args = Vec::new();
        for e in &a.args {
            args.push(self.slot(cx, e)?);
        }
        let eid = match &a.eid {
            Some(e) => Some(self.slot(cx, e)?),
            None => None,
        };
        let stored = |p: Slot, s: Slot, o: Slot, e: Option<Slot>| {
            Elem::Pos(CAtom {
                src: Source::Stored(StoredPat {
                    s,
                    p,
                    o,
                    e,
                    record: false,
                }),
                ops: Vec::new(),
            })
        };
        match &a.pred {
            Pred::Iri(iri) if !derived.contains(&a.pred) => {
                if args.len() != 2 {
                    return Err(unsafe_err(
                        cx.index,
                        format!("stored predicate <{iri}> takes 2 arguments"),
                    ));
                }
                self.predicates.insert(iri.clone());
                let p = self.konst(Value::Iri(iri.clone()));
                Ok(stored(p, args[0], args[1], eid))
            }
            Pred::Name(n) if n == "triple" || n == "stmt" => {
                let (e, rest) = if n == "stmt" {
                    if args.len() != 4 {
                        return Err(unsafe_err(cx.index, "stmt takes 4 arguments"));
                    }
                    (Some(args[0]), &args[1..])
                } else {
                    if args.len() != 3 {
                        return Err(unsafe_err(cx.index, "triple takes 3 arguments"));
                    }
                    (eid, &args[..])
                };
                match rest[1] {
                    Slot::Const(c) => match &self.consts[c as usize] {
                        Value::Iri(i) => {
                            self.predicates.insert(i.clone());
                        }
                        _ => self.variable_predicate = true,
                    },
                    Slot::Var(_) => self.variable_predicate = true,
                }
                let mut el = stored(rest[1], rest[0], rest[2], e);
                if let Elem::Pos(CAtom {
                    src: Source::Stored(p),
                    ..
                }) = &mut el
                {
                    p.record = n == "stmt";
                }
                Ok(el)
            }
            Pred::Name(n) if n == "in_graph" => {
                if args.len() != 2 {
                    return Err(unsafe_err(cx.index, "in_graph takes 2 arguments"));
                }
                self.predicates.insert(SYS_IN_GRAPH.into());
                let p = self.konst(Value::Iri(SYS_IN_GRAPH.into()));
                Ok(stored(p, args[0], args[1], None))
            }
            Pred::Name(n) if BKind::of(n).is_some() => {
                let k = BKind::of(n).expect("checked");
                if args.len() != k.arity() || eid.is_some() {
                    return Err(unsafe_err(
                        cx.index,
                        format!("{n} takes {} arguments", k.arity()),
                    ));
                }
                Ok(Elem::Builtin(k, args))
            }
            _ => {
                if eid.is_some() {
                    return Err(unsafe_err(
                        cx.index,
                        format!("`@ E` on the derived relation {}", a.pred),
                    ));
                }
                let rel = self.rel(a.pred.clone(), args.len(), false)?;
                Ok(Elem::Pos(CAtom {
                    src: Source::Derived { rel, args },
                    ops: Vec::new(),
                }))
            }
        }
    }

    fn tatom(&mut self, cx: &mut RuleCx, t: &TAtom, derived: &BTreeSet<Pred>) -> Result<Elem> {
        let e = self.atom(cx, &t.atom, derived)?;
        if t.ops.is_empty() {
            return Ok(e);
        }
        match e {
            Elem::Pos(mut a) => {
                a.ops = t.ops.clone();
                Ok(Elem::Pos(a))
            }
            _ => Err(unsafe_err(
                cx.index,
                "temporal operators apply to atoms, not builtins",
            )),
        }
    }

    fn catom(&mut self, cx: &mut RuleCx, t: &TAtom, derived: &BTreeSet<Pred>) -> Result<CAtom> {
        match self.tatom(cx, t, derived)? {
            Elem::Pos(a) => Ok(a),
            _ => Err(unsafe_err(cx.index, "since and until need atoms")),
        }
    }

    fn body(
        &mut self,
        cx: &mut RuleCx,
        body: &[Literal],
        derived: &BTreeSet<Pred>,
    ) -> Result<Vec<Elem>> {
        let mut out = Vec::new();
        for l in body {
            let e = match l {
                Literal::Pos(t) => self.tatom(cx, t, derived)?,
                Literal::Neg(t) => match self.tatom(cx, t, derived)? {
                    Elem::Pos(a) => Elem::Neg(a),
                    _ => return Err(unsafe_err(cx.index, "`not` applies to atoms")),
                },
                Literal::Cmp(l, op, r, _) => {
                    let l = self.expr(cx, l)?;
                    let r = self.expr(cx, r)?;
                    Elem::Cmp(l, *op, r)
                }
                Literal::Binary {
                    lhs,
                    until,
                    window,
                    rhs,
                } => {
                    let lhs = self.catom(cx, lhs, derived)?;
                    let rhs = self.catom(cx, rhs, derived)?;
                    Elem::Binary {
                        lhs,
                        until: *until,
                        window: *window,
                        rhs,
                    }
                }
                Literal::Agg {
                    result,
                    func,
                    over,
                    body,
                    ..
                } => {
                    let outv = cx.var(result);
                    let saved = std::mem::take(&mut cx.extra);
                    let before: BTreeSet<Var> = (0..cx.vars.len() as Var).collect();
                    let mut inner_body = self.body(cx, body, derived)?;
                    let over = match over {
                        Some(e) => Some(self.expr(cx, e)?),
                        None => None,
                    };
                    inner_body.append(&mut cx.extra);
                    cx.extra = saved;
                    let mut used = BTreeSet::new();
                    for e in &inner_body {
                        elem_vars(e, &mut used);
                    }
                    if let Some(o) = &over {
                        expr_vars(o, &mut used);
                    }
                    if used.contains(&outv) {
                        return Err(unsafe_err(
                            cx.index,
                            format!("aggregate result {result} occurs in its body"),
                        ));
                    }
                    // a variable named before the aggregate is a group variable;
                    // the rest are local to the aggregate (fixed below)
                    let group: Vec<Var> = used.iter().copied().filter(|v| before.contains(v)).collect();
                    let inner: Vec<Var> = used.iter().copied().filter(|v| !before.contains(v)).collect();
                    Elem::Agg(Box::new(Agg {
                        out: outv,
                        func: *func,
                        over,
                        body: inner_body,
                        group,
                        inner,
                    }))
                }
            };
            out.push(e);
            out.append(&mut cx.extra);
        }
        Ok(out)
    }
}

fn slot_var(s: &Slot, out: &mut BTreeSet<Var>) {
    if let Slot::Var(v) = s {
        out.insert(*v);
    }
}

/// The variables of an expression.
pub fn expr_vars(e: &CExpr, out: &mut BTreeSet<Var>) {
    match e {
        CExpr::Slot(s) => slot_var(s, out),
        CExpr::Bin(a, _, b) => {
            expr_vars(a, out);
            expr_vars(b, out);
        }
        CExpr::Call(_, args) => args.iter().for_each(|a| expr_vars(a, out)),
    }
}

/// The variables an element mentions.
pub fn elem_vars(e: &Elem, out: &mut BTreeSet<Var>) {
    match e {
        Elem::Pos(a) | Elem::Neg(a) => a.slots().iter().for_each(|s| slot_var(s, out)),
        Elem::Cmp(l, _, r) => {
            expr_vars(l, out);
            expr_vars(r, out);
        }
        Elem::Builtin(_, args) => args.iter().for_each(|s| slot_var(s, out)),
        Elem::Agg(a) => {
            out.insert(a.out);
            out.extend(a.group.iter().copied());
        }
        Elem::Binary { lhs, rhs, .. } => {
            lhs.slots().iter().for_each(|s| slot_var(s, out));
            rhs.slots().iter().for_each(|s| slot_var(s, out));
        }
    }
}

/// Whether `e` can run with the variables `bound`, and which it then binds.
pub fn ready(e: &Elem, bound: &BTreeSet<Var>, rule: &Rule) -> Option<BTreeSet<Var>> {
    let is_bound = |s: &Slot| match s {
        Slot::Const(_) => true,
        Slot::Var(v) => bound.contains(v),
    };
    let mut binds = BTreeSet::new();
    match e {
        Elem::Pos(a) => {
            a.slots().iter().for_each(|s| slot_var(s, &mut binds));
        }
        Elem::Binary { lhs, rhs, .. } => {
            lhs.slots().iter().for_each(|s| slot_var(s, &mut binds));
            rhs.slots().iter().for_each(|s| slot_var(s, &mut binds));
        }
        Elem::Neg(a) => {
            for s in a.slots() {
                if let Slot::Var(v) = s {
                    if !rule.anon(v) && !bound.contains(&v) {
                        return None;
                    }
                }
            }
        }
        Elem::Cmp(l, op, r) => {
            let (mut lv, mut rv) = (BTreeSet::new(), BTreeSet::new());
            expr_vars(l, &mut lv);
            expr_vars(r, &mut rv);
            let lb = lv.is_subset(bound);
            let rb = rv.is_subset(bound);
            if lb && rb {
            } else if *op == CmpOp::Eq {
                match (l, r) {
                    (CExpr::Slot(Slot::Var(v)), _) if !bound.contains(v) && rb => {
                        binds.insert(*v);
                    }
                    (_, CExpr::Slot(Slot::Var(v))) if !bound.contains(v) && lb => {
                        binds.insert(*v);
                    }
                    _ => return None,
                }
            } else {
                return None;
            }
        }
        Elem::Builtin(k, args) => {
            if !args[..k.inputs()].iter().all(is_bound) {
                return None;
            }
            args[k.inputs()..]
                .iter()
                .for_each(|s| slot_var(s, &mut binds));
        }
        Elem::Agg(a) => {
            if !a.group.iter().all(|v| bound.contains(v)) {
                return None;
            }
            binds.insert(a.out);
        }
    }
    Some(binds)
}

/// Checks that some order evaluates `body` from `bound`; returns what it binds.
fn check_body(body: &[Elem], bound: &BTreeSet<Var>, rule: &Rule, index: usize) -> Result<BTreeSet<Var>> {
    let mut bound = bound.clone();
    let mut left: Vec<&Elem> = body.iter().collect();
    while !left.is_empty() {
        let pick = left.iter().position(|e| ready(e, &bound, rule).is_some());
        match pick {
            Some(i) => {
                let e = left.remove(i);
                if let Elem::Agg(a) = e {
                    let inner = check_body(&a.body, &bound, rule, index)?;
                    if let Some(o) = &a.over {
                        let mut ov = BTreeSet::new();
                        expr_vars(o, &mut ov);
                        if let Some(v) = ov.iter().find(|v| !inner.contains(v)) {
                            return Err(unsafe_err(
                                index,
                                format!("{} in an aggregate is not bound by its body", rule.vars[*v as usize]),
                            ));
                        }
                    }
                }
                bound.extend(ready(e, &bound, rule).unwrap_or_default());
            }
            None => {
                let mut need = BTreeSet::new();
                for e in &left {
                    elem_vars(e, &mut need);
                }
                let v = need
                    .iter()
                    .find(|v| !bound.contains(v) && !rule.anon(**v))
                    .or_else(|| need.iter().find(|v| !bound.contains(v)));
                let name = v.map_or("?".to_string(), |v| rule.vars[*v as usize].clone());
                return Err(unsafe_err(
                    index,
                    format!("variable {name} is not bound by a positive atom"),
                ));
            }
        }
    }
    Ok(bound)
}

/// Compiles and analyses program text.
pub fn compile(text: &str) -> Result<Program> {
    compile_ast(&syntax::parse(text)?)
}

/// Compiles and analyses a syntax tree.
pub fn compile_ast(ast: &syntax::Ast) -> Result<Program> {
    let mut c = Compiler {
        rels: Vec::new(),
        rel_ix: FxHashMap::default(),
        consts: Vec::new(),
        const_ix: FxHashMap::default(),
        rules: Vec::new(),
        predicates: BTreeSet::new(),
        variable_predicate: false,
    };
    // relations defined by heads; IRI heads hold their stored statements too
    let mut derived: BTreeSet<Pred> = BTreeSet::new();
    for item in &ast.items {
        if let Item::Rule { head, .. } = item {
            if let Pred::Name(n) = &head.atom.pred {
                if reserved(n) {
                    return Err(Error::DatalogReserved { name: n.clone() });
                }
            }
            if let Some((op, _)) = head.op {
                if matches!(op, TempOp::Past | TempOp::Future) {
                    return Err(Error::DatalogNotHorn {
                        message: format!("`{}` in the head of {}", op.name(), head.atom.pred),
                    });
                }
            }
            derived.insert(head.atom.pred.clone());
        }
    }
    let mut goals = Vec::new();
    let mut constraints: Vec<Constraint> = Vec::new();
    let mut text_rules = 0;
    let mut seen_constraints = BTreeSet::new();
    for item in &ast.items {
        match item {
            Item::Rule { head, body, .. } => {
                let index = text_rules;
                text_rules += 1;
                let mut cx = RuleCx {
                    vars: Vec::new(),
                    names: FxHashMap::default(),
                    extra: Vec::new(),
                    index,
                };
                let exists: Vec<Var> = head.exists.iter().map(|v| cx.var(v)).collect();
                let mut b = c.body(&mut cx, body, &derived)?;
                let mut args = Vec::new();
                for e in &head.atom.args {
                    if matches!(e, Expr::Anon) {
                        return Err(unsafe_err(index, "`_` in a head"));
                    }
                    args.push(c.slot(&mut cx, e)?);
                }
                b.append(&mut cx.extra);
                if head.atom.eid.is_some() {
                    return Err(unsafe_err(index, "`@ E` in a head"));
                }
                if let Pred::Iri(_) = head.atom.pred {
                    if args.len() != 2 {
                        return Err(unsafe_err(index, "an IRI head takes 2 arguments"));
                    }
                }
                let rel = c.rel(head.atom.pred.clone(), args.len(), false)?;
                c.rules.push(Rule {
                    head: rel,
                    args,
                    head_op: head.op,
                    exists,
                    body: b,
                    vars: cx.vars,
                    kind: RuleKind::Rule(index),
                });
            }
            Item::Goal { body, .. } | Item::Constraint { body, .. } => {
                let mut cx = RuleCx {
                    vars: Vec::new(),
                    names: FxHashMap::default(),
                    extra: Vec::new(),
                    index: text_rules + goals.len() + constraints.len(),
                };
                let b = c.body(&mut cx, body, &derived)?;
                let mut named: Vec<(String, Var)> = cx
                    .vars
                    .iter()
                    .enumerate()
                    .filter(|(_, n)| !n.starts_with('_') && !n.starts_with('$'))
                    .map(|(i, n)| (n.clone(), i as Var))
                    .collect();
                // aggregate-local variables are not columns
                let mut local = BTreeSet::new();
                for e in &b {
                    if let Elem::Agg(a) = e {
                        local.extend(a.inner.iter().copied());
                    }
                }
                named.retain(|(_, v)| !local.contains(v));
                let columns: Vec<String> = named.iter().map(|(n, _)| n.clone()).collect();
                let args: Vec<Slot> = named.iter().map(|(_, v)| Slot::Var(*v)).collect();
                let (name, kind) = match item {
                    Item::Goal { .. } => (
                        Pred::Name(format!("$goal{}", goals.len())),
                        RuleKind::Goal(goals.len()),
                    ),
                    Item::Constraint { name, .. } => {
                        if !seen_constraints.insert(name.clone()) {
                            return Err(Error::DatalogDuplicateConstraint { name: name.clone() });
                        }
                        (
                            Pred::Name(format!("$constraint{}", constraints.len())),
                            RuleKind::Constraint(constraints.len()),
                        )
                    }
                    Item::Rule { .. } => unreachable!(),
                };
                let rel = c.rel(name, args.len(), true)?;
                let rule_ix = c.rules.len();
                c.rules.push(Rule {
                    head: rel,
                    args,
                    head_op: None,
                    exists: Vec::new(),
                    body: b,
                    vars: cx.vars,
                    kind: kind.clone(),
                });
                match item {
                    Item::Goal { .. } => goals.push(Goal { rel, columns }),
                    Item::Constraint { name, severity, .. } => constraints.push(Constraint {
                        name: name.clone(),
                        severity: *severity,
                        rel,
                        columns,
                        rule: rule_ix,
                    }),
                    Item::Rule { .. } => unreachable!(),
                }
            }
        }
    }
    // seed rules: `p(S, O) :- <stored p>(S, O)` for every IRI head
    for (i, r) in c.rels.clone().iter().enumerate() {
        if let Pred::Iri(iri) = &r.name {
            c.predicates.insert(iri.clone());
            let p = c.konst(Value::Iri(iri.clone()));
            c.rules.push(Rule {
                head: i as u32,
                args: vec![Slot::Var(0), Slot::Var(1)],
                head_op: None,
                exists: Vec::new(),
                body: vec![Elem::Pos(CAtom {
                    src: Source::Stored(StoredPat {
                        s: Slot::Var(0),
                        p,
                        o: Slot::Var(1),
                        e: None,
                        record: false,
                    }),
                    ops: Vec::new(),
                })],
                vars: vec!["S".into(), "O".into()],
                kind: RuleKind::Seed,
            });
        }
    }
    // relations used but never defined have no facts: an error, as Soufflé
    let defined: BTreeSet<u32> = c.rules.iter().map(|r| r.head).collect();
    for (i, r) in c.rels.iter().enumerate() {
        if !defined.contains(&(i as u32)) {
            return Err(unsafe_err(0, format!("relation {} is used but never defined", r.name)));
        }
    }
    let mut p = Program {
        rels: c.rels,
        rules: c.rules,
        consts: c.consts,
        strata: Vec::new(),
        goals,
        constraints,
        text_rules,
        predicates: c.predicates.into_iter().collect(),
        variable_predicate: c.variable_predicate,
    };
    // safety
    for (i, r) in p.rules.iter().enumerate() {
        let index = match r.kind {
            RuleKind::Rule(k) => k,
            _ => i,
        };
        let bound = check_body(&r.body, &BTreeSet::new(), r, index)?;
        for (k, s) in r.args.iter().enumerate() {
            if let Slot::Var(v) = s {
                if !bound.contains(v) && !r.exists.contains(v) {
                    return Err(unsafe_err(
                        index,
                        format!("head variable {} (argument {}) is not bound", r.vars[*v as usize], k + 1),
                    ));
                }
            }
        }
    }
    stratify(&mut p)?;
    Ok(p)
}

// ---------------------------------------------------------------- strata

/// The derived relations an element reads, with polarity (false: through
/// negation or aggregation) and whether through a temporal operator.
fn deps(e: &Elem, out: &mut Vec<(u32, bool, bool)>) {
    match e {
        Elem::Pos(a) => {
            if let Some(r) = a.rel() {
                out.push((r, true, !a.ops.is_empty()));
            }
        }
        Elem::Neg(a) => {
            if let Some(r) = a.rel() {
                out.push((r, false, !a.ops.is_empty()));
            }
        }
        Elem::Agg(a) => {
            let mut inner = Vec::new();
            for e in &a.body {
                deps(e, &mut inner);
            }
            out.extend(inner.into_iter().map(|(r, _, t)| (r, false, t)));
        }
        Elem::Binary { lhs, rhs, .. } => {
            for a in [lhs, rhs] {
                if let Some(r) = a.rel() {
                    out.push((r, true, true));
                }
            }
        }
        Elem::Cmp(..) | Elem::Builtin(..) => {}
    }
}

/// Computes the strata of `p` in evaluation order.
pub fn stratify(p: &mut Program) -> Result<()> {
    let n = p.rels.len();
    let mut edges: Vec<Vec<(u32, bool)>> = vec![Vec::new(); n];
    for r in &p.rules {
        let mut d = Vec::new();
        for e in &r.body {
            deps(e, &mut d);
        }
        for (to, pos, _) in d {
            edges[r.head as usize].push((to, pos));
        }
    }
    // Tarjan: components come out dependencies first
    struct T<'a> {
        edges: &'a [Vec<(u32, bool)>],
        index: Vec<Option<usize>>,
        low: Vec<usize>,
        on: Vec<bool>,
        stack: Vec<usize>,
        next: usize,
        comps: Vec<Vec<usize>>,
    }
    fn visit(t: &mut T<'_>, v: usize) {
        t.index[v] = Some(t.next);
        t.low[v] = t.next;
        t.next += 1;
        t.stack.push(v);
        t.on[v] = true;
        for &(w, _) in &t.edges[v] {
            let w = w as usize;
            match t.index[w] {
                None => {
                    visit(t, w);
                    t.low[v] = t.low[v].min(t.low[w]);
                }
                Some(iw) if t.on[w] => t.low[v] = t.low[v].min(iw),
                _ => {}
            }
        }
        if Some(t.low[v]) == t.index[v] {
            let mut comp = Vec::new();
            while let Some(w) = t.stack.pop() {
                t.on[w] = false;
                comp.push(w);
                if w == v {
                    break;
                }
            }
            comp.sort_unstable();
            t.comps.push(comp);
        }
    }
    let mut t = T {
        edges: &edges,
        index: vec![None; n],
        low: vec![0; n],
        on: vec![false; n],
        stack: Vec::new(),
        next: 0,
        comps: Vec::new(),
    };
    for v in 0..n {
        if t.index[v].is_none() {
            visit(&mut t, v);
        }
    }
    let comps = t.comps;
    let mut comp_of = vec![0usize; n];
    for (i, c) in comps.iter().enumerate() {
        for &v in c {
            comp_of[v] = i;
        }
    }
    let mut strata = Vec::new();
    for (ci, comp) in comps.iter().enumerate() {
        let in_comp = |r: u32| comp_of[r as usize] == ci;
        let rules: Vec<usize> = p
            .rules
            .iter()
            .enumerate()
            .filter(|(_, r)| in_comp(r.head))
            .map(|(i, _)| i)
            .collect();
        let mut recursive = comp.len() > 1;
        let mut monotonic = true;
        let mut metric = false;
        let mut invention = false;
        for &ri in &rules {
            let r = &p.rules[ri];
            let mut d = Vec::new();
            for e in &r.body {
                deps(e, &mut d);
                if matches!(e, Elem::Neg(_) | Elem::Agg(_)) {
                    monotonic = false;
                }
            }
            let rec_rule = d.iter().any(|(to, _, _)| in_comp(*to));
            if rec_rule {
                recursive = true;
            }
            for (to, pos, temporal) in &d {
                if in_comp(*to) {
                    if !pos {
                        let names: Vec<String> =
                            comp.iter().map(|&v| p.rels[v].name.to_string()).collect();
                        return Err(Error::DatalogUnstratifiable {
                            message: format!(
                                "{} depend on each other through negation or aggregation",
                                names.join(" and ")
                            ),
                        });
                    }
                    if *temporal {
                        metric = true;
                    }
                }
            }
            if rec_rule && r.head_op.is_some() {
                metric = true;
            }
            if rec_rule {
                let mut assigned = BTreeSet::new();
                for e in &r.body {
                    if let Elem::Cmp(CExpr::Slot(Slot::Var(v)), CmpOp::Eq, rhs) = e {
                        if !matches!(rhs, CExpr::Slot(_)) {
                            assigned.insert(*v);
                        }
                    }
                }
                let computed_head = r.args.iter().any(|s| matches!(s, Slot::Var(v) if assigned.contains(v)));
                if !r.exists.is_empty() || computed_head {
                    invention = true;
                }
            }
        }
        strata.push(Stratum {
            rels: comp.iter().map(|&v| v as u32).collect(),
            rules,
            recursive,
            monotonic,
            metric_recursion: metric,
            invention_recursion: invention,
        });
    }
    p.strata = strata;
    Ok(())
}

impl Program {
    /// What analysis found.
    pub fn analysis(&self) -> Analysis {
        let strata = self
            .strata
            .iter()
            .filter(|s| s.rels.iter().any(|&r| !self.rels[r as usize].hidden))
            .map(|s| StratumInfo {
                relations: s
                    .rels
                    .iter()
                    .map(|&r| match &self.rels[r as usize].name {
                        Pred::Iri(i) => i.clone(),
                        Pred::Name(n) => n.clone(),
                    })
                    .collect(),
                recursive: s.recursive,
                monotonic: s.monotonic,
            })
            .collect();
        Analysis {
            rules: self.text_rules,
            goals: self.goals.len(),
            constraints: self
                .constraints
                .iter()
                .map(|c| (c.name.clone(), c.severity))
                .collect(),
            strata,
            predicates: self.predicates.clone(),
            variable_predicate: self.variable_predicate,
            metric_recursion: self.strata.iter().any(|s| s.metric_recursion),
            invention_recursion: self.strata.iter().any(|s| s.invention_recursion),
        }
    }

    /// True when a rule uses a temporal operator.
    pub fn temporal_operator(&self) -> Option<String> {
        fn in_elem(e: &Elem) -> Option<String> {
            match e {
                Elem::Pos(a) | Elem::Neg(a) => a.ops.first().map(|(o, _)| o.name().to_string()),
                Elem::Binary { until, .. } => Some(if *until { "until" } else { "since" }.into()),
                Elem::Agg(a) => a.body.iter().find_map(in_elem),
                _ => None,
            }
        }
        self.rules.iter().find_map(|r| {
            r.head_op
                .map(|(o, _)| o.name().to_string())
                .or_else(|| r.body.iter().find_map(in_elem))
        })
    }

    /// The relation names in a stable map (for magic rewriting).
    pub fn rel_names(&self) -> BTreeMap<String, u32> {
        self.rels
            .iter()
            .enumerate()
            .map(|(i, r)| (r.name.to_string(), i as u32))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analysis_of_a_recursive_temporal_program() {
        let p = compile(
            "contagious(X) :- v:sick(X, true).\n\
             contagious(Y) :- past[0d, 3d] contagious(X), v:contact(X, Y).\n",
        )
        .unwrap();
        let a = p.analysis();
        assert_eq!(a.strata.len(), 1);
        assert!(a.strata[0].recursive && a.strata[0].monotonic);
        assert_eq!(
            a.predicates,
            vec!["urn:tiramemsu:v:contact".to_string(), "urn:tiramemsu:v:sick".to_string()]
        );
        assert!(a.metric_recursion);
    }

    #[test]
    fn errors() {
        assert!(matches!(compile("p(X, Y) :- v:q(X, Z)."), Err(Error::DatalogUnsafe { .. })));
        assert!(matches!(
            compile("q(X) :- v:a(X, X).\np(X) :- q(X), not r(X).\nr(X) :- q(X), not p(X)."),
            Err(Error::DatalogUnstratifiable { .. })
        ));
        assert!(matches!(compile("valid(X, Y, Z) :- v:a(X, Y), v:a(Y, Z)."), Err(Error::DatalogReserved { .. })));
        assert!(matches!(compile("future[0d, 1d] p(X) :- v:a(X, X)."), Err(Error::DatalogNotHorn { .. })));
        assert!(matches!(
            compile("constraint a warn :- v:a(X, X).\nconstraint a error :- v:a(X, X)."),
            Err(Error::DatalogDuplicateConstraint { .. })
        ));
        let p = compile("n(0).\nn(Y) :- n(X), Y = X + 1.").unwrap();
        assert!(p.analysis().invention_recursion);
    }
}
