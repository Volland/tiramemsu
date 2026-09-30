//! Variable scopes: kinds, the `dual_used` flag and shadowing (design Decision 4).

use std::collections::BTreeMap;

use crate::ast::Name;
use crate::error::{CResult, CypherError};
use crate::span::Span;

/// What a variable holds.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A node (or a statement in node form).
    Node,
    /// A relationship. `dual_used` is set when it also appears in node position.
    Rel {
        /// Used in node position somewhere.
        dual_used: bool,
    },
    /// A scalar, list or map.
    Value,
    /// A path.
    Path,
}

/// The variables visible at one point of a query.
#[derive(Clone, Debug, Default)]
pub struct Scope {
    vars: BTreeMap<String, Kind>,
}

impl Scope {
    /// An empty scope.
    pub fn new() -> Scope {
        Scope::default()
    }

    /// The kind of `name`.
    pub fn get(&self, name: &str) -> Option<Kind> {
        self.vars.get(name).copied()
    }

    /// Declares (or redeclares) `name`.
    pub fn set(&mut self, name: &str, k: Kind) {
        self.vars.insert(name.to_string(), k);
    }

    /// Variable names, sorted.
    pub fn names(&self) -> Vec<String> {
        self.vars.keys().cloned().collect()
    }

    /// True when empty.
    pub fn is_empty(&self) -> bool {
        self.vars.is_empty()
    }

    /// Binds a node-position use of `n`: a new node, or an existing node or
    /// relationship (which becomes `dual_used`).
    pub fn bind_node(&mut self, n: &Name) -> CResult<()> {
        match self.get(&n.text) {
            None => self.set(&n.text, Kind::Node),
            Some(Kind::Node) => {}
            Some(Kind::Rel { .. }) => self.set(&n.text, Kind::Rel { dual_used: true }),
            // a value computed by an expression may hold a node (checked when it runs)
            Some(Kind::Value) => {}
            Some(_) => {
                return Err(CypherError::parse(
                    n.span,
                    format!("variable `{}` is not a node", n.text),
                ))
            }
        }
        Ok(())
    }

    /// Binds a relationship-position use of `n`; a node variable is a `Parse` error.
    pub fn bind_rel(&mut self, n: &Name) -> CResult<()> {
        match self.get(&n.text) {
            None => self.set(&n.text, Kind::Rel { dual_used: false }),
            Some(Kind::Rel { .. }) | Some(Kind::Value) => {}
            Some(_) => {
                return Err(CypherError::parse(
                    n.span,
                    format!(
                        "variable `{}` was bound as a node and cannot be used as a relationship",
                        n.text
                    ),
                ))
            }
        }
        Ok(())
    }

    /// Declares a fresh variable that must not shadow anything (`AS x`, `UNWIND … AS x`).
    pub fn declare_value(&mut self, name: &str) {
        self.set(name, Kind::Value);
    }

    /// The `Parse` error for a reference to an undefined variable.
    pub fn undefined(name: &str, span: Span) -> CypherError {
        CypherError::parse(span, format!("variable `{name}` is not defined"))
    }
}
