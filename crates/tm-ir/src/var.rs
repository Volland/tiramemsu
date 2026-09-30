//! Query variables.

use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;

/// A query variable, identified by its name (without the leading `?`).
///
/// Cloning is cheap: the name is reference counted.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Var(Arc<str>);

impl Var {
    /// A variable named `name` (a leading `?` or `$` is stripped).
    pub fn new(name: impl AsRef<str>) -> Var {
        let n = name.as_ref();
        let n = n
            .strip_prefix('?')
            .or_else(|| n.strip_prefix('$'))
            .unwrap_or(n);
        Var(Arc::from(n))
    }

    /// The variable name without `?`.
    pub fn name(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Var {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "?{}", self.0)
    }
}

impl fmt::Display for Var {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "?{}", self.0)
    }
}

impl From<&str> for Var {
    fn from(s: &str) -> Var {
        Var::new(s)
    }
}

/// An ordered set of variables.
pub type VarSet = BTreeSet<Var>;

/// The name prefix of provenance eid variables.
///
/// A [`TriplePattern`](crate::TriplePattern) whose `eid` is such a variable binds
/// the matched statement's eid but otherwise plans as if `eid` were `None`: under
/// `SetOfTriples` it still matches only one eid (the canonical one) of each visible
/// `(s, p, o)`, so binding it never changes the solutions. `~` cannot start a
/// SPARQL or Cypher variable, so user queries never produce one.
pub const PROVENANCE_PREFIX: &str = "~prov";

/// True for a provenance eid variable (see [`PROVENANCE_PREFIX`]).
///
/// ```
/// use tm_ir::var::{is_provenance, Var};
///
/// assert!(is_provenance(&Var::new("~prov3")));
/// assert!(!is_provenance(&Var::new("r")));
/// ```
pub fn is_provenance(v: &Var) -> bool {
    v.name().starts_with(PROVENANCE_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_strip_sigils() {
        assert_eq!(Var::new("?a"), Var::new("a"));
        assert_eq!(Var::new("$a").name(), "a");
        assert_eq!(Var::new("a").to_string(), "?a");
    }
}
