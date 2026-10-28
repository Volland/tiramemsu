//! Derived beliefs: a statement written together with engine-owned support links
//! `(D sys:derivedFrom e)`. A port of `tm_core::engine::derive`.

use std::collections::BTreeSet;

use tm_core::vocab;
use tm_core::{Asserted, Eid, Error, ObjectId, Position, Result, Tag, Valid};

use super::ops::Writer;
use super::{IntoObject, Tx};

/// One support link of a derivation: the support, the link and whether it is new.
pub(crate) type SupportLink = (Eid, Eid, bool);

impl Tx<'_> {
    /// Asserts the belief `D = (s, p, o)` valid over `valid` as derived from every
    /// statement in `support` (see `tm_core::Tx::derive`): `Existing(D)` for a
    /// live `D` with the same content, an overlapping interval and exactly the
    /// same live support set, otherwise a new `D` with one engine-owned link
    /// `(D sys:derivedFrom e)` per support, in ascending eid order.
    // @tg: implements:: [[openspec:derived-beliefs#Derive a belief with support]] {since: "2026-10-28"}
    // @tg: implements:: [[openspec:derived-beliefs#Derivation idempotency]] {since: "2026-10-28"}
    // @tg: implements:: [[openspec:derived-beliefs#Derivation in both engines]] {since: "2026-10-28"}
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

    /// The node of a live key `(n p o)` derived from exactly `support`, or a new
    /// node whose key is written through [`Tx::derive`] (see
    /// `tm_core::Tx::upsert_derived`).
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
        for r in self.live_rows(None, Some(p), Some(o))? {
            let Some(key) = Eid::from_oid(ObjectId::from_raw(r.eid)) else {
                continue;
            };
            if self.support_set(key)? == support {
                return Ok(ObjectId::from_raw(r.s));
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
        let candidates: Vec<Eid> = self
            .live_rows(Some(s), Some(p), Some(o))?
            .into_iter()
            .filter(|r| {
                Valid {
                    from: r.v_from,
                    to: r.v_to,
                }
                .overlaps(&valid)
            })
            .filter_map(|r| Eid::from_oid(ObjectId::from_raw(r.eid)))
            .collect();
        for d in candidates {
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
            .live_rows(Some(d.oid()), Some(df), None)?
            .into_iter()
            .filter_map(|r| {
                let id = |x: i64| Eid::from_oid(ObjectId::from_raw(x));
                Some((id(r.o)?, id(r.eid)?))
            })
            .collect())
    }
}
