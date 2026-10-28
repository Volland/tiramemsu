//! Materialization of a stored rule set (`Db::materialize`, OpenSpec change
//! `add-n3-reasoning`, facade feature `n3`): reason on a snapshot outside the
//! writer, then write the conclusions in one short transaction through
//! `derive`, with the premises and the rule-set statement as support.

use std::collections::{BTreeSet, HashMap};

use tm_core::engine::reserved;
use tm_core::vocab::SYS_DERIVED_FROM;
use tm_core::{
    budget, Asserted, Eid, Error, Obj, ObjectId, Patch, Result, Tag, TxHandle as Tx, TxOptions,
    TxReport, Valid, Value,
};
use tm_n3::vocab::{N3_DERIVED_BY, N3_RULE, N3_SKOLEM};
use tm_n3::{
    Conclusion, Limits, Premise, ReasonOptions, ReasonResult, Scope, Status, Term, TimeMode,
};

use crate::db::Db;

/// The options of [`Db::materialize`] (feature `n3`).
#[derive(Debug, Clone)]
pub struct MaterializeOptions {
    /// The statements loaded (default: the whole now view).
    pub scope: Scope,
    /// The valid-time window of the sweep (default: unbounded).
    pub window: Valid,
    /// The most segments the sweep runs (default 1 024).
    pub max_segments: usize,
    /// eyeron's limits for each segment.
    pub limits: Limits,
    /// Refuse to write the conclusions of an incomplete run (`N3Incomplete`).
    pub require_complete: bool,
    /// Run the commit with full semantics, report, then discard it.
    pub dry_run: bool,
}

impl Default for MaterializeOptions {
    fn default() -> MaterializeOptions {
        MaterializeOptions {
            scope: Scope::All,
            window: Valid::ALWAYS,
            max_segments: 1024,
            limits: Limits::default(),
            require_complete: false,
            dry_run: false,
        }
    }
}

/// What [`Db::materialize`] did (feature `n3`).
#[derive(Debug, Clone)]
pub struct MaterializeReport {
    /// The rule set.
    pub rule_set: Value,
    /// Its live `n3:source` statement `R` at the snapshot: a support of every
    /// conclusion written.
    pub rules_statement: Eid,
    /// The transaction number of the snapshot reasoned over (`t₀`).
    pub reasoned_at: u64,
    /// The report of the commit (of the discarded commit with `dry_run`).
    pub commit: Option<TxReport>,
    /// Conclusions written as new derived statements (not their links, labels or
    /// Skolem keys).
    pub derived: Vec<Eid>,
    /// Conclusions already live with the same content and support and an
    /// interval containing the new one.
    pub existing: Vec<Eid>,
    /// Conclusions widened to the hull of the old and the new interval, as
    /// `(old, new)`.
    pub widened: Vec<(Eid, Eid)>,
    /// The nodes invented for blank nodes of conclusions (`upsert_derived`).
    pub skolems: Vec<ObjectId>,
    /// Conclusions skipped because a support was no longer live at commit time,
    /// with every conclusion depending on one.
    pub stale_support: Vec<Conclusion>,
    /// Conclusions skipped because the store cannot hold them (a list or a
    /// formula term, a predicate that is not a writable IRI, a literal subject),
    /// with every conclusion depending on one.
    pub unstorable: Vec<Conclusion>,
    /// The status of the reasoning run.
    pub status: Status,
    /// True when the commit was a dry run.
    pub dry_run: bool,
}

/// The reasoning half of a materialization: what the commit writes.
struct Prepared {
    rule_set: Value,
    r: Eid,
    result: ReasonResult,
}

/// What became of one conclusion in the commit.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Fate {
    Written(Eid),
    Stale,
    Unstorable,
}

impl Db {
    /// Reasons with stored rule set `rule_set` over a snapshot of the now view
    /// and writes its conclusions as derived beliefs in one transaction
    /// (feature `n3`).
    ///
    /// The run loads the scope of `opts` (excluding the statements derived from
    /// the rule set's own live statement `R`, which it recomputes), sweeps the
    /// valid-time segments of `opts.window`, and writes every conclusion in proof
    /// order through `derive`, with support = its premise statements, the
    /// statements written for its derived premises, and `R`, plus the labels
    /// `(D n3:derivedBy rule_set)` and `(D n3:rule i)`. Retracting or correcting
    /// a premise, or editing the rule set, retracts the conclusion through the
    /// support cascade. A conclusion already live with the same content and
    /// support is `existing` when its interval contains the new one, and is
    /// superseded to the hull when they overlap or touch (`widened`). A blank
    /// node becomes the node of `upsert_derived(n3:skolem,
    /// "<R raw id>/<rule>/<label>", support)`, so re-runs reuse it. A conclusion
    /// whose support was retracted after the snapshot is skipped with its
    /// dependents and listed in `stale_support`.
    ///
    /// # Errors
    ///
    /// `UnknownRuleSet` when the rule set has no live `n3:source`;
    /// `N3NotMaterializable` for a non-monotonic, clock-dependent or
    /// untraced-premise rule set (before reasoning); `N3Contradiction` when an
    /// inference fuse fires and `N3Incomplete` for an incomplete run with
    /// `require_complete` (both before writing); the budget, read and write
    /// errors.
    ///
    /// ```
    /// # use tiramemsu::*;
    /// # let dir = tempfile::tempdir().unwrap();
    /// # let db = Db::open(dir.path().join("m.db"), OpenOptions::default())?;
    /// let v = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
    /// db.transact(TxOptions::default(), |tx| {
    ///     tx.assert(v("socrates"), Value::iri(vocab::RDF_TYPE), v("Human"), Valid::ALWAYS)?;
    ///     tx.store_rules(v("mortality"), "{ ?x a v:Human } => { ?x a v:Mortal } .")?;
    ///     Ok(())
    /// })?;
    /// let r = db.materialize(&v("mortality"), &MaterializeOptions::default())?;
    /// assert_eq!(r.derived.len(), 1);
    /// // a second run finds the conclusion and writes nothing
    /// let again = db.materialize(&v("mortality"), &MaterializeOptions::default())?;
    /// assert_eq!(again.existing, r.derived);
    /// assert!(again.commit.unwrap().asserted.is_empty());
    /// # Ok::<(), Error>(())
    /// ```
    // @tg: implements:: [[openspec:n3-materialization#Materialize a rule set]] {since: "2026-10-28"}
    pub fn materialize(
        &self,
        rule_set: &Value,
        opts: &MaterializeOptions,
    ) -> Result<MaterializeReport> {
        self.materialize_between(rule_set, opts, |_| Ok(()))
    }

    /// [`Db::materialize`] with `between` run after the snapshot is reasoned and
    /// before the commit opens: a test hook for writes that race the commit.
    #[doc(hidden)]
    pub fn materialize_between(
        &self,
        rule_set: &Value,
        opts: &MaterializeOptions,
        between: impl FnOnce(&Db) -> Result<()>,
    ) -> Result<MaterializeReport> {
        let prepared = self.prepare_materialize(rule_set, opts)?;
        between(self)?;
        self.commit_materialize(prepared, opts)
    }

    /// Reasons on a snapshot outside the writer (design N13) and applies the
    /// refusals that come before writing.
    // @tg: implements:: [[openspec:n3-materialization#Snapshot then commit]] {since: "2026-10-28"}
    fn prepare_materialize(&self, rule_set: &Value, opts: &MaterializeOptions) -> Result<Prepared> {
        let ropts = ReasonOptions {
            scope: opts.scope.clone(),
            window: opts.window,
            time: TimeMode::Pointwise,
            max_segments: opts.max_segments,
            proof_n3: false,
            limits: opts.limits,
        };
        let result = self.now().reason_for_materialization(rule_set, &ropts)?;
        if let Some(c) = &result.contradiction {
            return Err(Error::N3Contradiction { rule: c.rule });
        }
        if let (true, Status::Incomplete { limits }) = (opts.require_complete, &result.status) {
            return Err(Error::N3Incomplete {
                limits: limits.iter().map(|l| format!("{l:?}")).collect(),
                errors: result.errors.clone(),
            });
        }
        let r = result
            .rule_set_statement
            .ok_or_else(|| Error::UnknownRuleSet {
                rule_set: rule_set.to_string(),
            })?;
        Ok(Prepared {
            rule_set: rule_set.clone(),
            r,
            result,
        })
    }

    /// Writes the prepared conclusions in one transaction, in proof order.
    // @tg: implements:: [[openspec:n3-materialization#Valid time of materialized conclusions]] {since: "2026-10-28"}
    // @tg: implements:: [[openspec:n3-materialization#Conclusions fall with premises and rules]] {since: "2026-10-28"}
    fn commit_materialize(
        &self,
        p: Prepared,
        opts: &MaterializeOptions,
    ) -> Result<MaterializeReport> {
        let mut out = MaterializeReport {
            rule_set: p.rule_set.clone(),
            rules_statement: p.r,
            reasoned_at: p.result.t,
            commit: None,
            derived: Vec::new(),
            existing: Vec::new(),
            widened: Vec::new(),
            skolems: Vec::new(),
            stale_support: Vec::new(),
            unstorable: Vec::new(),
            status: p.result.status.clone(),
            dry_run: opts.dry_run,
        };
        let tx_opts = TxOptions {
            dry_run: opts.dry_run,
            ..TxOptions::default()
        };
        let report = self.transact(tx_opts, |tx| {
            Commit {
                tx,
                rule_set: &p.rule_set,
                r: p.r,
                skolems: HashMap::new(),
                used: BTreeSet::new(),
            }
            .run(&p.result.conclusions, &mut out)
        })?;
        out.commit = Some(report);
        Ok(out)
    }
}

/// The state of one commit.
struct Commit<'t, 'a> {
    tx: &'t mut Tx<'a>,
    rule_set: &'t Value,
    r: Eid,
    /// (key, support) -> the node `upsert_derived` returned in this run.
    skolems: HashMap<(String, Vec<Eid>), ObjectId>,
    /// The statements this run assigned to a conclusion: never widened again in
    /// the same run, since a later conclusion may already rest on them.
    used: BTreeSet<Eid>,
}

/// True when the half-open intervals overlap or touch.
fn meets(a: &Valid, b: &Valid) -> bool {
    let a_to_ok = match (a.to, b.from) {
        (Some(t), Some(f)) => f <= t,
        _ => true,
    };
    let b_to_ok = match (b.to, a.from) {
        (Some(t), Some(f)) => f <= t,
        _ => true,
    };
    a_to_ok && b_to_ok
}

/// True when `outer` contains `inner`.
fn contains(outer: &Valid, inner: &Valid) -> bool {
    let from_ok = match (outer.from, inner.from) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(a), Some(b)) => a <= b,
    };
    let to_ok = match (outer.to, inner.to) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(a), Some(b)) => a >= b,
    };
    from_ok && to_ok
}

/// The smallest interval containing both.
fn hull(a: &Valid, b: &Valid) -> Valid {
    Valid {
        from: a.from.zip(b.from).map(|(x, y)| x.min(y)),
        to: a.to.zip(b.to).map(|(x, y)| x.max(y)),
    }
}

/// The part of `a` before or after `b`, for intervals that meet without `b`
/// containing `a` and without `b` lying strictly inside `a` (then the part
/// before it).
fn beside(a: &Valid, b: &Valid) -> Valid {
    let b_starts_first = match (b.from, a.from) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(x), Some(y)) => x <= y,
    };
    if b_starts_first {
        Valid {
            from: b.to,
            to: a.to,
        }
    } else {
        Valid {
            from: a.from,
            to: b.from,
        }
    }
}

/// The tag a subject value encodes to, `None` for a value that cannot be one.
fn subject_tag(v: &Value) -> Option<Tag> {
    match v {
        Value::Iri(_) => Some(Tag::Iri),
        Value::Node(_) => Some(Tag::Node),
        Value::BNode(_) => Some(Tag::BNode),
        Value::Stmt(_) => Some(Tag::Stmt),
        Value::Tx(_) => Some(Tag::Tx),
        _ => None,
    }
}

impl Commit<'_, '_> {
    fn run(&mut self, conclusions: &[Conclusion], out: &mut MaterializeReport) -> Result<()> {
        let mut fates: Vec<Fate> = Vec::with_capacity(conclusions.len());
        for (i, c) in conclusions.iter().enumerate() {
            if i % 256 == 255 && budget::active() {
                budget::check()?;
            }
            let fate = self.one(c, &fates, out)?;
            match fate {
                Fate::Stale => out.stale_support.push(c.clone()),
                Fate::Unstorable => out.unstorable.push(c.clone()),
                Fate::Written(e) => {
                    self.used.insert(e);
                }
            }
            fates.push(fate);
        }
        Ok(())
    }

    fn live(&mut self, e: Eid) -> Result<bool> {
        Ok(self.tx.statement(e)?.is_some_and(|row| row.live()))
    }

    /// Writes one conclusion (design N9, N13, N14, N16).
    // @tg: implements:: [[openspec:n3-materialization#Skolem nodes]] {since: "2026-10-28"}
    fn one(&mut self, c: &Conclusion, fates: &[Fate], out: &mut MaterializeReport) -> Result<Fate> {
        // support: premise statements, statements written for derived premises, R
        let mut support = BTreeSet::from([self.r]);
        let mut unstorable = false;
        for p in &c.proof.premises {
            let e = match *p {
                Premise::Statement(e) => e,
                Premise::Conclusion(j) => match fates.get(j) {
                    Some(Fate::Written(e)) => *e,
                    Some(Fate::Stale) => return Ok(Fate::Stale),
                    Some(Fate::Unstorable) | None => {
                        unstorable = true;
                        continue;
                    }
                },
            };
            support.insert(e);
        }
        // a support retracted since the snapshot: skip, never fail the run
        for &e in &support {
            if !self.live(e)? {
                return Ok(Fate::Stale);
            }
        }
        if unstorable {
            return Ok(Fate::Unstorable);
        }
        let support: Vec<Eid> = support.into_iter().collect();

        // the terms as stored objects, blank nodes as Skolem nodes
        let rule = c.proof.rule;
        let p_iri = match &c.p {
            Term::Value(Value::Iri(p)) => p.clone(),
            _ => return Ok(Fate::Unstorable),
        };
        let s_tag = match &c.s {
            Term::Value(v) => subject_tag(v),
            Term::Blank(_) => Some(Tag::Node),
            _ => None,
        };
        let Some(s_tag) = s_tag else {
            return Ok(Fate::Unstorable);
        };
        if reserved::check_predicate(&p_iri, s_tag).is_err() {
            return Ok(Fate::Unstorable);
        }
        if matches!(c.o, Term::List(_) | Term::Formula(_)) {
            return Ok(Fate::Unstorable);
        }
        let s = self.obj(&c.s, rule, &support, out)?;
        let o = self.obj(&c.o, rule, &support, out)?;
        let p = Obj::Val(Value::Iri(p_iri));

        // an earlier materialization of the same content from the same support
        let mut widen = None;
        let mut valid = c.valid;
        for (e, v) in self.same_derivations(&s, &p, &o, &support)? {
            if contains(&v, &c.valid) {
                out.existing.push(e);
                return Ok(Fate::Written(e));
            }
            if meets(&v, &c.valid) {
                if !self.used.contains(&e) {
                    widen.get_or_insert((e, v));
                } else if widen.is_none() {
                    // widening it would retract what this run already wrote on
                    // it: derive the part it does not cover, touching it
                    valid = beside(&c.valid, &v);
                }
            }
        }
        if let Some((old, valid)) = widen {
            let h = hull(&valid, &c.valid);
            let patch = Patch {
                v_from: Some(h.from),
                v_to: Some(h.to),
                ..Patch::default()
            };
            // the replay keeps its support links and its `n3:` labels
            let new = self.tx.supersede(old, patch)?;
            out.widened.push((old, new));
            return Ok(Fate::Written(new));
        }
        match self.tx.derive(s, p, o, valid, &support)? {
            Asserted::New(d) => {
                self.tx.assert(
                    d,
                    Value::iri(N3_DERIVED_BY),
                    self.rule_set.clone(),
                    Valid::ALWAYS,
                )?;
                self.tx.assert(
                    d,
                    Value::iri(N3_RULE),
                    Value::Int(rule as i64),
                    Valid::ALWAYS,
                )?;
                out.derived.push(d);
                Ok(Fate::Written(d))
            }
            Asserted::Existing(d) => {
                out.existing.push(d);
                Ok(Fate::Written(d))
            }
        }
    }

    /// A term as a statement position: a value, or the Skolem node of a blank
    /// node (`upsert_derived(n3:skolem, "<R>/<rule>/<label>", support)`).
    fn obj(
        &mut self,
        t: &Term,
        rule: usize,
        support: &[Eid],
        out: &mut MaterializeReport,
    ) -> Result<Obj> {
        match t {
            Term::Value(v) => Ok(Obj::Val(v.clone())),
            Term::Blank(label) => {
                let key = format!("{}/{rule}/{label}", self.r.oid().raw());
                let k = (key, support.to_vec());
                if let Some(&n) = self.skolems.get(&k) {
                    return Ok(Obj::Id(n));
                }
                let n = self.tx.upsert_derived(
                    Value::iri(N3_SKOLEM),
                    Value::str(k.0.as_str()),
                    support,
                )?;
                if !out.skolems.contains(&n) {
                    out.skolems.push(n);
                }
                self.skolems.insert(k, n);
                Ok(Obj::Id(n))
            }
            Term::List(_) | Term::Formula(_) => Err(Error::storage("an unstorable term")),
        }
    }

    /// The live statements with content `(s, p, o)` whose live support links
    /// name exactly `support`, with their valid time.
    fn same_derivations(
        &mut self,
        s: &Obj,
        p: &Obj,
        o: &Obj,
        support: &[Eid],
    ) -> Result<Vec<(Eid, Valid)>> {
        let mut ids = [None; 3];
        for (slot, x) in ids.iter_mut().zip([s, p, o]) {
            *slot = match x {
                Obj::Id(id) => Some(*id),
                Obj::Val(v) => self.tx.lookup(v)?,
            };
        }
        let ([Some(s), Some(p), Some(o)], Some(df)) =
            (ids, self.tx.lookup(&Value::iri(SYS_DERIVED_FROM))?)
        else {
            return Ok(Vec::new());
        };
        let mut found = Vec::new();
        for row in self.tx.live_rows(Some(s), Some(p), Some(o))? {
            let Some(e) = Eid::from_oid(ObjectId::from_raw(row.eid)) else {
                continue;
            };
            let links: BTreeSet<Eid> = self
                .tx
                .live_rows(Some(e.oid()), Some(df), None)?
                .into_iter()
                .filter_map(|l| Eid::from_oid(ObjectId::from_raw(l.o)))
                .collect();
            if links.iter().eq(support.iter()) {
                found.push((
                    e,
                    Valid {
                        from: row.v_from,
                        to: row.v_to,
                    },
                ));
            }
        }
        Ok(found)
    }
}
