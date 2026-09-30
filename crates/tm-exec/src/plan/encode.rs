//! Constant encoding at plan time (design D4): inline through the codec, or a
//! read-only dictionary lookup on the executing connection. Planning never
//! inserts a term.

use tm_core::{Executor, ObjectId, Result, TermReader, Value};

/// The encoding of one constant.
#[derive(Clone, Debug, PartialEq)]
pub enum Enc {
    /// Its ObjectId (inline or found in the dictionary).
    Id(ObjectId),
    /// Not in the dictionary: no stored statement can hold it.
    Missing(Value),
    /// Its kind cannot occur in this position.
    Impossible,
}

/// A statement position.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Pos {
    /// Subject: IRI, node, blank node, statement or transaction.
    Subject,
    /// Predicate: an IRI.
    Predicate,
    /// Object: anything.
    Object,
}

/// Encodes a value (lookup only).
pub fn encode_value(exec: &mut dyn Executor, v: &Value) -> Result<Enc> {
    Ok(match TermReader::encode(exec, v)? {
        Some(id) => Enc::Id(id),
        None => Enc::Missing(v.canonical()),
    })
}

/// Checks that an ObjectId can occur in `pos`.
pub fn position_ok(id: ObjectId, pos: Pos) -> bool {
    match (id.tag(), pos) {
        (Err(_), _) => false,
        (Ok(_), Pos::Object) => true,
        (Ok(t), Pos::Subject) => t.is_subject(),
        (Ok(t), Pos::Predicate) => t == tm_core::Tag::Iri,
    }
}

/// Checks that a value's kind can occur in `pos` (before any lookup).
pub fn value_position_ok(v: &Value, pos: Pos) -> bool {
    let v = v.canonical();
    match pos {
        Pos::Object => true,
        Pos::Subject => matches!(
            v,
            Value::Iri(_) | Value::Node(_) | Value::BNode(_) | Value::Stmt(_) | Value::Tx(_)
        ),
        Pos::Predicate => matches!(v, Value::Iri(_)),
    }
}

/// Encodes a constant for a pattern position.
pub fn encode_at(exec: &mut dyn Executor, v: &Value, pos: Pos) -> Result<Enc> {
    if !value_position_ok(v, pos) {
        return Ok(Enc::Impossible);
    }
    encode_value(exec, v)
}
