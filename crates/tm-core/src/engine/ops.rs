//! Statement operations: assert, create, retract, retract_matching, confirm, meta,
//! upsert.

use super::schema::Flag;
use super::{reserved, IntoObject, Tx};
use crate::error::{Error, Position, Result};
use crate::exec::{Params, SqlValue};
use crate::id::{Eid, ObjectId, Tag};
use crate::report::{AssertOpts, Asserted, OnExisting, RetKind, Valid};
use crate::vocab;

/// Who is writing: user writes are checked against the reserved namespace.
#[derive(Copy, Clone, PartialEq, Eq)]
pub(crate) enum Writer {
    User,
    Engine,
}

impl Tx<'_> {
    /// Asserts `(s, p, o)` valid over `valid`. Idempotent: returns
    /// `Existing(eid)` when a live statement with the same `(s, p, o)` and an
    /// overlapping valid interval exists.
    ///
    /// This is the default way to record a fact: asserting the same thing twice
    /// yields one statement. Use [`Tx::create`] when parallel edges are wanted. The
    /// returned [`Asserted`] carries the eid, which can then be the subject of
    /// further statements.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidInterval`], [`Error::ReservedNamespace`] for a `sys:`/`tm:`
    /// predicate a user may not write, [`Error::ValueTypeMismatch`] and
    /// [`Error::UniqueViolation`] from the predicate's schema.
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
    // @lat: [[time-model#Operations#Assert]]
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

    /// Records that this transaction corroborates live statement `eid`:
    /// asserts `(eid sys:confirmedBy tx)` and returns its eid.
    // @lat: [[time-model#Operations#Confirm]]
    pub fn confirm(&mut self, eid: Eid) -> Result<Eid> {
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
    // @lat: [[time-model#Operations#Unique Upsert]]
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
        let found = self.exec.query_i64(
            "SELECT s FROM triple WHERE p = ?1 AND o = ?2 AND t_ret IS NULL ORDER BY eid LIMIT 1",
            &[SqlValue::Integer(p.raw()), SqlValue::Integer(o.raw())],
        )?;
        if let Some(s) = found {
            return Ok(ObjectId::from_raw(s));
        }
        let node = self.new_node()?;
        self.assert(node, p, o, Valid::ALWAYS)?;
        Ok(node)
    }

    /// Retracts a live statement with cascade. Returns false for a retracted or
    /// unknown eid.
    ///
    /// This is how a fact is forgotten: the row stays and gets `t_ret`, so `as_of`
    /// and history views still show it. Every statement layered on it (its
    /// provenance, confidence, memberships) is retracted with it.
    ///
    /// # Errors
    ///
    /// [`Error::CascadeLimitExceeded`] when the cascade is larger than
    /// [`crate::TxOptions::max_cascade`].
    // @lat: [[time-model#Operations#Retract]]
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
        let mut params = Params::new();
        let mut sql = String::from("SELECT eid FROM triple WHERE t_ret IS NULL");
        for (col, v) in [("s", s), ("p", p), ("o", o)] {
            if let Some(v) = v {
                v.tag()?;
                let ph = params.push(v.raw());
                sql.push_str(&format!(" AND {col} = {ph}"));
            }
        }
        sql.push_str(" ORDER BY eid");
        let matched: Vec<Eid> = self
            .exec
            .rows(&sql, params.values())?
            .into_iter()
            .filter_map(|r| r[0].as_i64())
            .filter_map(|r| Eid::from_oid(ObjectId::from_raw(r)))
            .collect();
        for e in &matched {
            self.retract_root(*e, RetKind::Explicit)?;
        }
        Ok(matched)
    }

    /// `Some(true)` live, `Some(false)` retracted, `None` unknown.
    pub(crate) fn live(&mut self, eid: Eid) -> Result<Option<bool>> {
        Ok(self
            .exec
            .first_row(
                "SELECT t_ret FROM triple WHERE eid = ?1",
                &[SqlValue::Integer(eid.oid().raw())],
            )?
            .map(|r| r[0].is_null()))
    }

    /// The pre-insert pipeline shared by assert, create, confirm and metadata
    /// (design D-8): positions, reserved namespace, interval, value type or flag
    /// validation, schema-change validation, idempotency, unique, cardinality-one,
    /// allocation, self-reference, insert.
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
                self.validate_schema_change(f, s, o)?;
            }
            None => self.check_value_type(p, o)?,
        }
        if idempotent {
            if let Some(e) = self.find_overlapping(s, p, o, valid)? {
                return Ok(Asserted::Existing(e));
            }
        }
        self.unique_and_cardinality(s, p, o, valid, flag.is_some())?;
        let eid = self.alloc_eid();
        if s == eid.oid() || o == eid.oid() {
            return Err(Error::SelfReference(eid));
        }
        self.insert_row(eid, s, p, o, valid)?;
        Ok(Asserted::New(eid))
    }

    /// Unique check then cardinality-one replacement for a statement about to be
    /// inserted (design D-8 steps 5 and 6). Flag predicates are implicitly `one`
    /// with valid time ignored.
    // @lat: [[time-model#Operations#Cardinality One]]
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
            if let Some(existing) = self.exec.query_i64(
                "SELECT s FROM triple WHERE p = ?1 AND o = ?2 AND t_ret IS NULL AND s <> ?3 LIMIT 1",
                &[
                    SqlValue::Integer(p.raw()),
                    SqlValue::Integer(o.raw()),
                    SqlValue::Integer(s.raw()),
                ],
            )? {
                return Err(Error::UniqueViolation {
                    p,
                    o,
                    existing: ObjectId::from_raw(existing),
                });
            }
        }
        if schema.one {
            let valid = if is_flag { Valid::ALWAYS } else { valid };
            let rows = self.exec.rows(
                "SELECT eid FROM triple WHERE s = ?1 AND p = ?2 AND o <> ?3 AND t_ret IS NULL \
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
            for r in rows {
                if let Some(e) = r[0]
                    .as_i64()
                    .and_then(|r| Eid::from_oid(ObjectId::from_raw(r)))
                {
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
            .exec
            .query_i64(
                "SELECT eid FROM triple WHERE s = ?1 AND p = ?2 AND o = ?3 AND t_ret IS NULL \
                 AND (v_from IS NULL OR ?5 IS NULL OR v_from < ?5) \
                 AND (?4 IS NULL OR v_to IS NULL OR ?4 < v_to) ORDER BY eid LIMIT 1",
                &[
                    SqlValue::Integer(s.raw()),
                    SqlValue::Integer(p.raw()),
                    SqlValue::Integer(o.raw()),
                    SqlValue::from(valid.from),
                    SqlValue::from(valid.to),
                ],
            )?
            .and_then(|r| Eid::from_oid(ObjectId::from_raw(r))))
    }
}
