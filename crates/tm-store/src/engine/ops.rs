//! Statement operations: assert, create, retract, retract_matching, confirm, meta,
//! upsert. A port of `tm_core::engine::ops`.

use tm_core::engine::reserved;
use tm_core::{
    AssertOpts, Asserted, Eid, Error, ObjectId, OnExisting, Position, Result, RetKind, Tag, Valid,
};

use super::schema::Flag;
use super::{IntoObject, Tx};
use crate::store::{Pattern, Scope};
use tm_core::vocab;

/// Who is writing: user writes are checked against the reserved namespace.
#[derive(Copy, Clone, PartialEq, Eq)]
pub(crate) enum Writer {
    User,
    Engine,
}

impl Tx<'_> {
    /// Asserts `(s, p, o)` valid over `valid`. Idempotent: returns `Existing(eid)`
    /// when a live statement with the same `(s, p, o)` and an overlapping valid
    /// interval exists.
    pub fn assert(
        &mut self,
        s: impl IntoObject,
        p: impl IntoObject,
        o: impl IntoObject,
        valid: Valid,
    ) -> Result<Asserted> {
        self.assert_with(
            s,
            p,
            o,
            AssertOpts {
                valid,
                on_existing: OnExisting::Return,
            },
        )
    }

    /// [`Tx::assert`] with options; `OnExisting::Confirm` also confirms an existing match.
    pub fn assert_with(
        &mut self,
        s: impl IntoObject,
        p: impl IntoObject,
        o: impl IntoObject,
        opts: AssertOpts,
    ) -> Result<Asserted> {
        let (s, p, o) = (
            s.into_object(self)?,
            p.into_object(self)?,
            o.into_object(self)?,
        );
        let r = self.write(s, p, o, opts.valid, true, Writer::User)?;
        if let Asserted::Existing(e) = r {
            self.existing.push(e);
            if opts.on_existing == OnExisting::Confirm {
                self.confirm(e)?;
            }
        }
        Ok(r)
    }

    /// Always inserts a new statement (parallel edges); schema checks still apply.
    pub fn create(
        &mut self,
        s: impl IntoObject,
        p: impl IntoObject,
        o: impl IntoObject,
        valid: Valid,
    ) -> Result<Eid> {
        let (s, p, o) = (
            s.into_object(self)?,
            p.into_object(self)?,
            o.into_object(self)?,
        );
        Ok(self.write(s, p, o, valid, false, Writer::User)?.eid())
    }

    /// Asserts a metadata statement about this transaction: `(tx p o)`.
    pub fn meta(&mut self, p: impl IntoObject, o: impl IntoObject) -> Result<Eid> {
        let s = self.t().oid();
        let r = self.assert(s, p, o, Valid::ALWAYS)?;
        Ok(r.eid())
    }

    /// Records that this transaction corroborates live statement `eid`.
    pub fn confirm(&mut self, eid: Eid) -> Result<Eid> {
        eid.oid().check_origin()?;
        if self.live(eid)? != Some(true) {
            return Err(Error::NotLive(eid));
        }
        let p = self.sys(vocab::SYS_CONFIRMED_BY)?;
        let o = self.t().oid();
        Ok(self
            .write(eid.oid(), p, o, Valid::ALWAYS, true, Writer::Engine)?
            .eid())
    }

    /// For a `sys:unique` predicate: returns the live subject holding `o`, or creates
    /// a new node and asserts `(node p o)`.
    pub fn upsert(&mut self, p: impl IntoObject, o: impl IntoObject) -> Result<ObjectId> {
        let (p, o) = (p.into_object(self)?, o.into_object(self)?);
        if p.tag()? != Tag::Iri {
            return Err(Error::InvalidTerm {
                position: Position::Predicate,
                reason: "predicate must be an IRI".to_string(),
            });
        }
        self.check_known(p, Position::Predicate)?;
        if !self.schema(p)?.unique {
            return Err(Error::NotUniquePredicate(p));
        }
        if let Some(r) = self.live_rows(None, Some(p), Some(o))?.first() {
            return Ok(ObjectId::from_raw(r.s));
        }
        let node = self.new_node()?;
        self.assert(node, p, o, Valid::ALWAYS)?;
        Ok(node)
    }

    /// Retracts a live statement with cascade. Returns false for a retracted or
    /// unknown eid.
    pub fn retract(&mut self, eid: Eid) -> Result<bool> {
        self.retract_root(eid, RetKind::Explicit)
    }

    /// Retracts every statement live now that matches the bound positions (valid time
    /// ignored), in ascending eid order, and returns the matched eids.
    pub fn retract_matching(
        &mut self,
        s: Option<ObjectId>,
        p: Option<ObjectId>,
        o: Option<ObjectId>,
    ) -> Result<Vec<Eid>> {
        for v in [s, p, o].into_iter().flatten() {
            v.tag()?;
            v.check_origin()?;
        }
        let matched: Vec<Eid> = self
            .live_rows(s, p, o)?
            .into_iter()
            .filter_map(|r| Eid::from_oid(ObjectId::from_raw(r.eid)))
            .collect();
        for e in &matched {
            self.retract_root(*e, RetKind::Explicit)?;
        }
        Ok(matched)
    }

    /// `Some(true)` live, `Some(false)` retracted, `None` unknown.
    pub(crate) fn live(&mut self, eid: Eid) -> Result<Option<bool>> {
        Ok(self.store.statement(eid.oid().raw())?.map(|r| r.live()))
    }

    /// Structural endpoints must be live; engine lineage may name allocated history.
    pub(crate) fn check_statement_references(
        &mut self,
        s: ObjectId,
        p: ObjectId,
        o: ObjectId,
    ) -> Result<()> {
        let lineage = self.iri_of(p)?.as_deref() == Some(vocab::SYS_SUPERSEDES);
        for id in [s, o] {
            if let Some(eid) = Eid::from_oid(id) {
                match self.live(eid)? {
                    Some(true) => {}
                    Some(false) if lineage => {}
                    _ => return Err(Error::NotLive(eid)),
                }
            }
        }
        Ok(())
    }

    /// The pre-insert pipeline shared by assert, create, confirm and metadata.
    pub(crate) fn write(
        &mut self,
        s: ObjectId,
        p: ObjectId,
        o: ObjectId,
        valid: Valid,
        idempotent: bool,
        writer: Writer,
    ) -> Result<Asserted> {
        self.check_positions(s, p, o)?;
        let p_iri = self.iri_of(p)?.unwrap_or_default();
        if writer == Writer::User {
            reserved::check_predicate(&p_iri, s.tag()?)?;
        }
        valid.check()?;
        let flag = Flag::from_iri(&p_iri);
        match flag {
            Some(f) => {
                self.validate_flag(f, p, s, o)?;
                self.validate_schema_change(f, s, o, None)?;
            }
            None => {
                self.check_value_type(p, o)?;
                self.check_subject_type(p, s)?;
            }
        }
        let next = Eid::new(self.c.next_stmt as u64);
        if s == next.oid() || o == next.oid() {
            return Err(Error::SelfReference(next));
        }
        self.check_statement_references(s, p, o)?;
        if idempotent {
            if let Some(e) = self.find_overlapping(s, p, o, valid)? {
                return Ok(Asserted::Existing(e));
            }
        }
        self.unique_and_cardinality(s, p, o, valid, flag.is_some_and(Flag::single_valued))?;
        // cardinality replacement can retract an endpoint through its cascade
        self.check_statement_references(s, p, o)?;
        let eid = self.alloc_eid()?;
        if s == eid.oid() || o == eid.oid() {
            return Err(Error::SelfReference(eid));
        }
        self.insert_row(eid, s, p, o, valid)?;
        Ok(Asserted::New(eid))
    }

    /// Unique check then cardinality-one replacement for a statement about to be
    /// inserted. Single-valued flag predicates are implicitly `one` with valid time
    /// ignored.
    pub(crate) fn unique_and_cardinality(
        &mut self,
        s: ObjectId,
        p: ObjectId,
        o: ObjectId,
        valid: Valid,
        is_flag: bool,
    ) -> Result<()> {
        let schema = if is_flag {
            super::PredicateSchema {
                one: true,
                ..Default::default()
            }
        } else {
            self.schema(p)?
        };
        if schema.unique {
            // SQLite reads `live_pos` (p, o, s): the smallest other subject
            let mut holder: Option<i64> = None;
            for r in self.store.scan(
                Pattern::new(None, Some(p.raw()), Some(o.raw())),
                Scope::Live,
            )? {
                let r = r?;
                if r.s != s.raw() && holder.is_none_or(|h| r.s < h) {
                    holder = Some(r.s);
                }
            }
            if let Some(existing) = holder {
                return Err(Error::UniqueViolation {
                    p,
                    o,
                    existing: ObjectId::from_raw(existing),
                });
            }
        }
        if schema.one {
            let valid = if is_flag { Valid::ALWAYS } else { valid };
            let rows = self.live_rows(Some(s), Some(p), None)?;
            for r in rows {
                let rv = Valid {
                    from: r.v_from,
                    to: r.v_to,
                };
                if r.o == o.raw() || !rv.overlaps(&valid) {
                    continue;
                }
                if let Some(e) = Eid::from_oid(ObjectId::from_raw(r.eid)) {
                    self.retract_root(e, RetKind::Cardinality)?;
                }
            }
        }
        Ok(())
    }

    /// The smallest live eid with the same `(s, p, o)` and an overlapping interval.
    fn find_overlapping(
        &mut self,
        s: ObjectId,
        p: ObjectId,
        o: ObjectId,
        valid: Valid,
    ) -> Result<Option<Eid>> {
        Ok(self
            .live_rows(Some(s), Some(p), Some(o))?
            .into_iter()
            .find(|r| {
                Valid {
                    from: r.v_from,
                    to: r.v_to,
                }
                .overlaps(&valid)
            })
            .and_then(|r| Eid::from_oid(ObjectId::from_raw(r.eid))))
    }
}
