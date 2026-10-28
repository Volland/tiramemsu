//! N3 reasoning over a view (`View::reason`, `View::rule_set_info`) and stored
//! rule sets (`TxN3::store_rules`), with the reasoner of `tm-n3` (OpenSpec change
//! `add-n3-reasoning`, facade feature `n3`).
//!
//! This module loads: the scope of the view, pruned to the predicates the rules
//! read, in one read snapshot, and the view's clock. `tm-n3` reasons and never
//! touches the store.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use tm_core::sqlstore::SqlStore;
use tm_core::store::ReadStore;
use tm_core::vocab::SYS_DERIVED_FROM;
use tm_core::{
    budget, read, Eid, Error, Executor, ObjectId, Patch, Result, SqlValue, Tag, TermReader,
    TimeRef, Triple, TxHandle as Tx, TxSel, Valid, ValidSel, Value, ViewSpec,
};
use tm_n3::vocab::{N3_DERIVED_BY, N3_RULE, N3_SOURCE};
use tm_n3::{
    Fact, Input, Monotonicity, ReasonOptions, ReasonResult, RuleSet, RuleSetInfo, Rules, Scope,
};
use tm_store::dict::TermCache;

use crate::view::View;

/// The reads a load needs, on either engine.
trait Src {
    fn triples(
        &mut self,
        s: Option<ObjectId>,
        p: Option<ObjectId>,
        o: Option<ObjectId>,
    ) -> Result<Vec<Triple>>;
    fn members(&mut self, graph: ObjectId) -> Result<Vec<Eid>>;
    fn encode(&mut self, v: &Value) -> Result<Option<ObjectId>>;
    fn decode(&mut self, id: ObjectId) -> Result<Value>;
    fn tx_instant(&mut self, t: u64) -> Result<Option<i64>>;
    fn last_t(&mut self) -> Result<u64>;
    fn tx_at_or_before(&mut self, ms: i64) -> Result<u64>;
    /// Statement `eid`, if the view selects it.
    fn statement(&mut self, eid: Eid) -> Result<Option<Triple>>;
}

/// Statement `eid` of `store`, if the view `spec` selects it.
fn selected(store: &dyn ReadStore, spec: &ViewSpec, eid: Eid) -> Result<Option<Triple>> {
    let view = tm_store::read::View::resolve(store, spec)?;
    Ok(store
        .statement(eid.oid().raw())?
        .filter(|r| view.selects(r))
        .map(|r| tm_store::read::to_triple(&r, false)))
}

struct Sql<'e, 'r> {
    e: &'e mut dyn Executor,
    reader: Option<&'r TermReader>,
    fallback: TermReader,
    spec: ViewSpec,
}

impl Src for Sql<'_, '_> {
    fn triples(
        &mut self,
        s: Option<ObjectId>,
        p: Option<ObjectId>,
        o: Option<ObjectId>,
    ) -> Result<Vec<Triple>> {
        read::triples(self.e, &self.spec, s, p, o)
    }
    fn members(&mut self, graph: ObjectId) -> Result<Vec<Eid>> {
        read::graph_members(self.e, &self.spec, graph)
    }
    fn encode(&mut self, v: &Value) -> Result<Option<ObjectId>> {
        TermReader::encode(self.e, v)
    }
    fn decode(&mut self, id: ObjectId) -> Result<Value> {
        match self.reader {
            Some(r) => r.decode(self.e, id, true),
            None => self.fallback.decode(self.e, id, false),
        }
    }
    fn tx_instant(&mut self, t: u64) -> Result<Option<i64>> {
        read::tx_instant(self.e, t)
    }
    fn last_t(&mut self) -> Result<u64> {
        read::last_t(self.e)
    }
    fn tx_at_or_before(&mut self, ms: i64) -> Result<u64> {
        Ok(self
            .e
            .query_i64(
                "SELECT coalesce(max(t), 0) FROM tx WHERE instant <= ?1",
                &[SqlValue::Integer(ms)],
            )?
            .unwrap_or(0)
            .max(0) as u64)
    }
    fn statement(&mut self, eid: Eid) -> Result<Option<Triple>> {
        selected(&SqlStore::new(self.e), &self.spec, eid)
    }
}

struct Kv<'s> {
    r: &'s dyn ReadStore,
    terms: &'s TermCache,
    cache: bool,
    spec: ViewSpec,
}

impl Src for Kv<'_> {
    fn triples(
        &mut self,
        s: Option<ObjectId>,
        p: Option<ObjectId>,
        o: Option<ObjectId>,
    ) -> Result<Vec<Triple>> {
        tm_store::read::triples(self.r, &self.spec, s, p, o)
    }
    fn members(&mut self, graph: ObjectId) -> Result<Vec<Eid>> {
        tm_store::read::graph_members(self.r, &self.spec, graph)
    }
    fn encode(&mut self, v: &Value) -> Result<Option<ObjectId>> {
        tm_store::dict::encode(self.r, v)
    }
    fn decode(&mut self, id: ObjectId) -> Result<Value> {
        self.terms.decode(self.r, id, self.cache)
    }
    fn tx_instant(&mut self, t: u64) -> Result<Option<i64>> {
        tm_store::read::tx_instant(self.r, t)
    }
    fn last_t(&mut self) -> Result<u64> {
        tm_store::read::last_t(self.r)
    }
    fn tx_at_or_before(&mut self, ms: i64) -> Result<u64> {
        Ok(self.r.tx_at_or_before(ms)?.max(0) as u64)
    }
    fn statement(&mut self, eid: Eid) -> Result<Option<Triple>> {
        selected(self.r, &self.spec, eid)
    }
}

/// The live `n3:source` statement of `rule_set` in the view, and its text.
fn stored_source(src: &mut dyn Src, rule_set: &Value) -> Result<Option<(String, Eid)>> {
    let (Some(s), Some(p)) = (src.encode(rule_set)?, src.encode(&Value::iri(N3_SOURCE))?) else {
        return Ok(None);
    };
    // the newest version wins in a history view
    let Some(row) = src.triples(Some(s), Some(p), None)?.pop() else {
        return Ok(None);
    };
    match src.decode(row.o)? {
        Value::Str(text) => Ok(Some((text, row.eid))),
        _ => Ok(None),
    }
}

fn unknown(rule_set: &Value) -> Error {
    Error::UnknownRuleSet {
        rule_set: rule_set.to_string(),
    }
}

/// What a run reasons over, read in one snapshot.
struct Load {
    rules: RuleSet,
    facts: Vec<Fact>,
    now: i64,
    t: u64,
    rule_set: Option<Value>,
    rule_set_statement: Option<Eid>,
    pruned: bool,
}

/// An id whose statements a seed expansion follows (not a literal).
fn is_node(id: ObjectId) -> bool {
    matches!(id.tag(), Ok(Tag::Iri | Tag::Node | Tag::BNode | Tag::Stmt))
}

// @tg: implements:: [[openspec:n3-reasoning#Reasoning scope and pruning]] {since: "2026-10-28"}
// @tg: implements:: [[openspec:n3-reasoning#Transaction time and the pinned clock]] {since: "2026-10-28"}
fn load(
    src: &mut dyn Src,
    spec: ViewSpec,
    rules: &Rules,
    scope: &Scope,
    clock: i64,
    exclude_own: bool,
) -> Result<Load> {
    // the view's transaction and its instant: the pinned clock (design N7)
    let (t, now) = match spec.tx {
        TxSel::Now | TxSel::History => (src.last_t()?, clock),
        TxSel::AsOf(TimeRef::Tx(t)) => {
            let now = match src.tx_instant(t)? {
                Some(i) => i,
                // past the last transaction the view is the now view
                None if t > src.last_t()? => clock,
                None => 0,
            };
            (t, now)
        }
        TxSel::AsOf(TimeRef::Instant(ms)) => (src.tx_at_or_before(ms)?, ms),
    };
    let (text, rule_set, rule_set_statement) = match rules {
        Rules::Text(text) => (text.clone(), None, None),
        Rules::Stored(rs) => {
            let (text, eid) = stored_source(src, rs)?.ok_or_else(|| unknown(rs))?;
            (text, Some(rs.clone()), Some(eid))
        }
    };
    let rules = RuleSet::parse(&text)?;

    // pruning: only the predicates the bodies read (design N4)
    let preds: Option<Vec<ObjectId>> = match rules.load_predicates() {
        Some(ps) => {
            let mut ids = Vec::new();
            for p in ps {
                if let Some(id) = src.encode(&Value::iri(p.as_str()))? {
                    ids.push(id);
                }
            }
            Some(ids)
        }
        None => None,
    };
    let pruned = preds.is_some();
    let wanted = |row: &Triple| preds.as_ref().is_none_or(|ps| ps.contains(&row.p));
    let all = |src: &mut dyn Src| -> Result<Vec<Triple>> {
        match &preds {
            None => src.triples(None, None, None),
            Some(ps) => {
                let mut rows = Vec::new();
                for &p in ps {
                    rows.extend(src.triples(None, Some(p), None)?);
                }
                Ok(rows)
            }
        }
    };
    let mut rows: Vec<Triple> = match scope {
        Scope::All => all(src)?,
        Scope::Graphs(graphs) => {
            let mut members = BTreeSet::new();
            for g in graphs {
                if let Some(g) = src.encode(g)? {
                    members.extend(src.members(g)?);
                }
            }
            if members.is_empty() {
                Vec::new()
            } else {
                all(src)?
                    .into_iter()
                    .filter(|r| members.contains(&r.eid))
                    .collect()
            }
        }
        Scope::Seeds { nodes, hops } => {
            let mut frontier = Vec::new();
            for n in nodes {
                if let Some(id) = src.encode(n)? {
                    frontier.push(id);
                }
            }
            let mut seen: BTreeSet<ObjectId> = frontier.iter().copied().collect();
            let mut found: BTreeMap<Eid, Triple> = BTreeMap::new();
            for _ in 0..*hops {
                let mut next = Vec::new();
                for n in frontier {
                    let mut near = src.triples(Some(n), None, None)?;
                    near.extend(src.triples(None, None, Some(n))?);
                    for row in near {
                        if found.insert(row.eid, row).is_none() {
                            for m in [row.s, row.o, row.eid.oid()] {
                                if is_node(m) && seen.insert(m) {
                                    next.push(m);
                                }
                            }
                        }
                    }
                }
                frontier = next;
            }
            found.into_values().filter(|r| wanted(r)).collect()
        }
    };
    // rules reading `tm:content`, `tm:validFrom` or `tm:validTo` also see the
    // statements the loaded ones are about (the targets of layers), transitively
    if rules.reads_statements() {
        let mut have: BTreeSet<Eid> = rows.iter().map(|r| r.eid).collect();
        let mut todo: Vec<Eid> = rows
            .iter()
            .flat_map(|r| [r.s, r.o])
            .filter_map(Eid::from_oid)
            .collect();
        while let Some(e) = todo.pop() {
            if !have.insert(e) {
                continue;
            }
            if let Some(row) = src.statement(e)? {
                todo.extend([row.s, row.o].into_iter().filter_map(Eid::from_oid));
                rows.push(row);
            }
        }
    }
    rows.sort_by_key(|r| r.eid);
    rows.dedup_by_key(|r| r.eid);
    // materialization recomputes its own conclusions (design N10): the
    // statements derived from the rule set's live statement, and their support
    // links and `n3:` labels, are not input
    if let (true, Some(r)) = (exclude_own, rule_set_statement) {
        let own = own_conclusions(src, r)?;
        if !own.is_empty() {
            let mut labels = Vec::new();
            for p in [SYS_DERIVED_FROM, N3_DERIVED_BY, N3_RULE] {
                labels.extend(src.encode(&Value::iri(p))?);
            }
            let about_own = |id: ObjectId| Eid::from_oid(id).is_some_and(|e| own.contains(&e));
            rows.retain(|r| {
                let label = about_own(r.s) && labels.contains(&r.p);
                !own.contains(&r.eid) && !label
            });
        }
    }

    let mut values: HashMap<ObjectId, Value> = HashMap::new();
    let mut value = |src: &mut dyn Src, id: ObjectId| -> Result<Value> {
        if let Some(v) = values.get(&id) {
            return Ok(v.clone());
        }
        let v = src.decode(id)?;
        values.insert(id, v.clone());
        Ok(v)
    };
    let mut facts = Vec::with_capacity(rows.len());
    for r in rows {
        facts.push(Fact {
            eid: r.eid,
            s: value(src, r.s)?,
            p: value(src, r.p)?,
            o: value(src, r.o)?,
            valid: Valid {
                from: r.v_from,
                to: r.v_to,
            },
        });
    }
    Ok(Load {
        rules,
        facts,
        now,
        t,
        rule_set,
        rule_set_statement,
        pruned,
    })
}

/// The live statements derived from rule-set statement `r`: subjects of live
/// links `(D sys:derivedFrom r)`.
fn own_conclusions(src: &mut dyn Src, r: Eid) -> Result<BTreeSet<Eid>> {
    let Some(df) = src.encode(&Value::iri(SYS_DERIVED_FROM))? else {
        return Ok(BTreeSet::new());
    };
    Ok(src
        .triples(None, Some(df), Some(r.oid()))?
        .into_iter()
        .filter_map(|row| Eid::from_oid(row.s))
        .collect())
}

fn system_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

impl View<'_> {
    /// Runs N3 rules over the statements of this view and returns what follows,
    /// with proofs; writes nothing (feature `n3`).
    ///
    /// The run loads the scope of `opts` (the whole view, named graphs or seed
    /// nodes), pruned to the predicates the rule bodies read, in one read
    /// snapshot. A conclusion holds at `t` iff it follows from the statements
    /// valid at `t`: a `valid_at(t)` view reasons once and reports `[t, t+1)`;
    /// otherwise the run sweeps the segments of `opts.window` cut at every
    /// valid-time bound and merges adjacent segments with the same conclusion and
    /// support. The clock (`time:localTime`, `tm:run tm:now`) is the view's
    /// instant: the as-of instant, or the database clock read once at the start
    /// on a now view.
    ///
    /// Rule text may use the prefixes `v:` (`urn:tiramemsu:v:`), `tm:` and `n3:`
    /// without declaring them. A statement is `<urn:tiramemsu:stmt:N>`, with the
    /// virtual triples `tm:content { s p o }`, `tm:validFrom` and `tm:validTo`.
    /// Rules that read them also load the statements the loaded ones are about
    /// (the targets of layers), so a variable predicate inside `tm:content`
    /// keeps pruning on.
    ///
    /// # Errors
    ///
    /// `N3Parse { line, message }` for rule text that does not parse,
    /// `UnknownRuleSet` for a stored rule set with no live `n3:source` here, and
    /// the budget and read errors.
    ///
    /// ```
    /// # use tiramemsu::*;
    /// # let dir = tempfile::tempdir().unwrap();
    /// # let db = Db::open(dir.path().join("m.db"), OpenOptions::default())?;
    /// let v = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
    /// let r = db.transact(TxOptions::default(), |tx| {
    ///     tx.assert(v("socrates"), Value::iri(vocab::RDF_TYPE), v("Human"), Valid::ALWAYS)?;
    ///     Ok(())
    /// })?;
    /// let rules = Rules::Text("{ ?x a v:Human } => { ?x a v:Mortal } .".into());
    /// let result = db.now().reason(rules, &ReasonOptions::default())?;
    /// assert_eq!(result.conclusions.len(), 1);
    /// assert_eq!(result.conclusions[0].o, N3Term::Value(v("Mortal")));
    /// assert_eq!(result.conclusions[0].proof.premises, [Premise::Statement(r.asserted[0])]);
    /// # Ok::<(), Error>(())
    /// ```
    // @tg: implements:: [[openspec:n3-reasoning#Reason over a view]] {since: "2026-10-28"}
    pub fn reason(&self, rules: Rules, opts: &ReasonOptions) -> Result<ReasonResult> {
        self.run_rules(&rules, opts, false)
    }

    /// The reasoning of `Db::materialize`: stored rule set `rule_set`, its own
    /// conclusions excluded from the input, refused before reasoning when it is
    /// not monotonic (design N10, N11).
    // @tg: implements:: [[openspec:n3-materialization#Own conclusions are not input]] {since: "2026-10-28"}
    // @tg: implements:: [[openspec:n3-materialization#Only monotonic rule sets materialize]] {since: "2026-10-28"}
    pub(crate) fn reason_for_materialization(
        &self,
        rule_set: &Value,
        opts: &ReasonOptions,
    ) -> Result<ReasonResult> {
        self.run_rules(&Rules::Stored(rule_set.clone()), opts, true)
    }

    fn run_rules(
        &self,
        rules: &Rules,
        opts: &ReasonOptions,
        materialize: bool,
    ) -> Result<ReasonResult> {
        let spec = self.spec();
        let clock = self.db().map_or_else(system_now, |db| db.now_ms());
        self.op(|| {
            let l = self.either(
                |e, reader| {
                    let mut src = Sql {
                        e,
                        reader,
                        fallback: TermReader::new(64),
                        spec,
                    };
                    load(&mut src, spec, rules, &opts.scope, clock, materialize)
                },
                |r, terms, cache| {
                    let mut src = Kv {
                        r,
                        terms,
                        cache,
                        spec,
                    };
                    load(&mut src, spec, rules, &opts.scope, clock, materialize)
                },
            )?;
            if materialize {
                match &l.rules.info().monotonicity {
                    Monotonicity::Monotonic => {}
                    Monotonicity::NonMonotonic { rule, builtin }
                    | Monotonicity::ClockDependent { rule, builtin }
                    | Monotonicity::UntracedPremises { rule, builtin } => {
                        return Err(Error::N3NotMaterializable {
                            rule: *rule,
                            builtin: builtin.clone(),
                        })
                    }
                }
            }
            if budget::active() {
                budget::charge_rows(l.facts.len() as u64)?;
                budget::charge_bytes(l.facts.len() as u64 * 56)?;
            }
            let snapshot = match spec.valid {
                ValidSel::At(t) => Some(t),
                ValidSel::Unfiltered => None,
            };
            let input = Input {
                facts: &l.facts,
                now: l.now,
                snapshot,
                rule_set: l.rule_set.clone(),
            };
            let mut r = tm_n3::reason(&l.rules, &input, opts)?;
            r.t = l.t;
            r.pruned = l.pruned;
            r.rule_set_statement = l.rule_set_statement;
            Ok(r)
        })
    }

    /// The stored rule set `rule_set` as this view sees it: its monotonicity
    /// classification, rule count and the predicates its bodies read
    /// (feature `n3`).
    ///
    /// # Errors
    ///
    /// `UnknownRuleSet` when it has no live `n3:source` statement here.
    // @tg: implements:: [[openspec:n3-rule-sets#Monotonicity check]] {since: "2026-10-28"}
    pub fn rule_set_info(&self, rule_set: &Value) -> Result<RuleSetInfo> {
        let spec = self.spec();
        self.op(|| {
            let text = self.either(
                |e, reader| {
                    let mut src = Sql {
                        e,
                        reader,
                        fallback: TermReader::new(4),
                        spec,
                    };
                    stored_source(&mut src, rule_set)
                },
                |r, terms, cache| {
                    let mut src = Kv {
                        r,
                        terms,
                        cache,
                        spec,
                    };
                    stored_source(&mut src, rule_set)
                },
            )?;
            let (text, _) = text.ok_or_else(|| unknown(rule_set))?;
            Ok(RuleSet::parse(&text)?.info().clone())
        })
    }
}

/// Rule sets inside a transaction (feature `n3`).
pub trait TxN3 {
    /// Stores N3 rule text as rule set `rule_set`: the one live statement
    /// `(rule_set n3:source "<n3>")`. A live source with other text is superseded,
    /// so `as_of` views reason with the rules believed then; equal text returns
    /// the live statement and writes nothing. Returns the live statement's eid.
    ///
    /// # Errors
    ///
    /// `N3Parse { line, message }` when the text does not parse (nothing is
    /// written), and the write errors.
    ///
    /// ```
    /// # use tiramemsu::*;
    /// # let dir = tempfile::tempdir().unwrap();
    /// # let db = Db::open(dir.path().join("m.db"), OpenOptions::default())?;
    /// let v = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
    /// let text = "{ ?x a v:Human } => { ?x a v:Mortal } .";
    /// let mut first = None;
    /// db.transact(TxOptions::default(), |tx| {
    ///     first = Some(tx.store_rules(v("mortality"), text)?);
    ///     Ok(())
    /// })?;
    /// let info = db.now().rule_set_info(&v("mortality"))?;
    /// assert_eq!(info.monotonicity, Monotonicity::Monotonic);
    /// # Ok::<(), Error>(())
    /// ```
    fn store_rules(&mut self, rule_set: Value, n3: &str) -> Result<Eid>;
}

impl TxN3 for Tx<'_> {
    // @tg: implements:: [[openspec:n3-rule-sets#Store a rule set]] {since: "2026-10-28"}
    // @tg: implements:: [[openspec:n3-rule-sets#Rule sets have transaction time]] {since: "2026-10-28"}
    // @tg: implements:: [[openspec:datalog-programs#Rule language conflict]] {since: "2026-10-28"}
    fn store_rules(&mut self, rule_set: Value, n3: &str) -> Result<Eid> {
        RuleSet::parse(n3)?;
        // a node holds rules of one language (a live Datalog program refuses N3)
        if let (Some(s), Some(p)) = (
            self.lookup(&rule_set)?,
            self.lookup(&Value::iri(tm_core::vocab::DL_SOURCE))?,
        ) {
            if !self.live_rows(Some(s), Some(p), None)?.is_empty() {
                return Err(Error::RuleLanguageConflict {
                    node: rule_set.to_string(),
                    existing: "Datalog".into(),
                });
            }
        }
        let source = Value::iri(N3_SOURCE);
        let text = Value::str(n3);
        if let (Some(s), Some(p)) = (self.lookup(&rule_set)?, self.lookup(&source)?) {
            if let Some(row) = self.live_rows(Some(s), Some(p), None)?.pop() {
                let eid = Eid::from_oid(ObjectId::from_raw(row.eid))
                    .ok_or_else(|| Error::storage("a statement row without a statement id"))?;
                if self.decode(ObjectId::from_raw(row.o))? == text {
                    return Ok(eid);
                }
                return self.supersede(eid, Patch::object(text));
            }
        }
        Ok(self.assert(rule_set, source, text, Valid::ALWAYS)?.eid())
    }
}
