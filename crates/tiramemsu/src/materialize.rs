//! The commit half of materialization, shared by N3 rule sets and Datalog
//! programs (OpenSpec change `add-datalog-materialization`, decision DL13).
//!
//! The reasoning half hands over items in proof order: content, valid time,
//! the rule and the support (stored statements and earlier items). One write
//! transaction then derives each item through `derive` with the support plus
//! the rules' statement `R`, skips an item whose support was retracted since
//! the snapshot together with everything resting on it, finds earlier
//! derivations with the same content and support (existing, or widened to the
//! hull by supersede), turns invented terms into skolem nodes through
//! `upsert_derived`, and writes the labels.

use std::collections::{BTreeSet, HashMap};

use tm_core::engine::reserved;
use tm_core::vocab::SYS_DERIVED_FROM;
use tm_core::{
    budget, Asserted, Eid, Obj, ObjectId, Patch, Result, Tag, TxHandle as Tx, Valid, Value,
};

/// A term of an item.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum MTerm {
    /// A value.
    Value(Value),
    /// A node the rules invented, with a label stable across runs.
    #[cfg_attr(not(any(feature = "n3", feature = "datalog")), allow(dead_code))]
    Invent(String),
    /// Something a statement cannot hold (a list, a formula).
    Unstorable,
}

/// One premise of an item.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum MSupport {
    /// A stored statement.
    Statement(Eid),
    /// An earlier item.
    #[cfg_attr(not(any(feature = "n3", feature = "datalog")), allow(dead_code))]
    Item(usize),
}

/// One conclusion to write.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MItem {
    pub s: MTerm,
    /// The predicate IRI, `None` when it is not one.
    pub p: Option<String>,
    pub o: MTerm,
    pub valid: Valid,
    pub rule: usize,
    pub support: Vec<MSupport>,
}

/// The vocabulary of one rule language.
pub(crate) struct Labels {
    pub derived_by: &'static str,
    pub rule: &'static str,
    pub skolem: &'static str,
}

/// What became of one item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fate {
    Written(Eid),
    Stale,
    Unstorable,
}

/// What one commit did.
#[derive(Debug, Default)]
pub(crate) struct Outcome {
    pub derived: Vec<Eid>,
    pub existing: Vec<Eid>,
    pub widened: Vec<(Eid, Eid)>,
    pub skolems: Vec<ObjectId>,
    pub fates: Vec<Fate>,
}

/// Writes `items` in order (they reference only earlier items).
// @tg: implements:: [[openspec:datalog-materialization#Materialize a program]] {since: "2026-10-28", confidence: partial}
// @tg: implements:: [[openspec:datalog-materialization#Snapshot then commit]] {since: "2026-10-28"}
// @tg: implements:: [[openspec:datalog-materialization#Invented nodes]] {since: "2026-10-28"}
// @tg: implements:: [[openspec:datalog-materialization#Valid time of materialized conclusions]] {since: "2026-10-28"}
pub(crate) fn commit(
    tx: &mut Tx<'_>,
    items: &[MItem],
    node: &Value,
    r: Eid,
    labels: &Labels,
) -> Result<Outcome> {
    let mut c = Commit {
        tx,
        node,
        r,
        labels,
        skolems: HashMap::new(),
        used: BTreeSet::new(),
    };
    let mut out = Outcome::default();
    for (i, item) in items.iter().enumerate() {
        if i % 256 == 255 && budget::active() {
            budget::check()?;
        }
        let fate = c.one(item, &out.fates.clone(), &mut out)?;
        if let Fate::Written(e) = fate {
            c.used.insert(e);
        }
        out.fates.push(fate);
    }
    Ok(out)
}

struct Commit<'t, 'a> {
    tx: &'t mut Tx<'a>,
    node: &'t Value,
    r: Eid,
    labels: &'t Labels,
    /// (key, support) -> the node `upsert_derived` returned in this run.
    skolems: HashMap<(String, Vec<Eid>), ObjectId>,
    /// The statements this run assigned to an item: never widened again in the
    /// same run, since a later item may already rest on them.
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
    fn live(&mut self, e: Eid) -> Result<bool> {
        Ok(self.tx.statement(e)?.is_some_and(|row| row.live()))
    }

    fn one(&mut self, c: &MItem, fates: &[Fate], out: &mut Outcome) -> Result<Fate> {
        // support: premise statements, statements written for earlier items, R
        let mut support = BTreeSet::from([self.r]);
        let mut unstorable = false;
        for p in &c.support {
            let e = match *p {
                MSupport::Statement(e) => e,
                MSupport::Item(j) => match fates.get(j) {
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
        let Some(p_iri) = c.p.clone() else {
            return Ok(Fate::Unstorable);
        };
        let s_tag = match &c.s {
            MTerm::Value(v) => subject_tag(v),
            MTerm::Invent(_) => Some(Tag::Node),
            MTerm::Unstorable => None,
        };
        let Some(s_tag) = s_tag else {
            return Ok(Fate::Unstorable);
        };
        if reserved::check_predicate(&p_iri, s_tag).is_err() || c.o == MTerm::Unstorable {
            return Ok(Fate::Unstorable);
        }
        let s = self.obj(&c.s, c.rule, &support, out)?;
        let o = self.obj(&c.o, c.rule, &support, out)?;
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
            // the replay keeps its support links and its labels
            let new = self.tx.supersede(old, patch)?;
            out.widened.push((old, new));
            return Ok(Fate::Written(new));
        }
        match self.tx.derive(s, p, o, valid, &support)? {
            Asserted::New(d) => {
                self.tx.assert(
                    d,
                    Value::iri(self.labels.derived_by),
                    self.node.clone(),
                    Valid::ALWAYS,
                )?;
                self.tx.assert(
                    d,
                    Value::iri(self.labels.rule),
                    Value::Int(c.rule as i64),
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

    /// A term as a statement position: a value, or the skolem node of an
    /// invented term (`upsert_derived(skolem, "<R>/<rule>/<label>", support)`).
    fn obj(&mut self, t: &MTerm, rule: usize, support: &[Eid], out: &mut Outcome) -> Result<Obj> {
        match t {
            MTerm::Value(v) => Ok(Obj::Val(v.clone())),
            MTerm::Invent(label) => {
                let key = format!("{}/{rule}/{label}", self.r.oid().raw());
                let k = (key, support.to_vec());
                if let Some(&n) = self.skolems.get(&k) {
                    return Ok(Obj::Id(n));
                }
                let n = self.tx.upsert_derived(
                    Value::iri(self.labels.skolem),
                    Value::str(k.0.as_str()),
                    support,
                )?;
                if !out.skolems.contains(&n) {
                    out.skolems.push(n);
                }
                self.skolems.insert(k, n);
                Ok(Obj::Id(n))
            }
            MTerm::Unstorable => Err(tm_core::Error::storage("an unstorable term")),
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
