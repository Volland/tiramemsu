//! Embedding writes: model definitions and vectors. A port of
//! `tm_core::engine::embedding`. Both run inside the write transaction, so a failed
//! transaction, a dry run or a speculation leaves no definition and no vector.

use tm_core::vector::{encode_vector, sign_bits, EmbeddingModel, Metric, VectorEncoding};
use tm_core::vocab;
use tm_core::{Eid, Error, ObjectId, Position, Result, Tag, Valid, Value};

use super::ops::Writer;
use super::{IntoObject, Tx};
use crate::store::{EmbeddingRow, Pattern, Scope};

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
        let o = if ever {
            self.store
                .scan_eid(
                    Pattern::new(Some(model.raw()), Some(p.raw()), None),
                    Scope::All,
                )?
                .last()
                .map(|r| r.o)
        } else {
            self.live_rows(Some(model), Some(p), None)?
                .first()
                .map(|r| r.o)
        };
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
    /// `InvalidQuery` for 0 or more than [`tm_core::vector::MAX_DIMENSIONS`]
    /// dimensions or a model that is, or ever was, defined differently;
    /// `InvalidTerm` when `model` is not an IRI.
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
            let o = self.dict.intern(self.store, &v)?;
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
        if let Some(r) = self.store.embedding(m.raw(), eid.oid().raw())? {
            if r.vec == bytes {
                return Ok(false);
            }
            return Err(Error::EmbeddingConflict {
                eid,
                model: self.decode(m)?.to_string(),
            });
        }
        self.store.insert_embedding(&EmbeddingRow {
            model: m.raw(),
            eid: eid.oid().raw(),
            t: self.t as i64,
            bits: sign_bits(&v),
            vec: bytes,
        })?;
        Ok(true)
    }
}
