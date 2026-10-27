//! The volatile side table: high-churn state outside the graph. A port of
//! `tm_core::engine::volatile`.

use tm_core::{Error, ObjectId, Position, Result, Tag};

use super::{IntoObject, Tx};
use crate::store::VolatileRow;

impl Tx<'_> {
    fn volatile_key(&mut self, s: ObjectId, key: ObjectId) -> Result<()> {
        if !s.tag()?.is_subject() {
            return Err(Error::InvalidTerm {
                position: Position::Subject,
                reason: "volatile subject must be subject-capable".to_string(),
            });
        }
        if key.tag()? != Tag::Iri {
            return Err(Error::InvalidTerm {
                position: Position::Key,
                reason: "volatile key must be an IRI".to_string(),
            });
        }
        self.check_known(s, Position::Subject)?;
        self.check_known(key, Position::Key)
    }

    /// Upserts the volatile value of `(s, key)`, with `updated_at` = this instant.
    pub fn set_volatile(
        &mut self,
        s: impl IntoObject,
        key: impl IntoObject,
        value: impl IntoObject,
    ) -> Result<()> {
        let (s, key, value) = (
            s.into_object(self)?,
            key.into_object(self)?,
            value.into_object(self)?,
        );
        self.volatile_key(s, key)?;
        self.check_known(value, Position::Value)?;
        self.store.put_volatile(
            s.raw(),
            key.raw(),
            VolatileRow {
                value: value.raw(),
                updated_at: self.instant,
            },
        )
    }

    /// Removes the volatile value of `(s, key)`; succeeds when there is none.
    pub fn clear_volatile(&mut self, s: impl IntoObject, key: impl IntoObject) -> Result<()> {
        let (s, key) = (s.into_object(self)?, key.into_object(self)?);
        self.volatile_key(s, key)?;
        self.store.delete_volatile(s.raw(), key.raw())?;
        Ok(())
    }
}
