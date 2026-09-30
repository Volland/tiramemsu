//! Internal variables: fresh names that SPARQL text cannot express, and the
//! filter that hides them from `SELECT *`.

use tm_ir::Var;

/// The prefix of every internal variable. `~` is not a legal character of a
/// SPARQL variable name, so an internal name never collides with a user variable.
pub const INTERNAL_PREFIX: &str = "~";

/// Allocates fresh internal variables.
#[derive(Debug, Default, Clone)]
pub struct VarAlloc {
    next: u32,
}

impl VarAlloc {
    /// A fresh allocator.
    pub fn new() -> VarAlloc {
        VarAlloc::default()
    }

    /// A fresh internal variable named `~<kind><n>` (`kind` is a short mnemonic:
    /// `b` blank node, `e` eid, `r` reifier).
    pub fn fresh(&mut self, kind: &str) -> Var {
        let v = Var::new(format!("{INTERNAL_PREFIX}{kind}{}", self.next));
        self.next += 1;
        v
    }
}

/// True for variables the lowering introduced.
pub fn is_internal(v: &Var) -> bool {
    v.name().starts_with(INTERNAL_PREFIX)
}

/// The variables of `vars` that a user can see, in order.
pub fn visible(vars: &[Var]) -> Vec<Var> {
    vars.iter().filter(|v| !is_internal(v)).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_names_are_unique_and_internal() {
        let mut a = VarAlloc::new();
        let x = a.fresh("b");
        let y = a.fresh("b");
        assert_ne!(x, y);
        assert!(is_internal(&x));
        assert!(!is_internal(&Var::new("b0")));
        assert_eq!(visible(&[x, Var::new("s")]), vec![Var::new("s")]);
    }
}
