//! The volatile side table: high-churn state outside the graph.

use super::{IntoObject, Tx};
use crate::error::{Error, Position, Result};
use crate::exec::SqlValue;
use crate::id::{ObjectId, Tag};

// @lat: [[time-model#Never Forget]]
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
        self.exec.execute(
            "INSERT INTO volatile(s, key, value, updated_at) VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(s, key) DO UPDATE SET value = excluded.value, \
             updated_at = excluded.updated_at",
            &[
                SqlValue::Integer(s.raw()),
                SqlValue::Integer(key.raw()),
                SqlValue::Integer(value.raw()),
                SqlValue::Integer(self.instant),
            ],
        )?;
        Ok(())
    }

    /// Removes the volatile value of `(s, key)`; succeeds when there is none.
    pub fn clear_volatile(&mut self, s: impl IntoObject, key: impl IntoObject) -> Result<()> {
        let (s, key) = (s.into_object(self)?, key.into_object(self)?);
        self.volatile_key(s, key)?;
        self.exec.execute(
            "DELETE FROM volatile WHERE s = ?1 AND key = ?2",
            &[SqlValue::Integer(s.raw()), SqlValue::Integer(key.raw())],
        )?;
        Ok(())
    }
}
