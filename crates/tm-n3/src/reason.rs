//! The reasoning run: segments of valid time, one eyeron run per segment,
//! proofs mapped back to statement eids, and the merge of adjacent segments.

use std::collections::{BTreeSet, HashMap};

use eyeron::ast::{LOG_IMPLIED_BY, LOG_IMPLIES};
use eyeron::reasoner::{explain_backward, BackwardStep, DerivedFact};
use eyeron::{Document, Graph, Literal, ReasonerLimit, ReasonerOptions, ReasonerResult};
use eyeron::{Term as NTerm, Triple as NTriple};
use tm_core::rdf::datetime_lexical;
use tm_core::{vocab as core_vocab, Eid, Result, Valid, Value};

use crate::rules::RuleSet;
use crate::term::{from_n3, key, to_n3, Term};
use crate::vocab::{
    is_builtin, TM, TM_CONTENT, TM_NOW, TM_RUN, TM_SEGMENT_FROM, TM_SEGMENT_TO, TM_VALID_FROM,
    TM_VALID_TO,
};

/// The rules of a run: N3 text, or a rule set stored with `store_rules`.
#[derive(Clone, Debug, PartialEq)]
pub enum Rules {
    /// N3 rule text.
    Text(String),
    /// The live `n3:source` of this rule set in the reasoning view.
    Stored(Value),
}

/// Which statements of the view a run loads (before pruning).
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Scope {
    /// Every statement of the view.
    #[default]
    All,
    /// The members of these named graphs.
    Graphs(Vec<Value>),
    /// The statements whose subject or object lies within `hops` statements of
    /// the seed nodes (a statement reached is itself a node for the next hop).
    Seeds {
        /// The seed nodes.
        nodes: Vec<Value>,
        /// The number of statement steps from a seed (1: statements about a seed).
        hops: u32,
    },
}

/// How a run treats valid time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TimeMode {
    /// A conclusion holds at `t` iff it follows from the statements valid at `t`
    /// (design N5): one run per segment of the window.
    #[default]
    Pointwise,
    /// Ignore valid time: one run over every loaded statement, conclusions
    /// unbounded.
    Atemporal,
}

/// Bounds for each eyeron run; `None` keeps eyeron's default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Limits {
    /// Fixpoint steps.
    pub max_iterations: Option<usize>,
    /// Matcher steps per premise search.
    pub max_match_steps: Option<usize>,
    /// Backward-rule recursion depth.
    pub max_backward_depth: Option<usize>,
    /// Bytes of term text one derived fact may carry.
    pub max_term_bytes: Option<usize>,
}

/// The options of a run.
#[derive(Clone, Debug, PartialEq)]
pub struct ReasonOptions {
    /// The statements loaded (default: the whole view).
    pub scope: Scope,
    /// The valid-time window of a pointwise sweep (default: unbounded).
    /// Conclusions are reported only inside it.
    pub window: Valid,
    /// Pointwise (default) or atemporal.
    pub time: TimeMode,
    /// The most segments a sweep runs (default 1 024); past it the status is
    /// `Incomplete { limits: [Segments] }`.
    pub max_segments: usize,
    /// Add eyeron's N3 proof text to the result.
    pub proof_n3: bool,
    /// eyeron's limits for each run.
    pub limits: Limits,
}

impl Default for ReasonOptions {
    fn default() -> ReasonOptions {
        ReasonOptions {
            scope: Scope::All,
            window: Valid::ALWAYS,
            time: TimeMode::Pointwise,
            max_segments: 1024,
            proof_n3: false,
            limits: Limits::default(),
        }
    }
}

/// One loaded statement.
#[derive(Clone, Debug, PartialEq)]
pub struct Fact {
    /// Its eid.
    pub eid: Eid,
    /// Subject.
    pub s: Value,
    /// Predicate.
    pub p: Value,
    /// Object.
    pub o: Value,
    /// Its valid time.
    pub valid: Valid,
}

/// What a run reasons over: the loaded statements and the clock.
#[derive(Clone, Debug)]
pub struct Input<'a> {
    /// The loaded statements.
    pub facts: &'a [Fact],
    /// The pinned clock (epoch ms): `tm:run tm:now` and `time:localTime`.
    pub now: i64,
    /// `Some(t)` for a `valid_at(t)` view: one snapshot, conclusions `[t, t+1)`.
    pub snapshot: Option<i64>,
    /// The stored rule set the rules came from, named in every proof.
    pub rule_set: Option<Value>,
}

impl<'a> Input<'a> {
    /// The statements `facts` at clock `now`, no snapshot, no stored rule set.
    pub fn new(facts: &'a [Fact], now: i64) -> Input<'a> {
        Input {
            facts,
            now,
            snapshot: None,
            rule_set: None,
        }
    }
}

/// A premise of a conclusion.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Premise {
    /// A loaded statement (a fact, or the statement a virtual `tm:` triple is about).
    Statement(Eid),
    /// Another conclusion of the same result, by index (always a lower one).
    Conclusion(usize),
}

/// How a conclusion was reached (design N15).
#[derive(Clone, Debug, PartialEq)]
pub struct Proof {
    /// The stored rule set, `None` for rule text.
    pub rule_set: Option<Value>,
    /// The rule's index in its rule set (text order); an index past the rule
    /// count names a rule the run generated from data.
    pub rule: usize,
    /// The statements and earlier conclusions the rule instance used, in body
    /// order. Builtins, `tm:run` facts and facts of the rule text are not premises.
    pub premises: Vec<Premise>,
}

/// The identity of a conclusion's support (design N12): the rule, the premise
/// statements and, recursively, the content and support of the derived
/// premises. Conclusions of adjacent segments merge only when content and
/// support key are equal.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SupportKey {
    /// The rule index.
    pub rule: usize,
    /// The premise statements, ascending.
    pub statements: Vec<Eid>,
    /// The derived premises: content (as text) and support key, ascending.
    pub derived: Vec<(String, SupportKey)>,
}

/// One conclusion with the valid time it holds over.
#[derive(Clone, Debug, PartialEq)]
pub struct Conclusion {
    /// Subject.
    pub s: Term,
    /// Predicate.
    pub p: Term,
    /// Object.
    pub o: Term,
    /// The union of the segments it holds in with the same support, `[t, t+1)`
    /// in a snapshot, unbounded when atemporal.
    pub valid: Valid,
    /// The rule and premises.
    pub proof: Proof,
    /// The support identity used to merge segments.
    pub support: SupportKey,
}

impl Conclusion {
    /// The content as stored values, when every term is one.
    pub fn values(&self) -> Option<(Value, Value, Value)> {
        Some((
            self.s.as_value()?.clone(),
            self.p.as_value()?.clone(),
            self.o.as_value()?.clone(),
        ))
    }
}

/// A limit that stopped a run short of a complete answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Limit {
    /// More segments than `max_segments`.
    Segments,
    /// eyeron's fixpoint step limit.
    Iterations,
    /// eyeron's matcher step limit.
    MatchSteps,
    /// eyeron's backward-rule depth limit.
    BackwardDepth,
    /// eyeron's limit on the solutions of one backward goal.
    BackwardSolutions,
    /// eyeron's limit on the term bytes of one derived fact.
    TermBytes,
    /// eyeron's limit on the nesting of one derived fact.
    TermDepth,
}

/// Whether the result is a complete answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// Every segment ran to a fixpoint.
    Complete,
    /// Some limit was reached or a builtin failed (`errors`); every reported
    /// conclusion is still sound for the segments that ran.
    Incomplete {
        /// Every limit reached, ascending.
        limits: Vec<Limit>,
    },
}

/// A fired inference fuse (`{ … } => false`).
#[derive(Clone, Debug, PartialEq)]
pub struct Contradiction {
    /// The fuse rule's index.
    pub rule: usize,
    /// The rule body with the bindings that fired it, as N3 triples.
    pub instance: Vec<String>,
    /// The segment it fired in.
    pub valid: Valid,
}

/// The result of a run.
#[derive(Clone, Debug, PartialEq)]
pub struct ReasonResult {
    /// The conclusions not among the loaded statements, in proof order (a
    /// premise conclusion comes before every conclusion that uses it).
    pub conclusions: Vec<Conclusion>,
    /// The view's transaction number (set by the loader).
    pub t: u64,
    /// The pinned clock (epoch ms).
    pub now: i64,
    /// The number of loaded statements.
    pub loaded: usize,
    /// True when pruning by body predicates applied (set by the loader).
    pub pruned: bool,
    /// The number of segments reasoned.
    pub segments: usize,
    /// `Complete` or `Incomplete { limits }`.
    pub status: Status,
    /// Builtin failures eyeron reported (they make the status incomplete).
    pub errors: Vec<String>,
    /// The first fired inference fuse, if any.
    pub contradiction: Option<Contradiction>,
    /// The stored rule set, if the rules came from one.
    pub rule_set: Option<Value>,
    /// The rule set's live `n3:source` statement (set by the loader).
    pub rule_set_statement: Option<Eid>,
    /// eyeron's N3 proof, one document per segment, with `proof_n3`.
    pub proof_n3: Option<String>,
}

fn dt(ms: i64) -> NTerm {
    NTerm::Literal(Literal {
        value: datetime_lexical(ms, Some(0)).into(),
        datatype: Some(core_vocab::XSD_DATETIME.into()),
        language: None,
    })
}

fn limit_of(l: ReasonerLimit) -> Limit {
    match l {
        ReasonerLimit::Iterations => Limit::Iterations,
        ReasonerLimit::MatchSteps => Limit::MatchSteps,
        ReasonerLimit::BackwardDepth => Limit::BackwardDepth,
        ReasonerLimit::BackwardSolutionsPerGoal => Limit::BackwardSolutions,
        ReasonerLimit::TermSize => Limit::TermBytes,
        ReasonerLimit::TermDepth => Limit::TermDepth,
    }
}

/// True when the statement valid over `v` is valid throughout `seg`.
fn covers(v: &Valid, seg: &Valid) -> bool {
    let from_ok = match (v.from, seg.from) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(a), Some(b)) => a <= b,
    };
    let to_ok = match (v.to, seg.to) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(a), Some(b)) => a >= b,
    };
    from_ok && to_ok
}

/// The segments of a pointwise sweep: the window cut at every bound inside it.
// @tg: implements:: [[openspec:n3-reasoning#Pointwise valid time]] {since: "2026-10-28"}
fn segments(facts: &[Fact], window: Valid) -> Vec<Valid> {
    let inside = |x: i64| window.from.is_none_or(|f| f < x) && window.to.is_none_or(|t| x < t);
    let cuts: BTreeSet<i64> = facts
        .iter()
        .flat_map(|f| [f.valid.from, f.valid.to])
        .flatten()
        .filter(|&x| inside(x))
        .collect();
    let mut bounds: Vec<Option<i64>> = vec![window.from];
    bounds.extend(cuts.into_iter().map(Some));
    bounds.push(window.to);
    bounds
        .windows(2)
        .map(|w| Valid {
            from: w[0],
            to: w[1],
        })
        .collect()
}

/// One loaded statement in N3, with its virtual triples.
struct Loaded {
    eid: Eid,
    valid: Valid,
    triple: NTriple,
    key: String,
    virtuals: Vec<NTriple>,
}

/// A conclusion of one segment, before merging.
struct Local {
    triple: NTriple,
    key: String,
    rule: usize,
    premises: Vec<Premise>,
}

/// Runs `rules` over `input` (design N5, N7, N12, N15). Pure: it never reads or
/// writes a store. `t`, `pruned` and `rule_set_statement` of the result are left
/// for the loader to fill in.
///
/// Between segments it polls the current operation's budget
/// ([`tm_core::budget::check`]), so a cancelled or timed-out run stops there.
///
/// # Errors
///
/// `Cancelled` or `DeadlineExceeded` from the budget.
// @tg: implements:: [[openspec:n3-reasoning#Proofs and completion]] {since: "2026-10-28"}
pub fn reason(rules: &RuleSet, input: &Input<'_>, opts: &ReasonOptions) -> Result<ReasonResult> {
    let mut facts: Vec<&Fact> = input.facts.iter().collect();
    facts.sort_by_key(|f| f.eid);
    let v = rules.virtuals;
    let loaded: Vec<Loaded> = facts
        .iter()
        .map(|f| {
            let triple = NTriple::new(to_n3(&f.s), to_n3(&f.p), to_n3(&f.o));
            let e = to_n3(&Value::Stmt(f.eid));
            let mut virtuals = Vec::new();
            if v.content {
                virtuals.push(NTriple::new(
                    e.clone(),
                    NTerm::iri(TM_CONTENT),
                    NTerm::Formula(Graph::from(vec![triple.clone()])),
                ));
            }
            if let (true, Some(from)) = (v.valid_from, f.valid.from) {
                virtuals.push(NTriple::new(e.clone(), NTerm::iri(TM_VALID_FROM), dt(from)));
            }
            if let (true, Some(to)) = (v.valid_to, f.valid.to) {
                virtuals.push(NTriple::new(e, NTerm::iri(TM_VALID_TO), dt(to)));
            }
            Loaded {
                eid: f.eid,
                valid: f.valid,
                key: key(&triple),
                triple,
                virtuals,
            }
        })
        .collect();

    // (segment, the instant of a snapshot)
    let (plan, atemporal): (Vec<Valid>, bool) = match (input.snapshot, opts.time) {
        (Some(t), _) => (vec![Valid::between(t, t.saturating_add(1))], false),
        (None, TimeMode::Atemporal) => (vec![Valid::ALWAYS], true),
        (None, TimeMode::Pointwise) => (segments(input.facts, opts.window), false),
    };

    let mut options = ReasonerOptions {
        proof: true,
        ..ReasonerOptions::default()
    };
    let l = opts.limits;
    if let Some(x) = l.max_iterations {
        options.max_iterations = x;
    }
    if let Some(x) = l.max_match_steps {
        options.max_match_steps = x;
    }
    if let Some(x) = l.max_backward_depth {
        options.max_backward_depth = x;
    }
    if let Some(x) = l.max_term_bytes {
        options.max_term_bytes = x;
    }

    let mut out = ReasonResult {
        conclusions: Vec::new(),
        t: 0,
        now: input.now,
        loaded: input.facts.len(),
        pruned: false,
        segments: 0,
        status: Status::Complete,
        errors: Vec::new(),
        contradiction: None,
        rule_set: input.rule_set.clone(),
        rule_set_statement: None,
        proof_n3: opts.proof_n3.then(String::new),
    };
    let mut limits = BTreeSet::new();
    if plan.len() > opts.max_segments {
        limits.insert(Limit::Segments);
    }
    // (content, support) of the previous segment's conclusions -> global index
    let mut open: HashMap<(String, SupportKey), usize> = HashMap::new();
    let run_facts = |seg: &Valid| -> Vec<NTriple> {
        let mut v = vec![NTriple::new(
            NTerm::iri(TM_RUN),
            NTerm::iri(TM_NOW),
            dt(input.now),
        )];
        if !atemporal {
            if let Some(f) = seg.from {
                v.push(NTriple::new(
                    NTerm::iri(TM_RUN),
                    NTerm::iri(TM_SEGMENT_FROM),
                    dt(f),
                ));
            }
            if let Some(t) = seg.to {
                v.push(NTriple::new(
                    NTerm::iri(TM_RUN),
                    NTerm::iri(TM_SEGMENT_TO),
                    dt(t),
                ));
            }
        }
        v
    };

    for seg in plan.iter().take(opts.max_segments) {
        tm_core::budget::check()?;
        out.segments += 1;
        let here: Vec<&Loaded> = loaded
            .iter()
            .filter(|l| atemporal || covers(&l.valid, seg))
            .collect();
        let mut data = Document::new();
        // content key -> lowest eid with that content
        let mut by_key: HashMap<&str, Eid> = HashMap::new();
        for l in &here {
            by_key.entry(l.key.as_str()).or_insert(l.eid);
            data.facts.push(l.triple.clone());
            data.facts.extend(l.virtuals.iter().cloned());
        }
        if v.run {
            data.facts.extend(run_facts(seg));
        }
        let inputs: BTreeSet<String> = data.facts.iter().map(key).collect();
        let result = rules.program.reason(&data, &options);

        limits.extend(result.limits_reached.iter().map(|&l| limit_of(l)));
        out.errors
            .extend(result.errors.iter().map(ToString::to_string));
        if let (Some(f), None) = (&result.fuse, &out.contradiction) {
            out.contradiction = Some(Contradiction {
                rule: rules.index_of(&f.rule).unwrap_or(rules.rules.len()),
                instance: f
                    .instance
                    .iter()
                    .map(|t| eyeron::printing::triple_to_n3(&rules.prefixes, t))
                    .collect(),
                valid: *seg,
            });
        }
        if let Some(text) = out.proof_n3.as_mut() {
            let proof = eyeron::proof_to_n3(&rules.prefixes, &result);
            if !proof.is_empty() {
                text.push_str(&format!("# segment {}\n{proof}\n", show(seg)));
            }
        }

        let locals = segment_conclusions(rules, &result, &by_key, &inputs);
        let mut local_support: Vec<SupportKey> = Vec::with_capacity(locals.len());
        let mut global_of: Vec<usize> = Vec::with_capacity(locals.len());
        let mut next: HashMap<(String, SupportKey), usize> = HashMap::new();
        for c in locals.iter() {
            let mut statements: Vec<Eid> = Vec::new();
            let mut derived: Vec<(String, SupportKey)> = Vec::new();
            for p in &c.premises {
                match *p {
                    Premise::Statement(e) => statements.push(e),
                    Premise::Conclusion(j) => {
                        derived.push((locals[j].key.clone(), local_support[j].clone()))
                    }
                }
            }
            statements.sort();
            statements.dedup();
            derived.sort();
            derived.dedup();
            let support = SupportKey {
                rule: c.rule,
                statements,
                derived,
            };
            let k = (c.key.clone(), support.clone());
            let g = match open.get(&k) {
                Some(&g) if out.conclusions[g].valid.to == seg.from => {
                    out.conclusions[g].valid.to = seg.to;
                    g
                }
                _ => {
                    let premises = c
                        .premises
                        .iter()
                        .map(|p| match *p {
                            Premise::Conclusion(j) => Premise::Conclusion(global_of[j]),
                            s => s,
                        })
                        .collect();
                    out.conclusions.push(Conclusion {
                        s: from_n3(&c.triple.s),
                        p: from_n3(&c.triple.p),
                        o: from_n3(&c.triple.o),
                        valid: *seg,
                        proof: Proof {
                            rule_set: input.rule_set.clone(),
                            rule: c.rule,
                            premises,
                        },
                        support: support.clone(),
                    });
                    out.conclusions.len() - 1
                }
            };
            next.insert(k, g);
            global_of.push(g);
            local_support.push(support);
        }
        open = next;
    }

    if !limits.is_empty() || !out.errors.is_empty() {
        out.status = Status::Incomplete {
            limits: limits.into_iter().collect(),
        };
    }
    Ok(out)
}

fn show(v: &Valid) -> String {
    let b = |x: Option<i64>, inf: &str| x.map_or(inf.to_string(), |x| x.to_string());
    format!("[{}, {})", b(v.from, "-inf"), b(v.to, "+inf"))
}

/// The conclusions of one eyeron run, in derivation order, with their premises
/// mapped to loaded statements and earlier conclusions.
fn segment_conclusions(
    rules: &RuleSet,
    result: &ReasonerResult,
    by_key: &HashMap<&str, Eid>,
    inputs: &BTreeSet<String>,
) -> Vec<Local> {
    let mut proofs: HashMap<String, &DerivedFact> = HashMap::new();
    for p in &result.proofs {
        proofs.entry(key(&p.fact)).or_insert(p);
    }
    let mut locals: Vec<Local> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut ctx = Resolve {
        result,
        by_key,
        proofs: &proofs,
        backward: HashMap::new(),
        given: None,
    };
    for fact in &result.derived {
        let k = key(fact);
        if inputs.contains(&k) || index.contains_key(&k) {
            continue;
        }
        // `tm:` is reserved: virtual triples are never conclusions; a concluded
        // rule is a rule of the run, not a statement
        if matches!(&fact.p, NTerm::Iri(p) if p.starts_with(TM) || p == LOG_IMPLIES || p == LOG_IMPLIED_BY)
        {
            continue;
        }
        let (rule, premises) = match proofs.get(&k) {
            Some(df) => {
                let mut premises = Vec::new();
                let mut seen = BTreeSet::new();
                seen.insert(k.clone());
                for p in &df.premises {
                    ctx.premise(p, &index, &mut premises, &mut seen);
                }
                let mut uniq = Vec::new();
                for p in premises {
                    if !uniq.contains(&p) {
                        uniq.push(p);
                    }
                }
                (rules.index_of(&df.rule).unwrap_or(rules.rules.len()), uniq)
            }
            None => (rules.rules.len(), Vec::new()),
        };
        index.insert(k.clone(), locals.len());
        locals.push(Local {
            triple: fact.clone(),
            key: k,
            rule,
            premises,
        });
    }
    locals
}

/// Maps the premise triples of a proof back to statements and conclusions.
struct Resolve<'a> {
    result: &'a ReasonerResult,
    by_key: &'a HashMap<&'a str, Eid>,
    proofs: &'a HashMap<String, &'a DerivedFact>,
    /// Backward proofs already explained: goal key -> premises.
    backward: HashMap<String, Vec<Premise>>,
    /// The facts of the run (what a backward explanation may end in).
    given: Option<BTreeSet<NTriple>>,
}

impl Resolve<'_> {
    /// A statement or an earlier conclusion that is exactly this triple.
    fn direct(&self, t: &NTriple, index: &HashMap<String, usize>) -> Option<Option<Premise>> {
        let NTerm::Iri(p) = &t.p else {
            return None;
        };
        if is_builtin(p) {
            return Some(None);
        }
        if matches!(&t.s, NTerm::Iri(s) if s == TM_RUN) {
            return Some(None);
        }
        if p == TM_CONTENT || p == TM_VALID_FROM || p == TM_VALID_TO {
            if let Some(Value::Stmt(e)) = crate::term::value_of(&t.s) {
                return Some(Some(Premise::Statement(e)));
            }
        }
        let k = key(t);
        if let Some(&e) = self.by_key.get(k.as_str()) {
            return Some(Some(Premise::Statement(e)));
        }
        index.get(&k).map(|&i| Some(Premise::Conclusion(i)))
    }

    fn premise(
        &mut self,
        t: &NTriple,
        index: &HashMap<String, usize>,
        out: &mut Vec<Premise>,
        seen: &mut BTreeSet<String>,
    ) {
        if let Some(found) = self.direct(t, index) {
            out.extend(found);
            return;
        }
        let k = key(t);
        if !seen.insert(k.clone()) {
            return;
        }
        // derived, but not reported (a query run reports only its answers)
        if let Some(df) = self.proofs.get(&k) {
            for p in &df.premises {
                self.premise(p, index, out, seen);
            }
            return;
        }
        // proved by backward rules: the facts at the leaves of its proof
        if let Some(found) = self.backward.get(&k) {
            out.extend(found.iter().copied());
            return;
        }
        let given = self
            .given
            .get_or_insert_with(|| self.result.explicit.iter().cloned().collect());
        let mut leaves = Vec::new();
        explain_backward(
            t,
            &self.result.closure,
            given,
            &self.result.rules,
            ReasonerOptions::default().max_backward_depth,
            &mut |step| match step {
                BackwardStep::Fact { fact } | BackwardStep::Unproven { fact, .. } => {
                    leaves.push(fact)
                }
                BackwardStep::Rule(_) | BackwardStep::Builtin { .. } => {}
            },
        );
        let mut found = Vec::new();
        for fact in &leaves {
            match self.direct(fact, index) {
                Some(p) => found.extend(p),
                None => {
                    if let Some(df) = self.proofs.get(&key(fact)) {
                        for p in &df.premises {
                            self.premise(p, index, &mut found, seen);
                        }
                    }
                }
            }
        }
        out.extend(found.iter().copied());
        self.backward.insert(k, found);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Value {
        Value::iri(format!("urn:tiramemsu:v:{s}"))
    }

    fn fact(n: u64, s: &str, p: &str, o: &str, valid: Valid) -> Fact {
        Fact {
            eid: Eid::new(n),
            s: v(s),
            p: if p == "a" {
                Value::iri(core_vocab::RDF_TYPE)
            } else {
                v(p)
            },
            o: v(o),
            valid,
        }
    }

    #[test]
    fn segments_cut_the_window() {
        let f = [
            fact(1, "a", "p", "b", Valid::between(10, 20)),
            fact(2, "a", "p", "c", Valid::from(15)),
        ];
        assert_eq!(
            segments(&f, Valid::ALWAYS),
            [
                Valid::until(10),
                Valid::between(10, 15),
                Valid::between(15, 20),
                Valid::from(20)
            ]
        );
        assert_eq!(
            segments(&f, Valid::between(12, 18)),
            [Valid::between(12, 15), Valid::between(15, 18)]
        );
        assert_eq!(segments(&[], Valid::ALWAYS), [Valid::ALWAYS]);
    }

    #[test]
    fn overlap_gives_the_intersection() {
        let rules =
            RuleSet::parse("{ ?p v:worksAt ?c. ?c v:locatedIn ?l } => { ?p v:worksIn ?l } .")
                .unwrap();
        let facts = [
            fact(1, "alice", "worksAt", "acme", Valid::between(2020, 2025)),
            fact(2, "acme", "locatedIn", "berlin", Valid::from(2024)),
        ];
        let r = reason(&rules, &Input::new(&facts, 0), &ReasonOptions::default()).unwrap();
        assert_eq!(r.conclusions.len(), 1);
        let c = &r.conclusions[0];
        assert_eq!(c.valid, Valid::between(2024, 2025));
        assert_eq!(
            c.proof.premises,
            [
                Premise::Statement(Eid::new(1)),
                Premise::Statement(Eid::new(2))
            ]
        );
        assert_eq!(r.segments, 4);
        assert_eq!(r.status, Status::Complete);
    }

    #[test]
    fn derived_premises_and_merging() {
        let rules = RuleSet::parse(
            "{ ?x a v:Human } => { ?x a v:Mortal } . { ?x a v:Mortal } => { ?x v:has v:lifespan } .",
        )
        .unwrap();
        // the other fact only adds cuts: the conclusions merge across them
        let facts = [
            fact(1, "socrates", "a", "Human", Valid::ALWAYS),
            fact(2, "plato", "a", "Greek", Valid::between(5, 9)),
        ];
        let r = reason(&rules, &Input::new(&facts, 0), &ReasonOptions::default()).unwrap();
        assert_eq!(r.segments, 3);
        assert_eq!(r.conclusions.len(), 2);
        assert_eq!(r.conclusions[0].valid, Valid::ALWAYS);
        assert_eq!(r.conclusions[1].proof.premises, [Premise::Conclusion(0)]);
        assert_eq!(r.conclusions[1].proof.rule, 1);
    }

    #[test]
    fn backward_rule_premises_reach_the_facts() {
        let rules =
            RuleSet::parse("{ ?x v:r ?y } <= { ?x v:q ?y } . { ?x v:r ?y } => { ?x v:s ?y } .")
                .unwrap();
        let facts = [fact(7, "a", "q", "b", Valid::ALWAYS)];
        let r = reason(&rules, &Input::new(&facts, 0), &ReasonOptions::default()).unwrap();
        assert_eq!(r.conclusions.len(), 1);
        assert_eq!(r.conclusions[0].proof.rule, 1);
        assert_eq!(
            r.conclusions[0].proof.premises,
            [Premise::Statement(Eid::new(7))]
        );
    }

    #[test]
    fn segment_limit() {
        let rules = RuleSet::parse("{ ?x a v:Human } => { ?x a v:Mortal } .").unwrap();
        let facts: Vec<Fact> = (1..=10)
            .map(|i| {
                fact(
                    i,
                    "s",
                    "a",
                    "Human",
                    Valid::between(i as i64 * 10, i as i64 * 10 + 5),
                )
            })
            .collect();
        let opts = ReasonOptions {
            max_segments: 4,
            ..ReasonOptions::default()
        };
        let r = reason(&rules, &Input::new(&facts, 0), &opts).unwrap();
        assert_eq!(r.segments, 4);
        assert_eq!(
            r.status,
            Status::Incomplete {
                limits: vec![Limit::Segments]
            }
        );
        // sound for the segments that ran: up to 25, from two premises
        let valid: Vec<Valid> = r.conclusions.iter().map(|c| c.valid).collect();
        assert_eq!(valid, [Valid::between(10, 15), Valid::between(20, 25)]);
    }

    #[test]
    fn blank_nodes_keep_their_label() {
        let rules = RuleSet::parse("{ ?x a v:Employee } => { ?x v:manager _:m } .").unwrap();
        let facts = [fact(1, "bob", "a", "Employee", Valid::ALWAYS)];
        let a = reason(&rules, &Input::new(&facts, 0), &ReasonOptions::default()).unwrap();
        let b = reason(&rules, &Input::new(&facts, 5), &ReasonOptions::default()).unwrap();
        assert!(matches!(&a.conclusions[0].o, Term::Blank(l) if l.starts_with("m_")));
        assert_eq!(a.conclusions[0].o, b.conclusions[0].o);
    }
}
