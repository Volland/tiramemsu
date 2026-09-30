//! Pattern positions: a variable, a constant value, a raw ObjectId or a parameter.

use tm_core::{ObjectId, Value};

use crate::var::Var;

/// A position of a pattern, a Values cell, or skip/limit.
#[derive(Clone, Debug, PartialEq)]
pub enum TermOrVar {
    /// A variable.
    Var(Var),
    /// A constant value, encoded to an ObjectId at plan time.
    Const(Value),
    /// An already encoded ObjectId (for example an eid returned earlier).
    Id(ObjectId),
    /// A named parameter, resolved from the execution's parameter map.
    Param(String),
}

impl TermOrVar {
    /// The variable, if this is one.
    pub fn as_var(&self) -> Option<&Var> {
        match self {
            TermOrVar::Var(v) => Some(v),
            _ => None,
        }
    }

    /// A variable position.
    pub fn var(name: &str) -> TermOrVar {
        TermOrVar::Var(Var::new(name))
    }

    /// An IRI constant.
    pub fn iri(iri: impl Into<String>) -> TermOrVar {
        TermOrVar::Const(Value::Iri(iri.into()))
    }

    /// A parameter position.
    pub fn param(name: &str) -> TermOrVar {
        TermOrVar::Param(name.trim_start_matches('$').to_string())
    }
}

impl From<Var> for TermOrVar {
    fn from(v: Var) -> TermOrVar {
        TermOrVar::Var(v)
    }
}

impl From<Value> for TermOrVar {
    fn from(v: Value) -> TermOrVar {
        TermOrVar::Const(v)
    }
}

impl From<ObjectId> for TermOrVar {
    fn from(v: ObjectId) -> TermOrVar {
        TermOrVar::Id(v)
    }
}

impl From<tm_core::Eid> for TermOrVar {
    fn from(v: tm_core::Eid) -> TermOrVar {
        TermOrVar::Id(v.oid())
    }
}

impl From<&str> for TermOrVar {
    /// `"?x"` is a variable, `"$x"` a parameter, anything else an IRI.
    fn from(s: &str) -> TermOrVar {
        if s.starts_with('?') {
            TermOrVar::Var(Var::new(s))
        } else if let Some(p) = s.strip_prefix('$') {
            TermOrVar::Param(p.to_string())
        } else {
            TermOrVar::Const(Value::Iri(s.to_string()))
        }
    }
}
