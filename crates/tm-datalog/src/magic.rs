//! Demand (magic-set) rewriting of a goal with constants
//! (`lat.md/datalog#Magic Sets For Goals`).
//!
//! Bindings pass left to right through rule bodies. A derived relation is
//! adorned only when every rule defining it is plain (positive atoms without
//! temporal operators, comparisons and builtins, no `exists`, no head
//! operator); any other relation is evaluated in full and the rewrite notes
//! why. The rewritten program computes the same goal rows.

use std::collections::{BTreeSet, VecDeque};

use rustc_hash::FxHashMap;

use crate::program::{ready, stratify, CAtom, Elem, Program, Relation, Rule, RuleKind, Slot, Source, Var};
use crate::syntax::Pred;

fn plain(r: &Rule) -> Result<(), &'static str> {
    if !r.exists.is_empty() {
        return Err("invents values");
    }
    if r.head_op.is_some() {
        return Err("has a head operator");
    }
    for e in &r.body {
        match e {
            Elem::Pos(a) if !a.ops.is_empty() => return Err("uses a temporal operator"),
            Elem::Pos(_) | Elem::Cmp(..) | Elem::Builtin(..) => {}
            Elem::Neg(_) => return Err("uses negation"),
            Elem::Agg(_) => return Err("uses an aggregate"),
            Elem::Binary { .. } => return Err("uses since or until"),
        }
    }
    Ok(())
}

/// The rewrite of goal `goal`, with notes; `None` when no binding passes.
pub fn rewrite(p: &Program, goal: usize) -> Option<(Program, Vec<String>)> {
    let gr = p
        .rules
        .iter()
        .position(|r| r.kind == RuleKind::Goal(goal))?;
    let mut why: FxHashMap<u32, &'static str> = FxHashMap::default();
    for r in &p.rules {
        if let Err(e) = plain(r) {
            why.entry(r.head).or_insert(e);
        }
    }
    let eligible = |rel: u32| !p.rels[rel as usize].hidden && !why.contains_key(&rel);

    let mut rels: Vec<Relation> = p.rels.clone();
    let adorned: std::cell::RefCell<FxHashMap<(u32, u32), (u32, u32)>> = Default::default();
    let mut queue: VecDeque<(u32, u32)> = VecDeque::new();
    let mut rules: Vec<Rule> = Vec::new();
    let mut notes: BTreeSet<String> = BTreeSet::new();

    let adorn = |rel: u32, mask: u32, rels: &mut Vec<Relation>, queue: &mut VecDeque<(u32, u32)>| -> (u32, u32) {
        if let Some(&x) = adorned.borrow().get(&(rel, mask)) {
            return x;
        }
        let base = rels[rel as usize].clone();
        let bits: String = (0..base.arity)
            .map(|i| if mask & (1 << i) != 0 { 'b' } else { 'f' })
            .collect();
        let a = rels.len() as u32;
        rels.push(Relation {
            name: Pred::Name(format!("{}^{bits}", base.name)),
            arity: base.arity,
            hidden: true,
            magic: false,
        });
        let m = rels.len() as u32;
        rels.push(Relation {
            name: Pred::Name(format!("magic:{}^{bits}", base.name)),
            arity: mask.count_ones() as usize,
            hidden: true,
            magic: true,
        });
        adorned.borrow_mut().insert((rel, mask), (a, m));
        queue.push_back((rel, mask));
        (a, m)
    };

    // rewrites a body from `bound`, starting with `start`; emits demand rules
    let rewrite_body = |rule: &Rule,
                            start: Vec<Elem>,
                            mut bound: BTreeSet<Var>,
                            rels: &mut Vec<Relation>,
                            queue: &mut VecDeque<(u32, u32)>,
                            out_rules: &mut Vec<Rule>,
                            notes: &mut BTreeSet<String>|
     -> Vec<Elem> {
        let mut body = start;
        for e in &rule.body {
            let mut e2 = e.clone();
            if let Elem::Pos(CAtom {
                src: Source::Derived { rel, args },
                ops,
            }) = e
            {
                if ops.is_empty() {
                    let mask = args.iter().enumerate().fold(0u32, |m, (i, s)| match s {
                        Slot::Const(_) => m | (1 << i),
                        Slot::Var(v) if bound.contains(v) => m | (1 << i),
                        _ => m,
                    });
                    if mask != 0 && eligible(*rel) {
                        let (a, m) = adorn(*rel, mask, rels, queue);
                        // demand: magic(bound args) :- the body so far, minus
                        // filters whose variables it does not bind
                        let mut mb: Vec<Elem> = Vec::new();
                        let mut have: BTreeSet<Var> = BTreeSet::new();
                        let mut pending: Vec<&Elem> = body.iter().collect();
                        loop {
                            let pick = pending.iter().position(|x| {
                                matches!(x, Elem::Pos(_)) || ready(x, &have, rule).is_some()
                            });
                            let Some(i) = pick else { break };
                            let x = pending.remove(i);
                            if let Some(bs) = ready(x, &have, rule) {
                                have.extend(bs);
                            }
                            mb.push(x.clone());
                        }
                        let margs: Vec<Slot> = args
                            .iter()
                            .enumerate()
                            .filter(|(i, _)| mask & (1 << i) != 0)
                            .map(|(_, s)| *s)
                            .collect();
                        out_rules.push(Rule {
                            head: m,
                            args: margs,
                            head_op: None,
                            exists: Vec::new(),
                            body: mb,
                            vars: rule.vars.clone(),
                            kind: RuleKind::Magic(match rule.kind {
                                RuleKind::Rule(i) => Some(i),
                                _ => None,
                            }),
                        });
                        e2 = Elem::Pos(CAtom {
                            src: Source::Derived {
                                rel: a,
                                args: args.clone(),
                            },
                            ops: Vec::new(),
                        });
                    } else if mask != 0 {
                        if let Some(w) = why.get(rel) {
                            notes.insert(format!(
                                "{} is evaluated in full: a rule defining it {w}",
                                rels[*rel as usize].name
                            ));
                        }
                    }
                }
            }
            if let Some(bs) = ready(&e2, &bound, rule) {
                bound.extend(bs);
            } else if let Elem::Pos(_) = &e2 {
                let mut vs = BTreeSet::new();
                crate::program::elem_vars(&e2, &mut vs);
                bound.extend(vs);
            }
            body.push(e2);
        }
        body
    };

    // the goal rule
    let goal_rule = &p.rules[gr];
    let gbody = rewrite_body(goal_rule, Vec::new(), BTreeSet::new(), &mut rels, &mut queue, &mut rules, &mut notes);
    let mut new_goal = goal_rule.clone();
    new_goal.body = gbody;
    rules.push(new_goal);
    if queue.is_empty() {
        return None;
    }
    // the adorned relations
    while let Some((rel, mask)) = queue.pop_front() {
        let (a, m) = adorned.borrow()[&(rel, mask)];
        for r in p.rules.iter().filter(|r| r.head == rel) {
            let margs: Vec<Slot> = r
                .args
                .iter()
                .enumerate()
                .filter(|(i, _)| mask & (1 << i) != 0)
                .map(|(_, s)| *s)
                .collect();
            let bound: BTreeSet<Var> = margs
                .iter()
                .filter_map(|s| match s {
                    Slot::Var(v) => Some(*v),
                    Slot::Const(_) => None,
                })
                .collect();
            let start = vec![Elem::Pos(CAtom {
                src: Source::Derived { rel: m, args: margs },
                ops: Vec::new(),
            })];
            let body = rewrite_body(r, start, bound, &mut rels, &mut queue, &mut rules, &mut notes);
            let mut nr = r.clone();
            nr.head = a;
            nr.body = body;
            rules.push(nr);
        }
    }
    // relations the rewritten rules read unadorned keep their rules
    let mut need: BTreeSet<u32> = BTreeSet::new();
    let mut todo: Vec<u32> = Vec::new();
    fn reads(e: &Elem, out: &mut Vec<u32>) {
        match e {
            Elem::Pos(a) | Elem::Neg(a) => out.extend(a.rel()),
            Elem::Agg(g) => g.body.iter().for_each(|x| reads(x, out)),
            Elem::Binary { lhs, rhs, .. } => {
                out.extend(lhs.rel());
                out.extend(rhs.rel());
            }
            _ => {}
        }
    }
    for r in &rules {
        for e in &r.body {
            reads(e, &mut todo);
        }
    }
    while let Some(rel) = todo.pop() {
        if (rel as usize) < p.rels.len() && need.insert(rel) {
            for r in p.rules.iter().filter(|r| r.head == rel) {
                for e in &r.body {
                    reads(e, &mut todo);
                }
            }
        }
    }
    for r in &p.rules {
        if need.contains(&r.head) && !matches!(r.kind, RuleKind::Goal(_) | RuleKind::Constraint(_)) {
            rules.push(r.clone());
        }
    }
    let mut q = Program {
        rels,
        rules,
        consts: p.consts.clone(),
        strata: Vec::new(),
        goals: p.goals.clone(),
        constraints: Vec::new(),
        text_rules: p.text_rules,
        predicates: p.predicates.clone(),
        variable_predicate: p.variable_predicate,
    };
    // drop relations nothing defines (other goals and constraints)
    stratify(&mut q).ok()?;
    let defined: BTreeSet<u32> = q.rules.iter().map(|r| r.head).collect();
    q.strata.retain(|s| s.rels.iter().any(|r| defined.contains(r)));
    Some((q, notes.into_iter().collect()))
}
