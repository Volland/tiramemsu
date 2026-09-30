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
