//! Derived beliefs: a statement written together with engine-owned support links
//! `(D sys:derivedFrom e)`, one per supporting statement (spec `derived-beliefs`).
//!
//! The links make each support a referent of `D`, so the cascade of a support's
//! retraction reaches `D` ([`Tx::cascade_set`]) and a supersede of a support drops
//! `D` instead of replaying it.

use std::collections::BTreeSet;

use super::ops::Writer;
use super::{IntoObject, Tx};
use crate::error::{Error, Position, Result};
use crate::exec::SqlValue;
use crate::id::{Eid, ObjectId, Tag};
use crate::report::{Asserted, Valid};
use crate::vocab;

/// One support link of a derivation: the support, the link and whether it is new.
pub(crate) type SupportLink = (Eid, Eid, bool);

impl Tx<'_> {
    /// Asserts the belief `D = (s, p, o)` valid over `valid` as derived from every
    /// statement in `support`: inserts `D` and one engine-owned link
    /// `(D sys:derivedFrom e)` per support `e`, in ascending eid order.
    ///
    /// The derivation is conjunctive: when any support is retracted (or
    /// superseded), `D` is retracted in the same transaction. Deriving the same
    /// content again from the same live support set returns `Existing(D)`; any
    /// other derive creates a new statement, also over an identical statement that
    /// was asserted independently or derived from other support, so an
    /// independent fact never becomes dependent and parallel derivations stand
    /// for alternative grounds. Schema checks apply to `D` as to [`Tx::create`].
    ///
    /// # Errors
    ///
    /// [`Error::InvalidTerm`] for an empty `support`, [`Error::NotLive`] naming
    /// the first support that is retracted or unknown,
    /// [`Error::ReservedNamespace`] for a reserved predicate (`sys:derivedFrom`
    /// itself among them), and the schema errors of assert.
    ///
    /// # Example
    ///
    /// ```
    /// use tm_core::{vocab::v, Store, StoreOptions, TxOptions, Valid, Value};
    /// use tm_rusqlite as host;
    ///
    /// # let dir = tempfile::tempdir().unwrap();
    /// # let path = dir.path().join("db");
    /// let mut store = Store::open(&host::RusqliteHost::new(), &path, StoreOptions::default())?;
    /// let r = store.transact(TxOptions::default(), |tx| {
    ///     let e1 = tx.assert(Value::iri(v("alice")), Value::iri(v("ordered")), Value::iri(v("tea")), Valid::ALWAYS)?.eid();
    ///     tx.derive(Value::iri(v("alice")), Value::iri(v("prefers")), Value::iri(v("tea")), Valid::ALWAYS, &[e1])?;
    ///     Ok(())
    /// })?;
    /// let (e1, belief) = (r.asserted[0], r.asserted[1]);
    /// // retracting the evidence retracts the belief
    /// let r = store.transact(TxOptions::default(), |tx| tx.retract(e1).map(|_| ()))?;
    /// assert!(r.retracted.iter().any(|(e, _)| *e == belief));
    /// # Ok::<(), tm_core::Error>(())
    /// ```
    // @tg: implements:: [[openspec:derived-beliefs#Derive a belief with support]] {since: "2026-10-28"}
    // @tg: implements:: [[openspec:derived-beliefs#Derivation idempotency]] {since: "2026-10-28"}
    pub fn derive(
        &mut self,
        s: impl IntoObject,
        p: impl IntoObject,
        o: impl IntoObject,
        valid: Valid,
        support: &[Eid],
    ) -> Result<Asserted> {
        let (s, p, o) = (
            s.into_object(self)?,
            p.into_object(self)?,
            o.into_object(self)?,
        );
        Ok(self.derive_links(s, p, o, valid, support)?.0)
    }

    /// For predicate `p`: the node `n` of a live key `(n p o)` derived from
    /// exactly the live support set `support`, or a new node whose key is written
    /// through [`Tx::derive`] with `support` (valid always).
    ///
    /// A rule engine names the node it invents for an existential this way, so a
    /// re-run reuses it, and its key falls with its support. A key asserted
    /// independently, or derived from another support set, is never reused.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidTerm`] for a predicate that is not a known IRI, and the
    /// errors of [`Tx::derive`].
    // @tg: implements:: [[openspec:derived-beliefs#Derived upsert]] {since: "2026-10-28"}
    pub fn upsert_derived(
        &mut self,
        p: impl IntoObject,
        o: impl IntoObject,
        support: &[Eid],
    ) -> Result<ObjectId> {
        let (p, o) = (p.into_object(self)?, o.into_object(self)?);
        if p.tag()? != Tag::Iri {
            return Err(Error::InvalidTerm {
                position: Position::Predicate,
                reason: "predicate must be an IRI".to_string(),
            });
        }
        self.check_known(p, Position::Predicate)?;
        let support = self.check_support(support)?;
        let keys = self.exec.rows(
            "SELECT eid, s FROM triple WHERE p = +?1 AND o = +?2 AND t_ret IS NULL ORDER BY eid",
            &[SqlValue::Integer(p.raw()), SqlValue::Integer(o.raw())],
        )?;
        for r in keys {
            let (Some(key), Some(n)) = (
                r[0].as_i64()
                    .and_then(|e| Eid::from_oid(ObjectId::from_raw(e))),
                r[1].as_i64(),
            ) else {
                continue;
            };
            if self.support_set(key)? == support {
                return Ok(ObjectId::from_raw(n));
            }
        }
        let node = self.new_node()?;
        self.derive_new(node, p, o, Valid::ALWAYS, &support)?;
        Ok(node)
    }

    /// [`Tx::derive`] on encoded positions, also returning the support links.
    pub(crate) fn derive_links(
        &mut self,
        s: ObjectId,
        p: ObjectId,
        o: ObjectId,
        valid: Valid,
        support: &[Eid],
    ) -> Result<(Asserted, Vec<SupportLink>)> {
        let support = self.check_support(support)?;
        valid.check()?;
        let candidates = self.exec.rows(
            "SELECT eid FROM triple WHERE s = +?1 AND p = +?2 AND o = +?3 AND t_ret IS NULL \
             AND (v_from IS NULL OR ?5 IS NULL OR v_from < ?5) \
             AND (?4 IS NULL OR v_to IS NULL OR ?4 < v_to) ORDER BY eid",
            &[
                SqlValue::Integer(s.raw()),
                SqlValue::Integer(p.raw()),
                SqlValue::Integer(o.raw()),
                SqlValue::from(valid.from),
                SqlValue::from(valid.to),
            ],
        )?;
        for r in candidates {
            let Some(d) = r[0]
                .as_i64()
                .and_then(|e| Eid::from_oid(ObjectId::from_raw(e)))
            else {
                continue;
            };
            let links = self.support_links(d)?;
            let set: BTreeSet<Eid> = links.iter().map(|(e, _)| *e).collect();
            if set == support {
                self.existing.push(d);
                let links = links.into_iter().map(|(e, l)| (e, l, false)).collect();
                return Ok((Asserted::Existing(d), links));
            }
        }
        let (d, links) = self.derive_new(s, p, o, valid, &support)?;
        Ok((Asserted::New(d), links))
    }

    /// Inserts `D = (s, p, o)` through the create pipeline and its support links.
    fn derive_new(
        &mut self,
        s: ObjectId,
        p: ObjectId,
        o: ObjectId,
        valid: Valid,
        support: &BTreeSet<Eid>,
    ) -> Result<(Eid, Vec<SupportLink>)> {
        let d = self.write(s, p, o, valid, false, Writer::User)?.eid();
        let df = self.sys(vocab::SYS_DERIVED_FROM)?;
        let mut links = Vec::with_capacity(support.len());
        for e in support {
            // a support retracted by the schema pipeline of `D` fails here, NotLive
            let l = self.write(d.oid(), df, e.oid(), Valid::ALWAYS, true, Writer::Engine)?;
            links.push((*e, l.eid(), l.is_new()));
        }
        Ok((d, links))
    }

    /// The support set of a derivation: non-empty, every member a live statement.
    fn check_support(&mut self, support: &[Eid]) -> Result<BTreeSet<Eid>> {
        if support.is_empty() {
            return Err(Error::InvalidTerm {
                position: Position::Object,
                reason: "a derivation needs at least one supporting statement".to_string(),
            });
        }
        let set: BTreeSet<Eid> = support.iter().copied().collect();
        for e in &set {
            e.oid().check_origin()?;
            if self.live(*e)? != Some(true) {
                return Err(Error::NotLive(*e));
            }
        }
        Ok(set)
    }

    /// The live supports of `d`.
    fn support_set(&mut self, d: Eid) -> Result<BTreeSet<Eid>> {
        Ok(self.support_links(d)?.into_iter().map(|(e, _)| e).collect())
    }

    /// The live support links `(d sys:derivedFrom e)` of `d` as `(e, link)`, by link eid.
    fn support_links(&mut self, d: Eid) -> Result<Vec<(Eid, Eid)>> {
        let Some(df) = self.sys_lookup(vocab::SYS_DERIVED_FROM)? else {
            return Ok(Vec::new());
        };
        Ok(self
            .exec
            .rows(
                "SELECT o, eid FROM triple WHERE s = +?1 AND p = +?2 AND t_ret IS NULL ORDER BY eid",
                &[SqlValue::Integer(d.oid().raw()), SqlValue::Integer(df.raw())],
            )?
            .into_iter()
            .filter_map(|r| {
                let id = |k: usize| {
                    r[k].as_i64()
                        .and_then(|x| Eid::from_oid(ObjectId::from_raw(x)))
                };
                Some((id(0)?, id(1)?))
            })
            .collect())
    }
}
