//! Thompson NFA over `(atom, direction)` symbols and its determinisation.
//!
//! `^` pushes the direction down (and reverses sequences), bounded repetition is
//! unrolled, and subset construction runs over a refined alphabet of *letters*, so
//! every hop sequence has exactly one run: ambiguous expressions (`p|p`, `(p*)*`)
//! cannot produce duplicate paths. An unbounded Cypher cap is a searcher depth
//! bound, not an unrolling.

use std::collections::{BTreeSet, HashMap};

use tm_core::{Error, Result};
use tm_ir::vocab::{SYS_ANY_RELATIONSHIP, SYS_OBJECT, SYS_PREDICATE, SYS_SUBJECT};
use tm_ir::PathExpr;

use super::ast::nullable;

/// Direction of one hop.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Dir {
    /// Subject to object.
    Out,
    /// Object to subject.
    In,
}

/// What a path step matches, before resolution to ids.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AtomKey {
    /// A stored predicate.
    Iri(String),
    /// `sys:subject` (virtual).
    VSubject,
    /// `sys:object` (virtual).
    VObject,
    /// `sys:predicate` (virtual).
    VPredicate,
    /// `sys:anyRelationship`.
    Any,
}

impl AtomKey {
    /// The atom of a predicate IRI.
    pub fn of_iri(iri: &str) -> AtomKey {
        match iri {
            SYS_SUBJECT => AtomKey::VSubject,
            SYS_OBJECT => AtomKey::VObject,
            SYS_PREDICATE => AtomKey::VPredicate,
            SYS_ANY_RELATIONSHIP => AtomKey::Any,
            other => AtomKey::Iri(other.to_string()),
        }
    }
}

/// A member of the refined alphabet: a class of hops the DFA cannot tell apart.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LetterKind {
    /// Statements of one predicate; `rv` (only when the expression also has a
    /// wildcard) says whether the statement is in the relationship view.
    Pred {
        /// The predicate.
        iri: String,
        /// Relationship-view membership, when it matters.
        rv: Option<bool>,
    },
    /// Relationship-view statements of any other predicate.
    Other,
    /// `sys:subject`.
    VSubject,
    /// `sys:object`.
    VObject,
    /// `sys:predicate`.
    VPredicate,
}

/// A letter: a hop class in one direction.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Letter {
    /// The class.
    pub kind: LetterKind,
    /// The direction.
    pub dir: Dir,
}

fn matches(atom: &AtomKey, k: &LetterKind) -> bool {
    match (atom, k) {
        (AtomKey::Iri(a), LetterKind::Pred { iri, .. }) => a == iri,
        (AtomKey::Any, LetterKind::Pred { rv, .. }) => *rv == Some(true),
        (AtomKey::Any, LetterKind::Other) => true,
        (AtomKey::VSubject, LetterKind::VSubject)
        | (AtomKey::VObject, LetterKind::VObject)
        | (AtomKey::VPredicate, LetterKind::VPredicate) => true,
        _ => false,
    }
}

#[derive(Clone, Debug)]
enum Edge {
    Eps(usize),
    Sym(AtomKey, Dir, usize),
}

/// The NFA of an expression: one start, one accepting state.
#[derive(Clone, Debug)]
pub struct Nfa {
    edges: Vec<Vec<Edge>>,
    start: usize,
    accept: usize,
}

/// NFA size cap (guards adversarial repetition counts).
const MAX_NFA_STATES: usize = 200_000;
/// DFA size cap.
pub const MAX_DFA_STATES: usize = 4_096;

fn too_complex() -> Error {
    Error::unsupported("path expression too complex")
}

struct Builder {
    edges: Vec<Vec<Edge>>,
}

impl Builder {
    fn state(&mut self) -> Result<usize> {
        if self.edges.len() >= MAX_NFA_STATES {
            return Err(too_complex());
        }
        self.edges.push(Vec::new());
        Ok(self.edges.len() - 1)
    }

    fn eps(&mut self, a: usize, b: usize) {
        self.edges[a].push(Edge::Eps(b));
    }

    fn build(&mut self, e: &PathExpr, flip: bool) -> Result<(usize, usize)> {
        match e {
            PathExpr::Pred(v) => {
                let (s, t) = (self.state()?, self.state()?);
                let atom = match v {
                    tm_core::Value::Iri(i) => AtomKey::of_iri(i),
                    other => AtomKey::Iri(other.lexical()),
                };
                let dir = if flip { Dir::In } else { Dir::Out };
                self.edges[s].push(Edge::Sym(atom, dir, t));
                Ok((s, t))
            }
            PathExpr::Inverse(x) => self.build(x, !flip),
            PathExpr::Seq(xs) => {
                let mut order: Vec<&PathExpr> = xs.iter().collect();
                if flip {
                    order.reverse();
                }
                let mut it = order.into_iter();
                let Some(first) = it.next() else {
                    let s = self.state()?;
                    return Ok((s, s));
                };
                let (s, mut t) = self.build(first, flip)?;
                for x in it {
                    let (a, b) = self.build(x, flip)?;
                    self.eps(t, a);
                    t = b;
                }
                Ok((s, t))
            }
            PathExpr::Alt(xs) => {
                let (s, t) = (self.state()?, self.state()?);
                for x in xs {
                    let (a, b) = self.build(x, flip)?;
                    self.eps(s, a);
                    self.eps(b, t);
                }
                Ok((s, t))
            }
            PathExpr::ZeroOrMore(x) => {
                let (s, t) = (self.state()?, self.state()?);
                let (a, b) = self.build(x, flip)?;
                self.eps(s, a);
                self.eps(s, t);
                self.eps(b, a);
                self.eps(b, t);
                Ok((s, t))
            }
            PathExpr::OneOrMore(x) => {
                let (s, t) = (self.state()?, self.state()?);
                let (a, b) = self.build(x, flip)?;
                self.eps(s, a);
                self.eps(b, a);
                self.eps(b, t);
                Ok((s, t))
            }
            PathExpr::ZeroOrOne(x) => {
                let (s, t) = (self.state()?, self.state()?);
                let (a, b) = self.build(x, flip)?;
                self.eps(s, a);
                self.eps(s, t);
                self.eps(b, t);
                Ok((s, t))
            }
            PathExpr::Repeat { inner, min, max } => {
                let s = self.state()?;
                let mut t = s;
                for _ in 0..*min {
                    let (a, b) = self.build(inner, flip)?;
                    self.eps(t, a);
                    t = b;
                }
                match max {
                    None => {
                        let (a, b) = self.build(&PathExpr::ZeroOrMore(inner.clone()), flip)?;
                        self.eps(t, a);
                        t = b;
                    }
                    Some(mx) => {
                        let end = self.state()?;
                        // (max - min) nested optional copies
                        for _ in *min..*mx {
                            self.eps(t, end);
                            let (a, b) = self.build(inner, flip)?;
                            self.eps(t, a);
                            t = b;
                        }
                        self.eps(t, end);
                        t = end;
                    }
                }
                Ok((s, t))
            }
        }
    }
}

impl Nfa {
    /// Compiles `e` (which may contain `Inverse`, `Repeat` and every operator).
    pub fn compile(e: &PathExpr) -> Result<Nfa> {
        let mut b = Builder { edges: Vec::new() };
        let (start, accept) = b.build(e, false)?;
        Ok(Nfa {
            edges: b.edges,
            start,
            accept,
        })
    }

    /// Number of states.
    pub fn len(&self) -> usize {
        self.edges.len()
    }

    /// True when there are no states (never).
    pub fn is_empty(&self) -> bool {
        self.edges.is_empty()
    }

    fn closure(&self, set: &mut BTreeSet<usize>) {
        let mut stack: Vec<usize> = set.iter().copied().collect();
        while let Some(s) = stack.pop() {
            for e in &self.edges[s] {
                if let Edge::Eps(t) = e {
                    if set.insert(*t) {
                        stack.push(*t);
                    }
                }
            }
        }
    }

    fn step(&self, from: &BTreeSet<usize>, l: &Letter) -> BTreeSet<usize> {
        let mut out = BTreeSet::new();
        for s in from {
            for e in &self.edges[*s] {
                if let Edge::Sym(a, d, t) = e {
                    if *d == l.dir && matches(a, &l.kind) {
                        out.insert(*t);
                    }
                }
            }
        }
        self.closure(&mut out);
        out
    }

    /// True when the NFA accepts the word.
    pub fn accepts(&self, word: &[Letter]) -> bool {
        let mut cur: BTreeSet<usize> = [self.start].into();
        self.closure(&mut cur);
        for l in word {
            cur = self.step(&cur, l);
        }
        cur.contains(&self.accept)
    }

    fn symbols(&self) -> Vec<(AtomKey, Dir)> {
        let mut v: Vec<(AtomKey, Dir)> = self
            .edges
            .iter()
            .flatten()
            .filter_map(|e| match e {
                Edge::Sym(a, d, _) => Some((a.clone(), *d)),
                Edge::Eps(_) => None,
            })
            .collect();
        v.sort();
        v.dedup();
        v
    }
}

/// The deterministic automaton over letters.
#[derive(Clone, Debug)]
// @lat: [[query#Physical Planning#Path Engine]]
pub struct Dfa {
    /// The alphabet.
    pub letters: Vec<Letter>,
    /// Per state, its outgoing `(letter index, target)` pairs in letter order.
    pub trans: Vec<Vec<(usize, u32)>>,
    /// Per state, whether it accepts.
    pub accepting: Vec<bool>,
    /// The start state.
    pub start: u32,
    /// Whether the empty word is accepted (`accepting[start]`).
    pub nullable: bool,
}

impl Dfa {
    /// Compiles an expression to a DFA.
    pub fn compile(e: &PathExpr) -> Result<Dfa> {
        let nfa = Nfa::compile(e)?;
        Dfa::from_nfa(&nfa, nullable(e))
    }

    /// Subset construction over the refined alphabet.
    pub fn from_nfa(nfa: &Nfa, is_nullable: bool) -> Result<Dfa> {
        let syms = nfa.symbols();
        let has_any = syms.iter().any(|(a, _)| *a == AtomKey::Any);
        let mut letters: Vec<Letter> = Vec::new();
        for (a, d) in &syms {
            let kinds: Vec<LetterKind> = match a {
                AtomKey::Iri(i) if has_any => vec![
                    LetterKind::Pred {
                        iri: i.clone(),
                        rv: Some(true),
                    },
                    LetterKind::Pred {
                        iri: i.clone(),
                        rv: Some(false),
                    },
                ],
                AtomKey::Iri(i) => vec![LetterKind::Pred {
                    iri: i.clone(),
                    rv: None,
                }],
                AtomKey::Any => vec![LetterKind::Other],
                AtomKey::VSubject => vec![LetterKind::VSubject],
                AtomKey::VObject => vec![LetterKind::VObject],
                AtomKey::VPredicate => vec![LetterKind::VPredicate],
            };
            for kind in kinds {
                let l = Letter { kind, dir: *d };
                if !letters.contains(&l) {
                    letters.push(l);
                }
            }
        }
        if has_any {
            // `Other` in the direction of every wildcard occurrence is present; a
            // concrete atom in the other direction still needs its own letters
            for (a, d) in &syms {
                if *a == AtomKey::Any {
                    let l = Letter {
                        kind: LetterKind::Other,
                        dir: *d,
                    };
                    if !letters.contains(&l) {
                        letters.push(l);
                    }
                }
            }
        }
        letters.sort();
        let mut start: BTreeSet<usize> = [nfa.start].into();
        nfa.closure(&mut start);
        let mut ids: HashMap<BTreeSet<usize>, u32> = HashMap::new();
        let mut sets: Vec<BTreeSet<usize>> = vec![start.clone()];
        ids.insert(start, 0);
        let mut trans: Vec<Vec<(usize, u32)>> = Vec::new();
        let mut i = 0;
        while i < sets.len() {
            let cur = sets[i].clone();
            let mut row = Vec::new();
            for (li, l) in letters.iter().enumerate() {
                let next = nfa.step(&cur, l);
                if next.is_empty() {
                    continue;
                }
                let id = match ids.get(&next) {
                    Some(id) => *id,
                    None => {
                        if sets.len() >= MAX_DFA_STATES {
                            return Err(too_complex());
                        }
                        let id = sets.len() as u32;
                        ids.insert(next.clone(), id);
                        sets.push(next);
                        id
                    }
                };
                row.push((li, id));
            }
            trans.push(row);
            i += 1;
        }
        let accepting: Vec<bool> = sets.iter().map(|s| s.contains(&nfa.accept)).collect();
        debug_assert_eq!(accepting[0], is_nullable);
        Ok(Dfa {
            letters,
            trans,
            nullable: accepting[0],
            accepting,
            start: 0,
        })
    }

    /// Number of states.
    pub fn len(&self) -> usize {
        self.trans.len()
    }

    /// True when there are no states (never).
    pub fn is_empty(&self) -> bool {
        self.trans.is_empty()
    }

    /// True when the DFA accepts the word (letters must come from `self.letters`).
    pub fn accepts(&self, word: &[Letter]) -> bool {
        let mut s = self.start;
        for l in word {
            let Some(li) = self.letters.iter().position(|x| x == l) else {
                return false;
            };
            match self.trans[s as usize].iter().find(|(k, _)| *k == li) {
                Some((_, t)) => s = *t,
                None => return false,
            }
        }
        self.accepting[s as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn p(n: &str) -> PathExpr {
        PathExpr::iri(format!("urn:{n}"))
    }

    fn rep(e: PathExpr, min: u32, max: Option<u32>) -> PathExpr {
        PathExpr::Repeat {
            inner: Box::new(e),
            min,
            max,
        }
    }

    fn letter(n: &str, dir: Dir) -> Letter {
        Letter {
            kind: LetterKind::Pred {
                iri: format!("urn:{n}"),
                rv: None,
            },
            dir,
        }
    }

    // path-evaluation "Bounded repetition": unrolling sizes (2 states per atom, plus
    // the frame states of the operators)
    #[test]
    fn nfa_state_counts() {
        assert_eq!(Nfa::compile(&p("p")).unwrap().len(), 2);
        // p{2,3}: start + 3 copies (6) + end
        assert_eq!(Nfa::compile(&rep(p("p"), 2, Some(3))).unwrap().len(), 8);
        // p{2,}: start + 2 copies (4) + p* (2 + 2)
        assert_eq!(Nfa::compile(&rep(p("p"), 2, None)).unwrap().len(), 9);
        // (p/q)+: 4 atom states + 2 frame states
        let pq = PathExpr::Seq(vec![p("p"), p("q")]).plus();
        assert_eq!(Nfa::compile(&pq).unwrap().len(), 6);
    }

    // path-evaluation "All shortest does not duplicate ambiguous matches": one run per word
    #[test]
    fn dfa_is_deterministic_for_ambiguous_expressions() {
        for e in [PathExpr::Alt(vec![p("p"), p("p")]), p("p").star().star()] {
            let d = Dfa::compile(&e).unwrap();
            for row in &d.trans {
                let mut ls: Vec<usize> = row.iter().map(|(l, _)| *l).collect();
                let n = ls.len();
                ls.dedup();
                assert_eq!(ls.len(), n, "one target per letter");
            }
            assert!(d.accepts(&[letter("p", Dir::Out)]));
        }
        let d = Dfa::compile(&PathExpr::Alt(vec![p("p"), p("p")])).unwrap();
        assert_eq!(d.len(), 2);
        assert!(!d.accepts(&[letter("p", Dir::Out), letter("p", Dir::Out)]));
    }

    #[test]
    fn inverse_reverses_sequences_and_flips_direction() {
        let e = PathExpr::Inverse(Box::new(PathExpr::Seq(vec![p("a"), p("b")])));
        let d = Dfa::compile(&e).unwrap();
        assert!(d.accepts(&[letter("b", Dir::In), letter("a", Dir::In)]));
        assert!(!d.accepts(&[letter("a", Dir::In), letter("b", Dir::In)]));
    }

    #[test]
    fn wildcard_refines_the_alphabet() {
        let e = PathExpr::Alt(vec![
            PathExpr::iri(tm_ir::vocab::SYS_ANY_RELATIONSHIP),
            p("name"),
        ]);
        let d = Dfa::compile(&e).unwrap();
        // name (rv or not), name in the relationship view, and every other one
        assert_eq!(d.letters.len(), 3);
    }

    #[test]
    fn complexity_guard() {
        let big = rep(PathExpr::Alt(vec![p("a"), p("b")]), 300_000, None);
        match Dfa::compile(&big) {
            Err(Error::Unsupported { feature }) => assert!(feature.contains("too complex")),
            other => panic!("{other:?}"),
        }
        // many distinct states: (a|b)*a(a|b){12} needs 2^12 subsets or more
        let e = PathExpr::Seq(vec![
            PathExpr::Alt(vec![p("a"), p("b")]).star(),
            p("a"),
            rep(PathExpr::Alt(vec![p("a"), p("b")]), 13, Some(13)),
        ]);
        match Dfa::compile(&e) {
            Err(Error::Unsupported { feature }) => assert!(feature.contains("too complex")),
            other => panic!("{other:?}"),
        }
    }

    fn arb_expr() -> impl Strategy<Value = PathExpr> {
        let leaf = prop_oneof![Just(p("a")), Just(p("b")), Just(p("c"))];
        leaf.prop_recursive(4, 24, 3, |inner| {
            prop_oneof![
                inner.clone().prop_map(|e| PathExpr::Inverse(Box::new(e))),
                prop::collection::vec(inner.clone(), 2..3).prop_map(PathExpr::Seq),
                prop::collection::vec(inner.clone(), 2..3).prop_map(PathExpr::Alt),
                inner
                    .clone()
                    .prop_map(|e| PathExpr::ZeroOrMore(Box::new(e))),
                inner.clone().prop_map(|e| PathExpr::OneOrMore(Box::new(e))),
                inner.clone().prop_map(|e| PathExpr::ZeroOrOne(Box::new(e))),
                (inner, 0u32..3, 0u32..3).prop_map(|(e, min, extra)| rep(
                    e,
                    min,
                    Some(min + extra)
                )),
            ]
        })
    }

    proptest! {
        // NFA acceptance equals DFA acceptance
        #[test]
        fn nfa_and_dfa_agree(
            e in arb_expr(),
            word in prop::collection::vec((0usize..3, prop::bool::ANY), 0..6),
        ) {
            let nfa = Nfa::compile(&e).unwrap();
            let dfa = Dfa::from_nfa(&nfa, super::nullable(&e)).unwrap();
            let names = ["a", "b", "c"];
            let w: Vec<Letter> = word
                .iter()
                .map(|(i, out)| letter(names[*i], if *out { Dir::Out } else { Dir::In }))
                .collect();
            prop_assert_eq!(nfa.accepts(&w), dfa.accepts(&w));
        }
    }
}
