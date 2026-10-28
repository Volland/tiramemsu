//! Constraint checks seeded by a transaction's changes (OpenSpec change
//! `add-datalog-constraints`, `lat.md/datalog#Warn And Error Constraints`).
//!
//! A constraint body is first unfolded: every positive atom over a derived,
//! non-recursive relation is replaced by the bodies of its rules (one variant
//! per rule). Then each positive stored atom gets a variant in which it reads
//! only the statements the transaction added, and each negated stored atom a
//! variant that starts from the statements the transaction retracted. Every
//! other atom is probed in the post-state. A violation found this way is kept
//! only when the pre-state (the view as of the previous transaction) did not
//! already hold it over the same instants, so the cost follows the change.

use std::collections::BTreeSet;

use rustc_hash::FxHashSet;
use tm_core::store::{ReadStore, StmtRow};
use tm_core::{budget, Eid, Error, ObjectId, Result, Value, ViewSpec};
use tm_store::read::View;

use crate::eval::{Deriv, Engine, EvalOptions, Limits, Prem};
use crate::interval::IvSet;
use crate::program::{
    self, Agg, CAtom, CExpr, Elem, Program, Relation, Rule, RuleKind, Slot, Source, StoredPat, Var,
};
use crate::source::{Stored, TimeView};
use crate::syntax::{CmpOp, Pred, Severity};
use crate::terms::{Id, Terms};

/// One injected tuple: values, interval set, the statement it stands for.
type SeedRow = (Vec<Id>, IvSet, Option<Id>);
/// An injected relation: its number, whether it is a demand relation, its tuples.
type SeedSet = (u32, bool, Vec<SeedRow>);
/// A violation row: values, interval set, the statements its derivation rests on.
type Violated = (Vec<Id>, IvSet, BTreeSet<Eid>);

/// The statements a transaction changed.
#[derive(Clone, Debug, Default)]
pub struct Seeds {
    /// Added statements.
    pub added: Vec<StmtRow>,
    /// Retracted statements.
    pub retracted: Vec<StmtRow>,
}

/// One violation found at commit.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    /// The constraint.
    pub constraint: String,
    /// Its severity.
    pub severity: Severity,
    /// The named variables and their values.
    pub bindings: Vec<(String, Value)>,
    /// The statements its derivation used.
    pub eids: Vec<Eid>,
    /// When it holds.
    pub valid: Vec<tm_core::Valid>,
}

/// What a seeded check did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Seeded {
    /// The violations the change introduced.
    pub found: Vec<Found>,
    /// The constraints evaluated.
    pub checked: u64,
    /// The seed rows read.
    pub seed_rows: u64,
}

fn map_slot(s: Slot, f: &dyn Fn(Var) -> Slot) -> Slot {
    match s {
        Slot::Var(v) => f(v),
        c => c,
    }
}

fn map_var(v: Var, f: &dyn Fn(Var) -> Slot) -> Var {
    match f(v) {
        Slot::Var(w) => w,
        Slot::Const(_) => v,
    }
}

fn map_expr(e: &CExpr, f: &dyn Fn(Var) -> Slot) -> CExpr {
    match e {
        CExpr::Slot(s) => CExpr::Slot(map_slot(*s, f)),
        CExpr::Bin(a, op, b) => CExpr::Bin(Box::new(map_expr(a, f)), *op, Box::new(map_expr(b, f))),
        CExpr::Call(n, args) => {
            CExpr::Call(n.clone(), args.iter().map(|a| map_expr(a, f)).collect())
        }
    }
}

fn map_atom(a: &CAtom, f: &dyn Fn(Var) -> Slot) -> CAtom {
    CAtom {
        src: match &a.src {
            Source::Stored(p) => Source::Stored(StoredPat {
                s: map_slot(p.s, f),
                p: map_slot(p.p, f),
                o: map_slot(p.o, f),
                e: p.e.map(|e| map_slot(e, f)),
                record: p.record,
            }),
            Source::Derived { rel, args } => Source::Derived {
                rel: *rel,
                args: args.iter().map(|s| map_slot(*s, f)).collect(),
            },
        },
        ops: a.ops.clone(),
    }
}

/// `e` with its variables renamed (or replaced by constants) by `f`.
pub fn map_elem(e: &Elem, f: &dyn Fn(Var) -> Slot) -> Elem {
    match e {
        Elem::Pos(a) => Elem::Pos(map_atom(a, f)),
        Elem::Neg(a) => Elem::Neg(map_atom(a, f)),
        Elem::Cmp(l, op, r) => Elem::Cmp(map_expr(l, f), *op, map_expr(r, f)),
        Elem::Builtin(k, args) => {
            Elem::Builtin(k.clone(), args.iter().map(|s| map_slot(*s, f)).collect())
        }
        Elem::Agg(a) => Elem::Agg(Box::new(Agg {
            out: map_var(a.out, f),
            func: a.func,
            over: a.over.as_ref().map(|o| map_expr(o, f)),
            body: a.body.iter().map(|x| map_elem(x, f)).collect(),
            group: a.group.iter().map(|v| map_var(*v, f)).collect(),
            inner: a.inner.iter().map(|v| map_var(*v, f)).collect(),
        })),
        Elem::Binary {
            lhs,
            until,
            window,
            rhs,
        } => Elem::Binary {
            lhs: map_atom(lhs, f),
            until: *until,
            window: *window,
            rhs: map_atom(rhs, f),
        },
    }
}

fn rel_name(p: &Program, rel: u32) -> String {
    match &p.rels[rel as usize].name {
        Pred::Iri(i) => i.clone(),
        Pred::Name(n) => n.clone(),
    }
}

fn recursive(p: &Program, rel: u32) -> bool {
    p.strata
        .iter()
        .find(|s| s.rels.contains(&rel))
        .is_some_and(|s| s.recursive)
}

/// The variants of `r` with every derived atom unfolded, or why it cannot be.
pub fn unfold(p: &Program, r: &Rule, depth: usize) -> std::result::Result<Vec<Rule>, String> {
    if depth > 16 {
        return Err("derived relations nest too deeply".into());
    }
    for (k, e) in r.body.iter().enumerate() {
        match e {
            Elem::Agg(_) => return Err("it uses an aggregate".into()),
            Elem::Binary { .. } => return Err("it uses since or until".into()),
            Elem::Pos(a) | Elem::Neg(a) if a.ops.iter().any(|(_, w)| w.hi.is_none()) => {
                return Err("a temporal operator has an infinite window".into())
            }
            Elem::Neg(a) if a.rel().is_some() => {
                return Err(format!(
                    "it negates the derived relation {}",
                    rel_name(p, a.rel().expect("derived"))
                ))
            }
            Elem::Pos(CAtom {
                src: Source::Derived { rel, args },
                ops,
            }) => {
                if !ops.is_empty() {
                    return Err(format!(
                        "a temporal operator applies to the derived relation {}",
                        rel_name(p, *rel)
                    ));
                }
                if recursive(p, *rel) {
                    return Err(format!(
                        "it reads the recursive relation {}",
                        rel_name(p, *rel)
                    ));
                }
                let mut out = Vec::new();
                for dr in p.rules.iter().filter(|x| x.head == *rel) {
                    if !dr.exists.is_empty() || dr.head_op.is_some() {
                        return Err(format!(
                            "a rule of {} invents values or has a head operator",
                            rel_name(p, *rel)
                        ));
                    }
                    let base = r.vars.len();
                    let mut assigned: Vec<Option<Slot>> = vec![None; dr.vars.len()];
                    let mut eqs = Vec::new();
                    let mut matches = true;
                    for (i, h) in dr.args.iter().enumerate() {
                        let a = args[i];
                        match h {
                            Slot::Var(hv) => match assigned[*hv as usize] {
                                None => assigned[*hv as usize] = Some(a),
                                Some(prev) => eqs.push(Elem::Cmp(
                                    CExpr::Slot(prev),
                                    CmpOp::Eq,
                                    CExpr::Slot(a),
                                )),
                            },
                            Slot::Const(c) => match a {
                                Slot::Const(d) if d != *c => matches = false,
                                Slot::Const(_) => {}
                                Slot::Var(_) => {
                                    eqs.push(Elem::Cmp(CExpr::Slot(a), CmpOp::Eq, CExpr::Slot(*h)))
                                }
                            },
                        }
                    }
                    if !matches {
                        continue;
                    }
                    let subst = |v: Var| {
                        assigned[v as usize].unwrap_or(Slot::Var((base + v as usize) as Var))
                    };
                    let mut nr = r.clone();
                    nr.vars.extend(dr.vars.iter().map(|n| {
                        if n.starts_with('_') {
                            format!("_u{n}")
                        } else {
                            format!("$u{n}")
                        }
                    }));
                    let mut body: Vec<Elem> = r.body[..k].to_vec();
                    body.extend(dr.body.iter().map(|x| map_elem(x, &subst)));
                    body.extend(eqs);
                    body.extend(r.body[k + 1..].iter().cloned());
                    nr.body = body;
                    out.extend(unfold(p, &nr, depth + 1)?);
                }
                return Ok(out);
            }
            _ => {}
        }
    }
    Ok(vec![r.clone()])
}

/// Each constraint of `text` with its severity and, when the commit-time check
/// cannot evaluate it from a change, why.
// @tg: implements:: [[openspec:datalog-constraints#Enforceable constraints]] {since: "2026-10-28"}
pub fn enforceability(text: &str) -> Result<Vec<(String, Severity, Option<String>)>> {
    let p = program::compile(text)?;
    Ok(p.constraints
        .iter()
        .map(|c| {
            let why = unfold(&p, &p.rules[c.rule], 0).err();
            (c.name.clone(), c.severity, why)
        })
        .collect())
}

/// Runs `rules` (heads: relation 0) with `seeds` injected, on `spec`; returns
/// the constraint relation's rows as `(tuple, set, eids)`.
#[allow(clippy::too_many_arguments)]
fn run_variants(
    store: &dyn ReadStore,
    spec: &ViewSpec,
    clock: i64,
    base: &Program,
    arity: usize,
    rules: Vec<Rule>,
    seeds: &[SeedSet],
    terms: &Terms,
    extra_consts: Vec<Value>,
) -> Result<Vec<Violated>> {
    let mut rels = vec![Relation {
        name: Pred::Name("$violation".into()),
        arity,
        hidden: true,
        magic: false,
    }];
    for (i, (_, magic, _)) in seeds.iter().enumerate() {
        rels.push(Relation {
            name: Pred::Name(format!("$seed{i}")),
            arity: 4,
            hidden: true,
            magic: *magic,
        });
    }
    let mut consts = base.consts.clone();
    consts.extend(extra_consts);
    let mut q = Program {
        rels,
        rules,
        consts,
        strata: Vec::new(),
        goals: Vec::new(),
        constraints: Vec::new(),
        text_rules: base.text_rules,
        predicates: base.predicates.clone(),
        variable_predicate: base.variable_predicate,
    };
    program::stratify(&mut q)?;
    let view = View::resolve(store, spec)?;
    let mut stored = Stored::new(view, None, FxHashSet::default(), TimeView::Valid);
    // a few seed rows drive the check: probe, never load
    stored.allow_load = false;
    let mut eng = Engine::new(
        &q,
        store,
        stored,
        terms,
        EvalOptions {
            atemporal: false,
            window: None,
            limits: Limits::default(),
            proof: true,
            parallel_threshold: 0,
            now: clock,
            cheap_estimates: true,
        },
    )?;
    for (rel, _, rows) in seeds {
        for (tuple, iv, eid) in rows {
            eng.inject(*rel, tuple, iv, *eid);
        }
    }
    eng.run()?;
    // the statements each row's derivation rests on
    let r = &eng.sh.rels[0];
    let mut out = Vec::new();
    for row in 0..r.len() as u32 {
        let mut eids = BTreeSet::new();
        let mut todo: Vec<u32> = r.prov[row as usize].iter().map(|(_, d)| *d).collect();
        let mut seen = FxHashSet::default();
        while let Some(d) = todo.pop() {
            if !seen.insert(d) {
                continue;
            }
            let Deriv { premises, .. } = &eng.derivs[d as usize];
            for p in premises {
                match *p {
                    Prem::Stmt(e) => {
                        if let Some(e) = Eid::from_oid(ObjectId::from_raw(e)) {
                            eids.insert(e);
                        }
                    }
                    Prem::Row(rel, rr) => {
                        todo.extend(
                            eng.sh.rels[rel as usize].prov[rr as usize]
                                .iter()
                                .map(|(_, d)| *d),
                        );
                    }
                }
            }
        }
        out.push((r.tuple(row).to_vec(), r.ivs[row as usize].clone(), eids));
    }
    Ok(out)
}

/// Checks the constraints of `text` (both severities) for the violations the
/// change `seeds` introduces: evaluated on `post` and filtered against `pre`.
// @tg: implements:: [[openspec:datalog-constraints#Checks read only the change]] {since: "2026-10-28"}
// @tg: implements:: [[openspec:datalog-constraints#Error constraints are checked at commit]] {since: "2026-10-28"}
#[allow(clippy::too_many_arguments)]
pub fn check_seeded(
    store: &dyn ReadStore,
    post: &ViewSpec,
    pre: &ViewSpec,
    clock: i64,
    program_name: &str,
    text: &str,
    seeds: &Seeds,
    max_seed_rows: u64,
) -> Result<Seeded> {
    let p = program::compile(text)?;
    let terms = Terms::new();
    let mut res = Seeded::default();
    let iv_of = |r: &StmtRow| {
        IvSet::of_valid(tm_core::Valid {
            from: r.v_from,
            to: r.v_to,
        })
    };
    for c in &p.constraints {
        let rule = &p.rules[c.rule];
        let variants = match unfold(&p, rule, 0) {
            Ok(v) => v,
            Err(reason) if c.severity == Severity::Error => {
                return Err(Error::ConstraintNotEnforceable {
                    program: program_name.into(),
                    constraint: c.name.clone(),
                    reason,
                })
            }
            Err(_) => continue,
        };
        let mut rules = Vec::new();
        let mut seed_sets: Vec<SeedSet> = Vec::new();
        for d in &variants {
            for (k, e) in d.body.iter().enumerate() {
                let (a, neg) = match e {
                    Elem::Pos(a) => (a, false),
                    Elem::Neg(a) => (a, true),
                    _ => continue,
                };
                let Source::Stored(sp) = &a.src else { continue };
                let pred: Option<Id> = match sp.p {
                    Slot::Const(ci) => Some(terms.intern(store, &p.consts[ci as usize])?),
                    Slot::Var(_) => None,
                };
                let pool = if neg { &seeds.retracted } else { &seeds.added };
                // a row whose constants differ from the atom's can never bind it
                let fixed = |s: Slot| -> Result<Option<Id>> {
                    match s {
                        Slot::Const(ci) => Ok(Some(terms.intern(store, &p.consts[ci as usize])?)),
                        Slot::Var(_) => Ok(None),
                    }
                };
                let (fs, fo) = (fixed(sp.s)?, fixed(sp.o)?);
                let rows: Vec<&StmtRow> = pool
                    .iter()
                    .filter(|r| {
                        pred.is_none_or(|x| x == r.p)
                            && fs.is_none_or(|x| x == r.s)
                            && fo.is_none_or(|x| x == r.o)
                    })
                    .collect();
                if rows.is_empty() {
                    continue;
                }
                let seed_rel = (seed_sets.len() + 1) as u32;
                let mut nr = d.clone();
                nr.head = 0;
                nr.kind = RuleKind::Constraint(0);
                let mut fresh = |nr: &mut Rule| {
                    let v = nr.vars.len() as Var;
                    nr.vars.push(format!("$s{v}"));
                    Slot::Var(v)
                };
                if neg {
                    // bind the negated atom's named variables from the retracted rows
                    let keep =
                        |s: Slot, nr: &mut Rule, fresh: &mut dyn FnMut(&mut Rule) -> Slot| match s {
                            Slot::Var(v) if d.anon(v) => fresh(nr),
                            other => other,
                        };
                    let args = vec![
                        keep(sp.s, &mut nr, &mut fresh),
                        keep(sp.p, &mut nr, &mut fresh),
                        keep(sp.o, &mut nr, &mut fresh),
                        fresh(&mut nr),
                    ];
                    nr.body.insert(
                        0,
                        Elem::Pos(CAtom {
                            src: Source::Derived {
                                rel: seed_rel,
                                args,
                            },
                            ops: Vec::new(),
                        }),
                    );
                    seed_sets.push((
                        seed_rel,
                        true,
                        rows.iter()
                            .map(|r| (vec![r.s, r.p, r.o, r.eid], IvSet::all(), None))
                            .collect(),
                    ));
                } else {
                    let ev = match sp.e {
                        Some(e) => e,
                        None => fresh(&mut nr),
                    };
                    nr.body[k] = Elem::Pos(CAtom {
                        src: Source::Derived {
                            rel: seed_rel,
                            args: vec![sp.s, sp.p, sp.o, ev],
                        },
                        ops: a.ops.clone(),
                    });
                    seed_sets.push((
                        seed_rel,
                        false,
                        rows.iter()
                            .map(|r| {
                                let iv = if sp.record { IvSet::all() } else { iv_of(r) };
                                (vec![r.s, r.p, r.o, r.eid], iv, Some(r.eid))
                            })
                            .collect(),
                    ));
                }
                rules.push(nr);
            }
        }
        if rules.is_empty() {
            continue;
        }
        res.checked += 1;
        res.seed_rows += seed_sets.iter().map(|s| s.2.len() as u64).sum::<u64>();
        if res.seed_rows > max_seed_rows {
            return Err(Error::ConstraintBudgetExceeded {
                program: program_name.into(),
                constraint: c.name.clone(),
            });
        }
        budget::check()?;
        let found = run_variants(
            store,
            post,
            clock,
            &p,
            rule.args.len(),
            rules,
            &seed_sets,
            &terms,
            Vec::new(),
        )?;
        for (tuple, iv, eids) in found {
            // was it there before, over the same instants?
            let mut extra = Vec::new();
            let mut values = Vec::new();
            for &x in &tuple {
                let v = terms.value(store, x)?;
                values.push(v.clone());
                extra.push(v);
            }
            let first = p.consts.len() as u32;
            let mut pre_rules = Vec::new();
            for d in &variants {
                let mut nr = d.clone();
                nr.head = 0;
                nr.kind = RuleKind::Constraint(0);
                let mut binds: Vec<Elem> = rule
                    .args
                    .iter()
                    .enumerate()
                    .map(|(i, s)| {
                        Elem::Cmp(
                            CExpr::Slot(*s),
                            CmpOp::Eq,
                            CExpr::Slot(Slot::Const(first + i as u32)),
                        )
                    })
                    .collect();
                binds.append(&mut nr.body);
                nr.body = binds;
                pre_rules.push(nr);
            }
            let before = run_variants(
                store,
                pre,
                clock,
                &p,
                rule.args.len(),
                pre_rules,
                &[],
                &terms,
                extra,
            )?;
            let mut held = IvSet::empty();
            for (_, biv, _) in before {
                held = held.union(&biv);
            }
            if held.covers(&iv) {
                continue;
            }
            res.found.push(Found {
                constraint: c.name.clone(),
                severity: c.severity,
                bindings: c.columns.iter().cloned().zip(values).collect(),
                eids: eids.into_iter().collect(),
                valid: iv.valids(),
            });
        }
    }
    Ok(res)
}
