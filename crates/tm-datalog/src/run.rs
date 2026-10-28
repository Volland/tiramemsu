//! Running a program over a view: the public entry points and result types
//! (`lat.md/datalog#Own Calls On Every Surface`).

use std::collections::BTreeSet;

use rustc_hash::{FxHashMap, FxHashSet};
use tm_core::store::{Pattern, ReadStore};
use tm_core::vocab::{DL_SOURCE, SYS_IN_GRAPH};
use tm_core::{
    Eid, Error, ObjectId, Result, Tag, TimeRef, TxSel, Valid, ValidSel, Value, ViewSpec,
};
use tm_store::read::View;

use crate::eval::{Engine, EvalOptions, Limit, Limits, Prem};
use crate::interval::{IvSet, NEG_INF, POS_INF};
use crate::program::{self, Analysis, RuleKind};
use crate::source::{Stored, TimeView};
use crate::syntax::{Pred, Severity};
use crate::terms::{self, Id, Term, Terms};

/// A program to run: text, or a stored program node.
#[derive(Clone, Debug, PartialEq)]
pub enum Program {
    /// Program text.
    Text(String),
    /// The program stored on this node (`(node dl:source "<text>")`).
    Stored(Value),
}

/// What a run reads.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Scope {
    /// The whole view.
    #[default]
    All,
    /// The members of these named graphs.
    Graphs(Vec<Value>),
    /// The statements within `hops` of these nodes.
    Seeds {
        /// The seed nodes.
        nodes: Vec<Value>,
        /// How far to follow statements.
        hops: usize,
    },
}

/// How valid time is treated.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum TimeMode {
    /// A fact holds at an instant iff it follows from the facts valid then.
    #[default]
    Pointwise,
    /// Valid time is ignored.
    Atemporal,
}

/// How stored atoms are read (`Auto` decides from the probe count).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum AccessMode {
    /// Probe, then load past an eighth of a predicate's statements.
    #[default]
    Auto,
    /// Always probe.
    Probe,
    /// Always load.
    Load,
}

/// Options of a run.
#[derive(Clone, Debug, PartialEq)]
pub struct DatalogOptions {
    /// What to read.
    pub scope: Scope,
    /// The valid-time window results (and recursive temporal strata) are clipped to.
    pub window: Option<Valid>,
    /// How valid time is treated.
    pub time: TimeMode,
    /// Limits.
    pub limits: Limits,
    /// Rewrite goals with constants for demand (magic sets).
    pub demand: bool,
    /// Return one derivation per row and fragment.
    pub proof: bool,
    /// Also report the facts of plain-named relations (`datalog_reason`).
    pub include_relations: bool,
    /// Fail with `DatalogIncomplete` instead of returning partial results.
    pub require_complete: bool,
    /// Deltas larger than this run as parallel morsels (0: never).
    pub parallel_threshold: usize,
    /// How stored atoms are read.
    pub access: AccessMode,
}

impl Default for DatalogOptions {
    fn default() -> Self {
        DatalogOptions {
            scope: Scope::All,
            window: None,
            time: TimeMode::Pointwise,
            limits: Limits::default(),
            demand: true,
            proof: false,
            include_relations: false,
            require_complete: false,
            parallel_threshold: 4096,
            access: AccessMode::Auto,
        }
    }
}

/// Whether a run reached a limit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// Every fact was derived.
    Complete,
    /// A limit stopped the run; every reported fact still follows.
    Incomplete {
        /// The limits reached.
        limits: Vec<String>,
    },
}

/// One premise of a proof.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Premise {
    /// A stored statement.
    Statement(Eid),
    /// A derived fact: an index into the result's `facts`.
    Fact(usize),
}

/// One derivation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proof {
    /// The rule's index among the text's rules; `None` for a goal or constraint.
    pub rule: Option<usize>,
    /// The premises.
    pub premises: Vec<Premise>,
}

/// A derivation of one interval of a row.
#[derive(Clone, Debug, PartialEq)]
pub struct FragmentProof {
    /// The interval.
    pub valid: Valid,
    /// Its derivation.
    pub proof: Proof,
}

/// A derived fact referenced by proofs (or reported by `include_relations`).
#[derive(Clone, Debug, PartialEq)]
pub struct Fact {
    /// The relation: an IRI or a plain name.
    pub relation: String,
    /// The arguments.
    pub args: Vec<Term>,
    /// Its valid time.
    pub valid: Vec<Valid>,
    /// One derivation (proofs on).
    pub proof: Option<Proof>,
}

/// One goal row.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    /// One value per column.
    pub values: Vec<Term>,
    /// The valid time of the binding: disjoint, non-adjacent, ascending.
    pub valid: Vec<Valid>,
    /// One derivation per maximal interval (proofs on).
    pub proof: Option<Vec<FragmentProof>>,
}

/// The result of a goal query.
#[derive(Clone, Debug, PartialEq)]
pub struct DatalogResult {
    /// The goal's named variables.
    pub columns: Vec<String>,
    /// The rows, sorted by value, then by interval start.
    pub rows: Vec<Row>,
    /// Derived facts the proofs reference.
    pub facts: Vec<Fact>,
    /// The view's transaction number.
    pub t: u64,
    /// The pinned clock, epoch ms.
    pub now: i64,
    /// The number of strata run.
    pub strata: usize,
    /// The facts derived, demand facts included.
    pub derived: u64,
    /// The stored predicates read.
    pub predicates: Vec<String>,
    /// True when only the predicates the bodies name were read.
    pub pruned: bool,
    /// Whether demand rewriting applied, and why parts were evaluated in full.
    pub demand: Option<Vec<String>>,
    /// Completion.
    pub status: Status,
    /// The stored program, if any.
    pub program: Option<Value>,
    /// The program's `dl:source` statement.
    pub program_statement: Option<Eid>,
}

/// One conclusion of `datalog_reason`.
#[derive(Clone, Debug, PartialEq)]
pub struct Conclusion {
    /// Subject.
    pub s: Term,
    /// Predicate IRI.
    pub p: Value,
    /// Object.
    pub o: Term,
    /// One maximal interval.
    pub valid: Valid,
    /// Its derivation (proofs on).
    pub proof: Option<Proof>,
}

/// The result of `datalog_reason`.
#[derive(Clone, Debug, PartialEq)]
pub struct ReasonResult {
    /// The conclusions not among the stored statements, sorted.
    pub conclusions: Vec<Conclusion>,
    /// Facts of plain-named relations (with `include_relations`).
    pub relations: Vec<Fact>,
    /// Derived facts the proofs reference.
    pub facts: Vec<Fact>,
    /// The view's transaction number.
    pub t: u64,
    /// The pinned clock.
    pub now: i64,
    /// The stored predicates read.
    pub predicates: Vec<String>,
    /// Completion.
    pub status: Status,
    /// The stored program, if any.
    pub program: Option<Value>,
    /// The program's `dl:source` statement.
    pub program_statement: Option<Eid>,
}

/// One violation of a constraint.
#[derive(Clone, Debug, PartialEq)]
pub struct Violation {
    /// The body's named variables and their values.
    pub bindings: Vec<(String, Term)>,
    /// The statements the violation's derivation used, ascending.
    pub eids: Vec<Eid>,
    /// When it holds.
    pub valid: Vec<Valid>,
}

/// The violations of one constraint.
#[derive(Clone, Debug, PartialEq)]
pub struct ConstraintReport {
    /// Its name.
    pub name: String,
    /// Its severity.
    pub severity: Severity,
    /// Its violations.
    pub violations: Vec<Violation>,
}

/// The result of `check`.
#[derive(Clone, Debug, PartialEq)]
pub struct CheckResult {
    /// The violated constraints, in program order.
    pub violated: Vec<ConstraintReport>,
    /// The constraints evaluated.
    pub checked: Vec<String>,
    /// The view's transaction number.
    pub t: u64,
    /// The pinned clock.
    pub now: i64,
    /// Completion.
    pub status: Status,
}

/// What a program is.
#[derive(Clone, Debug, PartialEq)]
pub struct ProgramInfo {
    /// The stored program node.
    pub program: Option<Value>,
    /// Its `dl:source` statement.
    pub statement: Option<Eid>,
    /// Its text.
    pub text: String,
    /// The analysis.
    pub analysis: Analysis,
}

/// A run's plan and result.
#[derive(Clone, Debug, PartialEq)]
pub struct Explain {
    /// The plan, per stratum and rule, with access choices.
    pub plan: Vec<String>,
    /// The result.
    pub result: DatalogResult,
}

/// The live `dl:source` statement of `program` in the view, and its text.
pub fn stored_program(
    store: &dyn ReadStore,
    spec: &ViewSpec,
    program: &Value,
) -> Result<Option<(String, Eid)>> {
    let (Some(s), Some(p)) = (
        tm_store::dict::encode(store, program)?,
        tm_store::dict::encode(store, &Value::iri(DL_SOURCE))?,
    ) else {
        return Ok(None);
    };
    let Some(row) = tm_store::read::triples(store, spec, Some(s), Some(p), None)?.pop() else {
        return Ok(None);
    };
    let terms = tm_store::dict::TermCache::new(4);
    match terms.decode(store, row.o, false)? {
        Value::Str(text) => Ok(Some((text, row.eid))),
        _ => Ok(None),
    }
}

fn text_of(
    store: &dyn ReadStore,
    spec: &ViewSpec,
    program: &Program,
) -> Result<(String, Option<Value>, Option<Eid>)> {
    match program {
        Program::Text(t) => Ok((t.clone(), None, None)),
        Program::Stored(v) => {
            let (t, e) = stored_program(store, spec, v)?.ok_or_else(|| Error::UnknownProgram {
                program: v.to_string(),
            })?;
            Ok((t, Some(v.clone()), Some(e)))
        }
    }
}

/// The view's transaction and instant: the pinned clock (`clock` on a now view).
// @tg: implements:: [[openspec:datalog-temporal#Transaction time and the pinned clock]] {since: "2026-10-28"}
fn clock_of(store: &dyn ReadStore, spec: &ViewSpec, clock: i64) -> Result<(u64, i64)> {
    let last = tm_store::read::last_t(store)?;
    Ok(match spec.tx {
        TxSel::Now | TxSel::History => (last, clock),
        TxSel::AsOf(TimeRef::Tx(t)) => match tm_store::read::tx_instant(store, t)? {
            Some(i) => (t, i),
            None if t > last => (t, clock),
            None => (t, 0),
        },
        TxSel::AsOf(TimeRef::Instant(ms)) => (store.tx_at_or_before(ms)?.max(0) as u64, ms),
    })
}

fn is_node(id: ObjectId) -> bool {
    matches!(id.tag(), Ok(Tag::Iri | Tag::Node | Tag::BNode | Tag::Stmt))
}

/// The eids a scope admits (`None`: every statement).
fn scope_eids(store: &dyn ReadStore, view: &View, scope: &Scope) -> Result<Option<FxHashSet<Id>>> {
    match scope {
        Scope::All => Ok(None),
        Scope::Graphs(graphs) => {
            let mut out = FxHashSet::default();
            let Some(member) = tm_store::dict::encode(store, &Value::iri(SYS_IN_GRAPH))? else {
                return Ok(Some(out));
            };
            for g in graphs {
                if let Some(g) = tm_store::dict::encode(store, g)? {
                    for r in store.scan(
                        Pattern {
                            s: None,
                            p: Some(member.raw()),
                            o: Some(g.raw()),
                        },
                        view.scope(),
                    )? {
                        let r = r?;
                        if view.selects(&r) {
                            out.insert(r.s);
                        }
                    }
                }
            }
            Ok(Some(out))
        }
        Scope::Seeds { nodes, hops } => {
            let mut frontier = Vec::new();
            for n in nodes {
                if let Some(id) = tm_store::dict::encode(store, n)? {
                    frontier.push(id.raw());
                }
            }
            let mut seen: FxHashSet<Id> = frontier.iter().copied().collect();
            let mut out = FxHashSet::default();
            for _ in 0..*hops {
                let mut next = Vec::new();
                for n in frontier {
                    for pat in [
                        Pattern {
                            s: Some(n),
                            p: None,
                            o: None,
                        },
                        Pattern {
                            s: None,
                            p: None,
                            o: Some(n),
                        },
                    ] {
                        for r in store.scan(pat, view.scope())? {
                            let r = r?;
                            if !view.selects(&r) || !out.insert(r.eid) {
                                continue;
                            }
                            for m in [r.s, r.o, r.eid] {
                                if is_node(ObjectId::from_raw(m)) && seen.insert(m) {
                                    next.push(m);
                                }
                            }
                        }
                    }
                }
                frontier = next;
            }
            Ok(Some(out))
        }
    }
}

/// A prepared run: compiled program, engine, clock.
struct Prepared {
    prog: program::Program,
    t: u64,
    now: i64,
    program: Option<Value>,
    statement: Option<Eid>,
    window: Option<(i64, i64)>,
    valid_at: Option<i64>,
    demand: Option<Vec<String>>,
}

fn status(limits: &BTreeSet<Limit>) -> Status {
    if limits.is_empty() {
        Status::Complete
    } else {
        Status::Incomplete {
            limits: limits.iter().map(|l| l.name().to_string()).collect(),
        }
    }
}

// @tg: implements:: [[openspec:datalog-temporal#Atemporal mode]] {since: "2026-10-28"}
// @tg: implements:: [[openspec:datalog-temporal#Windows and recursion through time]] {since: "2026-10-28"}
fn prepare(
    store: &dyn ReadStore,
    spec: &ViewSpec,
    clock: i64,
    program: &Program,
    goal: Option<&str>,
    opts: &DatalogOptions,
) -> Result<Prepared> {
    let (t, now) = clock_of(store, spec, clock)?;
    let (mut text, pv, pe) = text_of(store, spec, program)?;
    if let Some(g) = goal {
        let g = g.trim();
        let g = g.strip_prefix("?-").unwrap_or(g).trim();
        let g = g.strip_suffix('.').unwrap_or(g);
        text.push_str("\n?- ");
        text.push_str(g);
        text.push_str(" .\n");
    }
    let prog = program::compile(&text)?;
    if opts.time == TimeMode::Atemporal {
        if let Some(op) = prog.temporal_operator() {
            return Err(Error::DatalogTemporalOperatorInAtemporal { operator: op });
        }
    }
    let window = opts
        .window
        .map(|w| (w.from.unwrap_or(NEG_INF), w.to.unwrap_or(POS_INF)));
    for s in &prog.strata {
        let name = || prog.rels[s.rels[0] as usize].name.to_string();
        if s.metric_recursion && window.is_none() && opts.time == TimeMode::Pointwise {
            return Err(Error::DatalogUnboundedRecursion { relation: name() });
        }
        if s.invention_recursion && opts.limits.max_new_values.is_none() {
            return Err(Error::DatalogUnboundedInvention { relation: name() });
        }
    }
    let valid_at = match spec.valid {
        ValidSel::At(t) => Some(t),
        ValidSel::Unfiltered => None,
    };
    Ok(Prepared {
        prog,
        t,
        now,
        program: pv,
        statement: pe,
        window,
        valid_at,
        demand: None,
    })
}

fn engine<'p, 's>(
    store: &'s dyn ReadStore,
    spec: &ViewSpec,
    prep: &'p Prepared,
    terms: &'s Terms,
    opts: &DatalogOptions,
    proof: bool,
    exclude: FxHashSet<Id>,
) -> Result<Engine<'p, 's>> {
    let view = View::resolve(store, spec)?;
    let scope = scope_eids(store, &view, &opts.scope)?;
    let time = match (opts.time, prep.valid_at) {
        (TimeMode::Atemporal, _) => TimeView::Atemporal,
        (_, Some(t)) => TimeView::At(t),
        _ => TimeView::Valid,
    };
    let mut stored = Stored::new(view, scope, exclude, time);
    match opts.access {
        AccessMode::Auto => {}
        AccessMode::Probe => stored.allow_load = false,
        AccessMode::Load => stored.force_load = true,
    }
    Engine::new(
        &prep.prog,
        store,
        stored,
        terms,
        EvalOptions {
            atemporal: opts.time == TimeMode::Atemporal,
            window: prep.window,
            limits: opts.limits.clone(),
            proof,
            parallel_threshold: opts.parallel_threshold,
            now: prep.now,
        },
    )
}

/// The final clip of every result: the window, and a `valid_at` instant.
fn clip(prep: &Prepared, iv: &IvSet) -> IvSet {
    let mut iv = iv.clone();
    if let Some((a, b)) = prep.window {
        iv = iv.clip(a, b);
    }
    if let Some(t) = prep.valid_at {
        iv = iv.clip(t, t.saturating_add(1));
    }
    iv
}

/// Resolves proofs into the result's `facts`.
struct Resolver<'a, 'p, 's> {
    eng: &'a Engine<'p, 's>,
    facts: Vec<Fact>,
    memo: FxHashMap<(u32, u32, usize), Premise>,
}

impl Resolver<'_, '_, '_> {
    fn term(&self, id: Id) -> Result<Term> {
        self.eng.sh.terms.term(self.eng.store(), id)
    }

    fn rule_index(&self, rule: u32) -> Option<usize> {
        match self.eng.sh.prog.rules[rule as usize].kind {
            RuleKind::Rule(i) => Some(i),
            _ => None,
        }
    }

    /// The derivation of the part of `row` that meets `frag` (the first one).
    fn portion(&self, rel: u32, row: u32, frag: &IvSet) -> Option<usize> {
        let p = &self.eng.sh.rels[rel as usize].prov[row as usize];
        p.iter()
            .position(|(part, _)| part.meets(frag))
            .or(if p.is_empty() { None } else { Some(0) })
    }

    fn proof(&mut self, d: u32, frag: &IvSet) -> Result<Proof> {
        let deriv = self.eng.derivs[d as usize].clone();
        let mut premises = Vec::new();
        for p in deriv.premises {
            match p {
                Prem::Stmt(e) => premises.push(Premise::Statement(stmt_eid(e)?)),
                Prem::Row(rel, row) => premises.push(self.fact(rel, row, frag)?),
            }
        }
        Ok(Proof {
            rule: self.rule_index(deriv.rule),
            premises,
        })
    }

    fn fact(&mut self, rel: u32, row: u32, frag: &IvSet) -> Result<Premise> {
        let Some(pi) = self.portion(rel, row, frag) else {
            return Ok(Premise::Fact(usize::MAX));
        };
        if let Some(p) = self.memo.get(&(rel, row, pi)) {
            return Ok(p.clone());
        }
        let r = &self.eng.sh.rels[rel as usize];
        let (part, d) = r.prov[row as usize][pi].clone();
        let deriv = &self.eng.derivs[d as usize];
        // a stored statement held by a derived IRI relation is that statement
        if self.eng.sh.prog.rules[deriv.rule as usize].kind == RuleKind::Seed {
            if let Some(Prem::Stmt(e)) = deriv.premises.first() {
                let p = Premise::Statement(stmt_eid(*e)?);
                self.memo.insert((rel, row, pi), p.clone());
                return Ok(p);
            }
        }
        let args = r
            .tuple(row)
            .iter()
            .map(|&x| self.term(x))
            .collect::<Result<Vec<_>>>()?;
        let ix = self.facts.len();
        self.facts.push(Fact {
            relation: rel_name(&self.eng.sh.prog.rels[rel as usize].name),
            args,
            valid: r.ivs[row as usize].valids(),
            proof: None,
        });
        self.memo.insert((rel, row, pi), Premise::Fact(ix));
        let proof = self.proof(d, &part.intersect(frag))?;
        self.facts[ix].proof = Some(proof);
        Ok(Premise::Fact(ix))
    }

    fn fragments(&mut self, rel: u32, row: u32, iv: &IvSet) -> Result<Vec<FragmentProof>> {
        let mut out = Vec::new();
        for &(a, b) in iv.intervals() {
            let frag = IvSet::one(a, b);
            let Some(pi) = self.portion(rel, row, &frag) else {
                continue;
            };
            let d = self.eng.sh.rels[rel as usize].prov[row as usize][pi].1;
            let proof = self.proof(d, &frag)?;
            out.push(FragmentProof {
                valid: frag.valids()[0],
                proof,
            });
        }
        Ok(out)
    }

    /// Every statement a proof tree rests on.
    fn eids(&self, proof: &Proof, out: &mut BTreeSet<Eid>) {
        for p in &proof.premises {
            match p {
                Premise::Statement(e) => {
                    out.insert(*e);
                }
                Premise::Fact(i) => {
                    if let Some(f) = self.facts.get(*i) {
                        if let Some(pr) = &f.proof {
                            self.eids(pr, out);
                        }
                    }
                }
            }
        }
    }
}

fn stmt_eid(e: Id) -> Result<Eid> {
    Eid::from_oid(ObjectId::from_raw(e))
        .ok_or_else(|| Error::storage("a statement row without a statement id"))
}

fn rel_name(p: &Pred) -> String {
    match p {
        Pred::Iri(i) => i.clone(),
        Pred::Name(n) => n.clone(),
    }
}

fn sort_key(store: &dyn ReadStore, terms: &Terms, tuple: &[Id]) -> Result<Vec<Value>> {
    tuple.iter().map(|&x| terms.value(store, x)).collect()
}

fn cmp_values(a: &[Value], b: &[Value]) -> std::cmp::Ordering {
    for (x, y) in a.iter().zip(b) {
        let o = terms::total_cmp(x, y);
        if o.is_ne() {
            return o;
        }
    }
    a.len().cmp(&b.len())
}

fn finish(store: &dyn ReadStore, eng: &Engine<'_, '_>, opts: &DatalogOptions) -> Result<Status> {
    let st = status(&eng.limits_hit);
    if opts.require_complete {
        if let Status::Incomplete { limits } = &st {
            return Err(Error::DatalogIncomplete {
                limits: limits.clone(),
            });
        }
    }
    let _ = store;
    Ok(st)
}

/// The rows of goal `gi` of a run.
fn goal_rows(
    eng: &Engine<'_, '_>,
    prep: &Prepared,
    rel: u32,
    proof: bool,
    res: &mut Resolver<'_, '_, '_>,
) -> Result<Vec<Row>> {
    let store = eng.store();
    let r = &eng.sh.rels[rel as usize];
    let mut rows: Vec<(Vec<Value>, i64, Row)> = Vec::new();
    for row in 0..r.len() as u32 {
        let iv = clip(prep, &r.ivs[row as usize]);
        if iv.is_empty() {
            continue;
        }
        let tuple = r.tuple(row);
        let values = tuple
            .iter()
            .map(|&x| eng.sh.terms.term(store, x))
            .collect::<Result<Vec<_>>>()?;
        let pf = if proof {
            Some(res.fragments(rel, row, &iv)?)
        } else {
            None
        };
        rows.push((
            sort_key(store, eng.sh.terms, tuple)?,
            iv.intervals()[0].0,
            Row {
                values,
                valid: iv.valids(),
                proof: pf,
            },
        ));
    }
    rows.sort_by(|a, b| cmp_values(&a.0, &b.0).then(a.1.cmp(&b.1)));
    Ok(rows.into_iter().map(|(_, _, r)| r).collect())
}

fn run_goal(
    store: &dyn ReadStore,
    spec: &ViewSpec,
    clock: i64,
    program: &Program,
    goal: Option<&str>,
    opts: &DatalogOptions,
    explain: bool,
) -> Result<(DatalogResult, Vec<String>)> {
    let mut prep = prepare(store, spec, clock, program, goal, opts)?;
    let gi = match goal {
        Some(_) => prep.prog.goals.len().checked_sub(1),
        None => (!prep.prog.goals.is_empty()).then_some(0),
    }
    .ok_or_else(|| Error::DatalogParse {
        line: 0,
        column: 0,
        message: "the program has no goal".into(),
    })?;
    let columns = prep.prog.goals[gi].columns.clone();
    let mut goal_rel = prep.prog.goals[gi].rel;
    if opts.demand {
        if let Some((q, notes)) = crate::magic::rewrite(&prep.prog, gi) {
            prep.prog = q;
            prep.demand = Some(notes);
            goal_rel = prep.prog.goals[gi].rel;
        }
    }
    let terms = Terms::new();
    let mut eng = engine(
        store,
        spec,
        &prep,
        &terms,
        opts,
        opts.proof,
        FxHashSet::default(),
    )?;
    eng.run()?;
    let status = finish(store, &eng, opts)?;
    let mut res = Resolver {
        eng: &eng,
        facts: Vec::new(),
        memo: FxHashMap::default(),
    };
    let rows = goal_rows(&eng, &prep, goal_rel, opts.proof, &mut res)?;
    let facts = std::mem::take(&mut res.facts);
    let mut plan = Vec::new();
    if explain {
        plan.push(match &prep.demand {
            Some(n) if n.is_empty() => "demand: applied".to_string(),
            Some(n) => format!("demand: applied; {}", n.join("; ")),
            None => "demand: not applied".to_string(),
        });
        plan.extend(eng.plan.iter().cloned());
        for a in eng.sh.stored.accesses() {
            let name = match a.p {
                Some(p) => eng.sh.terms.value(store, p)?.to_string(),
                None => "every predicate".into(),
            };
            plan.push(format!(
                "access {name}: {} ({} probes, {} rows loaded)",
                if a.loaded { "load" } else { "probe" },
                a.probes,
                a.rows
            ));
        }
        plan.push(format!("iterations: {}", eng.iterations));
    }
    Ok((
        DatalogResult {
            columns,
            rows,
            facts,
            t: prep.t,
            now: prep.now,
            strata: prep.prog.strata.len(),
            derived: eng.derived(),
            predicates: prep.prog.predicates.clone(),
            pruned: !prep.prog.variable_predicate,
            demand: prep.demand.clone(),
            status,
            program: prep.program.clone(),
            program_statement: prep.statement,
        },
        plan,
    ))
}

/// Evaluates `program` over the view `spec` of `store` and returns the rows of
/// `goal` (or of the text's first goal). `clock` is the instant a now view pins.
// @tg: implements:: [[openspec:datalog-evaluation#Goal queries over a view]] {since: "2026-10-28"}
// @tg: implements:: [[openspec:datalog-evaluation#Magic sets preserve answers]] {since: "2026-10-28"}
// @tg: implements:: [[openspec:datalog-evaluation#Deterministic results]] {since: "2026-10-28"}
pub fn query(
    store: &dyn ReadStore,
    spec: &ViewSpec,
    clock: i64,
    program: &Program,
    goal: Option<&str>,
    opts: &DatalogOptions,
) -> Result<DatalogResult> {
    Ok(run_goal(store, spec, clock, program, goal, opts, false)?.0)
}

/// [`query`] with its plan.
// @tg: implements:: [[openspec:datalog-evaluation#Explain a program run]] {since: "2026-10-28"}
pub fn explain(
    store: &dyn ReadStore,
    spec: &ViewSpec,
    clock: i64,
    program: &Program,
    goal: Option<&str>,
    opts: &DatalogOptions,
) -> Result<Explain> {
    let (result, plan) = run_goal(store, spec, clock, program, goal, opts, true)?;
    Ok(Explain { plan, result })
}

/// Every conclusion of `program`'s IRI relations not among the stored
/// statements, one per maximal interval, sorted.
// @tg: implements:: [[openspec:datalog-evaluation#Full reasoning over a view]] {since: "2026-10-28"}
pub fn reason(
    store: &dyn ReadStore,
    spec: &ViewSpec,
    clock: i64,
    program: &Program,
    opts: &DatalogOptions,
) -> Result<ReasonResult> {
    reason_excluding(store, spec, clock, program, opts, FxHashSet::default())
}

/// [`reason`] without the stored statements `exclude` (a materialization's
/// own conclusions).
// @tg: implements:: [[openspec:datalog-evaluation#Agreement with N3 on Horn programs]] {since: "2026-10-28"}
pub fn reason_excluding(
    store: &dyn ReadStore,
    spec: &ViewSpec,
    clock: i64,
    program: &Program,
    opts: &DatalogOptions,
    exclude: FxHashSet<Id>,
) -> Result<ReasonResult> {
    let prep = prepare(store, spec, clock, program, None, opts)?;
    let terms = Terms::new();
    let mut eng = engine(store, spec, &prep, &terms, opts, opts.proof, exclude)?;
    eng.run()?;
    let status = finish(store, &eng, opts)?;
    let mut res = Resolver {
        eng: &eng,
        facts: Vec::new(),
        memo: FxHashMap::default(),
    };
    let mut conclusions: Vec<(Vec<Value>, i64, Conclusion)> = Vec::new();
    let mut relations: Vec<(Vec<Value>, Fact)> = Vec::new();
    for (ri, rel) in prep.prog.rels.iter().enumerate() {
        if rel.hidden {
            continue;
        }
        let r = &eng.sh.rels[ri];
        for row in 0..r.len() as u32 {
            let tuple = r.tuple(row);
            match &rel.name {
                Pred::Iri(p) => {
                    let iv = clip(
                        &prep,
                        &r.ivs[row as usize].subtract(&r.stored[row as usize]),
                    );
                    for &(a, b) in iv.intervals() {
                        let frag = IvSet::one(a, b);
                        let proof = if opts.proof {
                            res.fragments(ri as u32, row, &frag)?
                                .into_iter()
                                .next()
                                .map(|f| f.proof)
                        } else {
                            None
                        };
                        let key = sort_key(store, &terms, tuple)?;
                        conclusions.push((
                            key,
                            a,
                            Conclusion {
                                s: terms.term(store, tuple[0])?,
                                p: Value::iri(p.as_str()),
                                o: terms.term(store, tuple[1])?,
                                valid: frag.valids()[0],
                                proof,
                            },
                        ));
                    }
                }
                Pred::Name(n) if opts.include_relations => {
                    let iv = clip(&prep, &r.ivs[row as usize]);
                    if iv.is_empty() {
                        continue;
                    }
                    let args = tuple
                        .iter()
                        .map(|&x| terms.term(store, x))
                        .collect::<Result<Vec<_>>>()?;
                    let mut key = vec![Value::str(n.as_str())];
                    key.extend(sort_key(store, &terms, tuple)?);
                    relations.push((
                        key,
                        Fact {
                            relation: n.clone(),
                            args,
                            valid: iv.valids(),
                            proof: None,
                        },
                    ));
                }
                Pred::Name(_) => {}
            }
        }
    }
    conclusions.sort_by(|a, b| {
        let pa = format!("{}", a.2.p);
        let pb = format!("{}", b.2.p);
        pa.cmp(&pb).then(cmp_values(&a.0, &b.0)).then(a.1.cmp(&b.1))
    });
    relations.sort_by(|a, b| cmp_values(&a.0, &b.0));
    let facts = std::mem::take(&mut res.facts);
    Ok(ReasonResult {
        conclusions: conclusions.into_iter().map(|c| c.2).collect(),
        relations: relations.into_iter().map(|r| r.1).collect(),
        facts,
        t: prep.t,
        now: prep.now,
        predicates: prep.prog.predicates.clone(),
        status,
        program: prep.program.clone(),
        program_statement: prep.statement,
    })
}

/// Evaluates the program's constraints (those named in `only`, or all) and
/// returns their violations.
// @tg: implements:: [[openspec:datalog-constraints#Check constraints]] {since: "2026-10-28"}
pub fn check(
    store: &dyn ReadStore,
    spec: &ViewSpec,
    clock: i64,
    program: &Program,
    only: &[String],
    opts: &DatalogOptions,
) -> Result<CheckResult> {
    let prep = prepare(store, spec, clock, program, None, opts)?;
    for n in only {
        if !prep.prog.constraints.iter().any(|c| &c.name == n) {
            return Err(Error::DatalogUnsafe {
                rule: 0,
                message: format!("no constraint named {n}"),
            });
        }
    }
    let terms = Terms::new();
    let mut eng = engine(store, spec, &prep, &terms, opts, true, FxHashSet::default())?;
    eng.run()?;
    let status = finish(store, &eng, opts)?;
    let mut res = Resolver {
        eng: &eng,
        facts: Vec::new(),
        memo: FxHashMap::default(),
    };
    let mut violated = Vec::new();
    let mut checked = Vec::new();
    for c in &prep.prog.constraints {
        if !only.is_empty() && !only.contains(&c.name) {
            continue;
        }
        checked.push(c.name.clone());
        let rows = goal_rows(&eng, &prep, c.rel, true, &mut res)?;
        if rows.is_empty() {
            continue;
        }
        let mut violations = Vec::new();
        for row in rows {
            let mut eids = BTreeSet::new();
            for f in row.proof.iter().flatten() {
                res.eids(&f.proof, &mut eids);
            }
            violations.push(Violation {
                bindings: c.columns.iter().cloned().zip(row.values).collect(),
                eids: eids.into_iter().collect(),
                valid: row.valid,
            });
        }
        violated.push(ConstraintReport {
            name: c.name.clone(),
            severity: c.severity,
            violations,
        });
    }
    Ok(CheckResult {
        violated,
        checked,
        t: prep.t,
        now: prep.now,
        status,
    })
}

/// What `program` is in the view.
// @tg: implements:: [[openspec:datalog-programs#Program information]] {since: "2026-10-28"}
pub fn info(store: &dyn ReadStore, spec: &ViewSpec, program: &Program) -> Result<ProgramInfo> {
    let (text, pv, pe) = text_of(store, spec, program)?;
    let prog = program::compile(&text)?;
    Ok(ProgramInfo {
        program: pv,
        statement: pe,
        text,
        analysis: prog.analysis(),
    })
}

/// Parses and analyses program text (`store_program` refuses what fails).
pub fn validate(text: &str) -> Result<Analysis> {
    Ok(program::compile(text)?.analysis())
}
