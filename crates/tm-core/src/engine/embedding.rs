//! Embedding writes: model definitions and vectors (OpenSpec change
//! `add-vector-recall`). Both run inside the write transaction, so a failed
//! transaction, a dry run or a speculation leaves no definition and no vector.

use super::ops::Writer;
use super::{IntoObject, Tx};
use crate::error::{Error, Position, Result};
use crate::exec::SqlValue;
use crate::id::{Eid, ObjectId, Tag};
use crate::report::Valid;
use crate::value::Value;
use crate::vector::{encode_vector, sign_bits, EmbeddingModel, Metric, VectorEncoding};
use crate::vocab;

impl Tx<'_> {
    fn model_id(&mut self, model: impl IntoObject) -> Result<ObjectId> {
        let m = model.into_object(self)?;
        if m.tag()? != Tag::Iri {
            return Err(Error::InvalidTerm {
                position: Position::Subject,
                reason: "an embedding model is named by an IRI".to_string(),
            });
        }
        Ok(m)
    }

    /// The object of `(model p ?)`, decoded, if any: the live statement, or with
    /// `ever` the latest one ever stored.
    fn model_value(&mut self, model: ObjectId, p: &str, ever: bool) -> Result<Option<Value>> {
        let Some(p) = self.sys_lookup(p)? else {
            return Ok(None);
        };
        let sql = if ever {
            "SELECT o FROM triple WHERE s = +?1 AND p = +?2 ORDER BY eid DESC LIMIT 1"
        } else {
            "SELECT o FROM triple WHERE s = +?1 AND p = +?2 AND t_ret IS NULL ORDER BY eid LIMIT 1"
        };
        let o = self.exec.query_i64(
            sql,
            &[SqlValue::Integer(model.raw()), SqlValue::Integer(p.raw())],
        )?;
        o.map(|o| self.decode(ObjectId::from_raw(o))).transpose()
    }

    /// The definition of `model` visible to this transaction, if it is defined.
    pub fn embedding_model(&mut self, model: impl IntoObject) -> Result<Option<EmbeddingModel>> {
        let m = self.model_id(model)?;
        self.model_def(m, false)
    }

    /// The live definition of `m`, or with `ever` the latest one ever stored.
    fn model_def(&mut self, m: ObjectId, ever: bool) -> Result<Option<EmbeddingModel>> {
        let Some(Value::Int(dims)) = self.model_value(m, vocab::SYS_DIMENSIONS, ever)? else {
            return Ok(None);
        };
        let metric = match self.model_value(m, vocab::SYS_METRIC, ever)? {
            Some(Value::Str(s)) => Metric::from_name(&s),
            _ => None,
        };
        let encoding = match self.model_value(m, vocab::SYS_VECTOR_ENCODING, ever)? {
            Some(Value::Str(s)) => VectorEncoding::from_name(&s),
            _ => None,
        };
        Ok(match (u32::try_from(dims).ok(), metric, encoding) {
            (Some(dimensions), Some(metric), Some(encoding)) => Some(EmbeddingModel {
                dimensions,
                metric,
                encoding,
            }),
            _ => None,
        })
    }

    /// Defines embedding model `model` (an IRI): asserts its `sys:dimensions`,
    /// `sys:metric` and `sys:vectorEncoding` statements. Returns true when the
    /// definition is new and false when the identical definition exists.
    ///
    /// Definitions are fixed for the life of the IRI: vectors already stored depend
    /// on them. A dropped model may be defined again only identically, which makes
    /// its stored vectors visible again. To change a definition, define a new model
    /// IRI.
    ///
    /// # Errors
    ///
    /// `InvalidQuery` for 0 or more than [`crate::vector::MAX_DIMENSIONS`]
    /// dimensions or a model that is, or ever was, defined differently;
    /// `InvalidTerm` when `model` is not an IRI.
    // @lat: [[query#Vector Recall#Embeddings And Models]]
    pub fn define_embedding_model(
        &mut self,
        model: impl IntoObject,
        def: &EmbeddingModel,
    ) -> Result<bool> {
        def.check()?;
        let m = self.model_id(model)?;
        if self.model_def(m, false)?.as_ref() == Some(def) {
            return Ok(false);
        }
        if let Some(existing) = self.model_def(m, true)? {
            if existing != *def {
                return Err(Error::invalid_query(format!(
                    "embedding model {} is, or was, defined as {} dimensions, {}, {}; \
                     define a new model IRI instead",
                    self.decode(m)?,
                    existing.dimensions,
                    existing.metric.name(),
                    existing.encoding.name()
                )));
            }
        }
        for (p, v) in [
            (vocab::SYS_DIMENSIONS, Value::Int(i64::from(def.dimensions))),
            (vocab::SYS_METRIC, Value::str(def.metric.name())),
            (vocab::SYS_VECTOR_ENCODING, Value::str(def.encoding.name())),
        ] {
            let p = self.sys(p)?;
            let o = self.dict.intern(self.exec, &v)?;
            self.write(m, p, o, Valid::ALWAYS, true, Writer::Engine)?;
        }
        Ok(true)
    }

    /// Drops embedding model `model`: retracts its live definition statements in
    /// one engine operation. Returns true when it was defined and false (writing
    /// nothing) when it was not.
    ///
    /// Stored embeddings are kept (they are append-only). The now view no longer
    /// sees the model, so storing and searching with it fail; views before the drop
    /// still see it. Defining it again identically restores it with its vectors.
    ///
    /// # Errors
    ///
    /// `InvalidTerm` when `model` is not an IRI.
    // @lat: [[query#Vector Recall#Embeddings And Models]]
    pub fn drop_embedding_model(&mut self, model: impl IntoObject) -> Result<bool> {
        let m = self.model_id(model)?;
        if self.model_def(m, false)?.is_none() {
            return Ok(false);
        }
        for p in [
            vocab::SYS_DIMENSIONS,
            vocab::SYS_METRIC,
            vocab::SYS_VECTOR_ENCODING,
        ] {
            if let Some(p) = self.sys_lookup(p)? {
                self.retract_matching(Some(m), Some(p), None)?;
            }
        }
        Ok(true)
    }

    /// Stores the embedding of live statement `eid` under `model`. The vector is
    /// checked against the model (dimensions, finite components), unit-normalised
    /// for cosine, and packed with its sign bits. Returns true when it is new and
    /// false when the identical vector is already stored.
    ///
    /// Embeddings are append-only: a statement has at most one vector per model.
    /// A superseded statement's replacement has a new eid and needs its own.
    ///
    /// # Errors
    ///
    /// [`Error::NotLive`] for a retracted or unknown statement, `InvalidQuery` for
    /// an undefined model or a vector that does not fit it, and
    /// [`Error::EmbeddingConflict`] for a different vector already stored.
    // @lat: [[query#Vector Recall#Embeddings And Models]]
    pub fn set_embedding(
        &mut self,
        eid: Eid,
        model: impl IntoObject,
        vector: &[f32],
    ) -> Result<bool> {
        eid.oid().check_origin()?;
        if self.live(eid)? != Some(true) {
            return Err(Error::NotLive(eid));
        }
        let m = self.model_id(model)?;
        let Some(def) = self.embedding_model(m)? else {
            return Err(Error::invalid_query(format!(
                "{} is not a defined embedding model (define it with define_embedding_model)",
                self.decode(m)?
            )));
        };
        let v = def.prepare(vector, "the embedding")?;
        let bytes = encode_vector(&v, def.encoding);
        let existing = self.exec.first_row(
            "SELECT vec FROM embedding WHERE model = +?1 AND eid = +?2",
            &[
                SqlValue::Integer(m.raw()),
                SqlValue::Integer(eid.oid().raw()),
            ],
        )?;
        if let Some(r) = existing {
            if r[0].as_blob() == Some(bytes.as_slice()) {
                return Ok(false);
            }
            return Err(Error::EmbeddingConflict {
                eid,
                model: self.decode(m)?.to_string(),
            });
        }
        self.exec.execute(
            "INSERT INTO embedding(model, eid, t, bits, vec) VALUES (?1, ?2, ?3, ?4, ?5)",
            &[
                SqlValue::Integer(m.raw()),
                SqlValue::Integer(eid.oid().raw()),
                SqlValue::Integer(self.t as i64),
                SqlValue::Blob(sign_bits(&v)),
                SqlValue::Blob(bytes),
            ],
        )?;
        Ok(true)
    }
}
