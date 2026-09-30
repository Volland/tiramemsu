//! The predicate schema: `sys:cardinality`, `sys:unique`, `sys:valueType`,
//! `sys:subjectType`, `sys:isEdge`.

use super::Tx;
use crate::error::{Error, Position, Result};
use crate::exec::SqlValue;
use crate::id::{Eid, ObjectId, Tag};
use crate::vocab;

/// The flags in force for one predicate.
///
/// # Example
///
/// A typed layer: `v:confidence` may annotate statements only.
///
/// ```
/// use tm_core::{vocab, Error, Store, StoreOptions, Tag, TxOptions, Valid, Value};
/// use tm_rusqlite as host;
///
/// # let dir = tempfile::tempdir().unwrap();
/// # let path = dir.path().join("db");
/// let mut store = Store::open(&host::RusqliteHost::new(), &path, StoreOptions::default())?;
/// let iri = |s: &str| Value::iri(vocab::v(s));
/// let stmt = Value::iri(vocab::tag_iri(Tag::Stmt));
/// store.transact(TxOptions::default(), |tx| {
///     tx.assert(iri("confidence"), Value::iri(vocab::SYS_SUBJECT_TYPE), stmt, Valid::ALWAYS)?;
///     let e1 = tx.assert(iri("alice"), iri("worksAt"), iri("acme"), Valid::ALWAYS)?.eid();
///     tx.assert(e1, iri("confidence"), Value::Double(0.8), Valid::ALWAYS)?;
///     let p = tx.encode(iri("confidence"))?;
///     assert_eq!(tx.schema(p)?.subject_types.len(), 1);
///     Ok(())
/// })?;
/// let node_level = store.transact(TxOptions::default(), |tx| {
///     tx.assert(iri("alice"), iri("confidence"), Value::Double(0.8), Valid::ALWAYS)
///         .map(|_| ())
/// });
/// assert!(matches!(node_level, Err(Error::SubjectTypeMismatch { got: Tag::Iri, .. })));
/// # Ok::<(), Error>(())
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PredicateSchema {
    /// `sys:cardinality sys:one`.
    pub one: bool,
    /// `sys:unique true`.
    pub unique: bool,
    /// `sys:valueType` object (a tag or datatype IRI).
    pub value_type: Option<ObjectId>,
    /// `sys:subjectType` objects (tag IRIs, any-of) in eid order; empty means any
    /// subject kind.
    pub subject_types: Vec<ObjectId>,
    /// `sys:isEdge` value.
    pub is_edge: Option<bool>,
}

/// One of the schema flags.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Flag {
    Cardinality,
    Unique,
    ValueType,
    SubjectType,
    IsEdge,
}

impl Flag {
    pub(crate) fn from_iri(iri: &str) -> Option<Flag> {
        match iri {
            vocab::SYS_CARDINALITY => Some(Flag::Cardinality),
            vocab::SYS_UNIQUE => Some(Flag::Unique),
            vocab::SYS_VALUE_TYPE => Some(Flag::ValueType),
            vocab::SYS_SUBJECT_TYPE => Some(Flag::SubjectType),
            vocab::SYS_IS_EDGE => Some(Flag::IsEdge),
            _ => None,
        }
    }

    /// True for a flag with one live value per predicate: a new value replaces the
    /// old one (implicit cardinality one). `sys:subjectType` values accumulate.
    pub(crate) fn single_valued(self) -> bool {
        self != Flag::SubjectType
    }
}

/// The tags a subject can have, and so the tags `sys:subjectType` may name.
const SUBJECT_TAGS: [Tag; 5] = [Tag::Iri, Tag::Node, Tag::BNode, Tag::Stmt, Tag::Tx];

const TRUE: ObjectId = ObjectId::from_raw((1 << 4) | Tag::Bool as i64);

/// The inline tags a datatype IRI matches (besides `TYPED` terms with that `dt`).
fn datatype_tags(iri: &str) -> &'static [Tag] {
    match iri {
        vocab::XSD_INTEGER => &[Tag::Int],
        vocab::XSD_STRING => &[Tag::ShortStr, Tag::Str],
        vocab::RDF_LANGSTRING => &[Tag::LangStr],
        vocab::XSD_BOOLEAN => &[Tag::Bool],
        vocab::XSD_DATE => &[Tag::Date],
        vocab::XSD_DATETIME => &[Tag::DateTime],
        vocab::XSD_DOUBLE => &[Tag::Double],
        vocab::XSD_DECIMAL => &[Tag::Decimal],
        _ => &[],
    }
}

// @lat: [[data-model#Predicate Schema]]
impl Tx<'_> {
    /// The schema of `p` at the current point of this transaction.
    pub fn schema(&mut self, p: ObjectId) -> Result<PredicateSchema> {
        if let Some(s) = self.schema_cache.get(&p) {
            return Ok(s.clone());
        }
        let ids = [
            self.sys_lookup(vocab::SYS_CARDINALITY)?,
            self.sys_lookup(vocab::SYS_UNIQUE)?,
            self.sys_lookup(vocab::SYS_VALUE_TYPE)?,
            self.sys_lookup(vocab::SYS_IS_EDGE)?,
            self.sys_lookup(vocab::SYS_SUBJECT_TYPE)?,
        ];
        let mut schema = PredicateSchema::default();
        if ids.iter().any(Option::is_some) {
            let one = self.sys_lookup(vocab::SYS_ONE)?;
            let raw = |o: Option<ObjectId>| SqlValue::Integer(o.map_or(0, ObjectId::raw));
            let rows = self.exec.rows(
                "SELECT p, o FROM triple WHERE s = ?1 AND t_ret IS NULL \
                 AND p IN (?2, ?3, ?4, ?5, ?6) ORDER BY eid",
                &[
                    SqlValue::Integer(p.raw()),
                    raw(ids[0]),
                    raw(ids[1]),
                    raw(ids[2]),
                    raw(ids[3]),
                    raw(ids[4]),
                ],
            )?;
            for r in rows {
                let fp = r[0].as_i64().map(ObjectId::from_raw);
                let o = ObjectId::from_raw(r[1].as_i64().unwrap_or(0));
                if fp == ids[0] {
                    schema.one = Some(o) == one;
                } else if fp == ids[1] {
                    schema.unique = o == TRUE;
                } else if fp == ids[2] {
                    schema.value_type = Some(o);
                } else if fp == ids[3] {
                    schema.is_edge = Some(o == TRUE);
                } else if fp == ids[4] {
                    schema.subject_types.push(o);
                }
            }
        }
        self.schema_cache.insert(p, schema.clone());
        Ok(schema)
    }

    /// True when `o` matches the value type `vt` (a tag IRI or a datatype IRI).
    pub(crate) fn matches_value_type(&mut self, vt: ObjectId, o: ObjectId) -> Result<bool> {
        let Some(iri) = self.iri_of(vt)? else {
            return Ok(false);
        };
        let otag = o.tag()?;
        if let Some(tag) = vocab::tag_from_iri(&iri) {
            return Ok(otag == tag);
        }
        if datatype_tags(&iri).contains(&otag) {
            return Ok(true);
        }
        if otag == Tag::Typed {
            let t = self.dict.term(self.exec, o.unsigned_payload() as i64)?;
            return Ok(t.and_then(|t| t.dt) == Some(vt));
        }
        Ok(false)
    }

    /// Checks `o` against the value type of `p`, if any.
    pub(crate) fn check_value_type(&mut self, p: ObjectId, o: ObjectId) -> Result<()> {
        if let Some(vt) = self.schema(p)?.value_type {
            if !self.matches_value_type(vt, o)? {
                return Err(Error::ValueTypeMismatch {
                    p,
                    expected: vt,
                    got: o.tag()?,
                });
            }
        }
        Ok(())
    }

    /// Checks the tag of subject `s` against the subject types of `p`, if any.
    pub(crate) fn check_subject_type(&mut self, p: ObjectId, s: ObjectId) -> Result<()> {
        let types = self.schema(p)?.subject_types;
        if types.is_empty() {
            return Ok(());
        }
        let got = s.tag()?;
        if self.subject_tags(&types)?.contains(&got) {
            return Ok(());
        }
        Err(Error::SubjectTypeMismatch {
            p,
            expected: types,
            got,
        })
    }

    /// The tags named by `sys:subjectType` objects.
    fn subject_tags(&mut self, types: &[ObjectId]) -> Result<Vec<Tag>> {
        let mut tags = Vec::with_capacity(types.len());
        for t in types {
            if let Some(tag) = self.iri_of(*t)?.and_then(|i| vocab::tag_from_iri(&i)) {
                tags.push(tag);
            }
        }
        Ok(tags)
    }

    /// Live `sys:subjectType` flag statements of predicate `x`, as `(eid, object)`.
    fn subject_type_rows(&mut self, x: ObjectId) -> Result<Vec<(Eid, ObjectId)>> {
        let Some(st) = self.sys_lookup(vocab::SYS_SUBJECT_TYPE)? else {
            return Ok(Vec::new());
        };
        let rows = self.exec.rows(
            "SELECT eid, o FROM triple WHERE s = ?1 AND p = ?2 AND t_ret IS NULL ORDER BY eid",
            &[SqlValue::Integer(x.raw()), SqlValue::Integer(st.raw())],
        )?;
        Ok(rows
            .into_iter()
            .filter_map(|r| {
                let e = Eid::from_oid(ObjectId::from_raw(r[0].as_i64()?))?;
                Some((e, ObjectId::from_raw(r[1].as_i64()?)))
            })
            .collect())
    }

    /// Live statements of predicate `x` whose subject tag is outside `tags`.
    fn subject_type_violations(&mut self, x: ObjectId, tags: &[Tag]) -> Result<Vec<Eid>> {
        let rows = self.exec.rows(
            "SELECT eid, s FROM triple WHERE p = ?1 AND t_ret IS NULL ORDER BY eid",
            &[SqlValue::Integer(x.raw())],
        )?;
        let mut out = Vec::new();
        for r in rows {
            let s = ObjectId::from_raw(r[1].as_i64().unwrap_or(0));
            if !tags.contains(&s.tag()?) {
                if let Some(e) = r[0]
                    .as_i64()
                    .and_then(|e| Eid::from_oid(ObjectId::from_raw(e)))
                {
                    out.push(e);
                }
            }
        }
        Ok(out)
    }

    /// Rejects retracting a `sys:subjectType` value when live data violates the
    /// narrowed set of the remaining values. Retracting the last value lifts the
    /// constraint; retracting any other statement is not a schema change here.
    pub(crate) fn validate_flag_retraction(&mut self, e: Eid) -> Result<()> {
        let Some(st) = self.sys_lookup(vocab::SYS_SUBJECT_TYPE)? else {
            return Ok(());
        };
        let Some(row) = self.exec.first_row(
            "SELECT s FROM triple WHERE eid = ?1 AND p = ?2 AND t_ret IS NULL",
            &[
                SqlValue::Integer(e.oid().raw()),
                SqlValue::Integer(st.raw()),
            ],
        )?
        else {
            return Ok(());
        };
        let x = ObjectId::from_raw(row[0].as_i64().unwrap_or(0));
        let rest: Vec<ObjectId> = self
            .subject_type_rows(x)?
            .into_iter()
            .filter(|(f, _)| *f != e)
            .map(|(_, o)| o)
            .collect();
        if rest.is_empty() {
            return Ok(());
        }
        let tags = self.subject_tags(&rest)?;
        let violating = self.subject_type_violations(x, &tags)?;
        if violating.is_empty() {
            return Ok(());
        }
        Err(Error::SchemaConflict { violating })
    }

    /// Built-in validation of a flag statement `(s flag o)`.
    pub(crate) fn validate_flag(
        &mut self,
        flag: Flag,
        fp: ObjectId,
        s: ObjectId,
        o: ObjectId,
    ) -> Result<()> {
        if s.tag()? != Tag::Iri {
            return Err(Error::InvalidTerm {
                position: Position::Subject,
                reason: "schema flags need an IRI subject".to_string(),
            });
        }
        let s_iri = self.iri_of(s)?.unwrap_or_default();
        if s_iri.starts_with(vocab::SYS) {
            return Err(Error::ReservedNamespace(s_iri));
        }
        let otag = o.tag()?;
        let (ok, expected) = match flag {
            Flag::Cardinality => {
                let iri = self.iri_of(o)?;
                (
                    matches!(iri.as_deref(), Some(vocab::SYS_ONE) | Some(vocab::SYS_MANY)),
                    Tag::Iri,
                )
            }
            Flag::Unique | Flag::IsEdge => (otag == Tag::Bool, Tag::Bool),
            Flag::ValueType => {
                let iri = self.iri_of(o)?;
                let ok = match iri.as_deref() {
                    Some(i) if i.starts_with(vocab::SYS) => vocab::tag_from_iri(i).is_some(),
                    Some(_) => true,
                    None => false,
                };
                (ok, Tag::Iri)
            }
            Flag::SubjectType => {
                let tag = self.iri_of(o)?.and_then(|i| vocab::tag_from_iri(&i));
                (tag.is_some_and(|t| SUBJECT_TAGS.contains(&t)), Tag::Iri)
            }
        };
        if !ok {
            let expected = self.sys(&vocab::tag_iri(expected))?;
            return Err(Error::ValueTypeMismatch {
                p: fp,
                expected,
                got: otag,
            });
        }
        Ok(())
    }

    /// Rejects a flag value that live data (including this transaction's writes)
    /// already violates. `replacing` is the flag statement a supersede retracts,
    /// which no longer counts towards the multi-valued `sys:subjectType`.
    pub(crate) fn validate_schema_change(
        &mut self,
        flag: Flag,
        x: ObjectId,
        o: ObjectId,
        replacing: Option<Eid>,
    ) -> Result<()> {
        let mut violating: Vec<Eid> = Vec::new();
        let push = |v: &mut Vec<Eid>, raw: Option<i64>| {
            if let Some(e) = raw.and_then(|r| Eid::from_oid(ObjectId::from_raw(r))) {
                v.push(e);
            }
        };
        match flag {
            Flag::Cardinality => {
                if self.iri_of(o)?.as_deref() != Some(vocab::SYS_ONE) {
                    return Ok(());
                }
                let rows = self.exec.rows(
                    "SELECT a.eid, b.eid FROM triple a JOIN triple b \
                       ON b.p = a.p AND b.s = a.s AND b.o <> a.o AND b.t_ret IS NULL \
                      AND (a.v_from IS NULL OR b.v_to IS NULL OR a.v_from < b.v_to) \
                      AND (b.v_from IS NULL OR a.v_to IS NULL OR b.v_from < a.v_to) \
                     WHERE a.p = ?1 AND a.t_ret IS NULL",
                    &[SqlValue::Integer(x.raw())],
                )?;
                for r in rows {
                    push(&mut violating, r[0].as_i64());
                    push(&mut violating, r[1].as_i64());
                }
            }
            Flag::Unique => {
                if o != TRUE {
                    return Ok(());
                }
                let rows = self.exec.rows(
                    "SELECT eid FROM triple WHERE p = ?1 AND t_ret IS NULL AND o IN \
                     (SELECT o FROM triple WHERE p = ?1 AND t_ret IS NULL \
                      GROUP BY o HAVING COUNT(DISTINCT s) > 1)",
                    &[SqlValue::Integer(x.raw())],
                )?;
                for r in rows {
                    push(&mut violating, r[0].as_i64());
                }
            }
            Flag::ValueType => {
                let rows = self.exec.rows(
                    "SELECT eid, o FROM triple WHERE p = ?1 AND t_ret IS NULL",
                    &[SqlValue::Integer(x.raw())],
                )?;
                for r in rows {
                    let obj = ObjectId::from_raw(r[1].as_i64().unwrap_or(0));
                    if !self.matches_value_type(o, obj)? {
                        push(&mut violating, r[0].as_i64());
                    }
                }
            }
            Flag::SubjectType => {
                // the resulting any-of set: the live values, minus a superseded one,
                // plus the new value
                let mut types: Vec<ObjectId> = self
                    .subject_type_rows(x)?
                    .into_iter()
                    .filter(|(e, _)| Some(*e) != replacing)
                    .map(|(_, t)| t)
                    .collect();
                types.push(o);
                let tags = self.subject_tags(&types)?;
                violating = self.subject_type_violations(x, &tags)?;
            }
            Flag::IsEdge => return Ok(()),
        }
        if violating.is_empty() {
            return Ok(());
        }
        violating.sort();
        violating.dedup();
        Err(Error::SchemaConflict { violating })
    }
}
