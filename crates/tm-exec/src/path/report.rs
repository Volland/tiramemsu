//! The completeness report of the path searches of one operation
//! (`lat.md/query#Physical Planning#Path Engine#Path Completeness`).
//!
//! A SPARQL or Cypher query runs its path regions inside SQL, one `tm_path` call
//! per input row, so the verdict cannot come back with the rows. Each search
//! records its [`PathCompleteness`] in a thread-local slot instead; [`collect`]
//! opens the slot around an operation and returns the merged verdict. Calls run
//! on the thread that steps the statement, so the slot sees all of them.

use std::cell::RefCell;

use tm_ir::PathCompleteness;

thread_local! {
    /// `Some(verdict so far)` inside a [`collect`]; `None` outside.
    static CURRENT: RefCell<Option<Option<PathCompleteness>>> = const { RefCell::new(None) };
}

/// Restores the enclosing slot when a scope ends (also on unwinding), merging the
/// inner verdict into it.
struct Restore(Option<Option<PathCompleteness>>);

impl Drop for Restore {
    fn drop(&mut self) {
        let outer = self.0.take();
        CURRENT.with(|c| {
            let mut c = c.borrow_mut();
            let inner = c.take().flatten();
            *c = outer.map(|o| merge(o, inner));
        });
    }
}

fn merge(a: Option<PathCompleteness>, b: Option<PathCompleteness>) -> Option<PathCompleteness> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.merge(b)),
        (a, b) => a.or(b),
    }
}

/// Runs `f` and returns its result with the merged completeness of every path
/// search it ran (`None` when it ran none). Scopes nest: an inner scope's verdict
/// also counts for the enclosing one.
///
/// ```
/// use tm_exec::path::report::{collect, record};
/// use tm_ir::PathCompleteness;
///
/// let ((), verdict) = collect(|| {
///     record(PathCompleteness::Exhaustive);
///     record(PathCompleteness::StoppedAtCap { max_hops: 15 });
/// });
/// assert_eq!(verdict, Some(PathCompleteness::StoppedAtCap { max_hops: 15 }));
/// assert_eq!(collect(|| ()).1, None);
/// ```
pub fn collect<R>(f: impl FnOnce() -> R) -> (R, Option<PathCompleteness>) {
    let outer = CURRENT.with(|c| c.borrow_mut().replace(None));
    let restore = Restore(outer);
    let r = f();
    let got = CURRENT.with(|c| c.borrow().flatten());
    drop(restore);
    (r, got)
}

/// Records the completeness of one search in the current [`collect`] scope; a
/// no-op outside one.
pub fn record(c: PathCompleteness) {
    CURRENT.with(|cur| {
        if let Some(slot) = cur.borrow_mut().as_mut() {
            *slot = merge(*slot, Some(c));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    // @lat: [[tests#Temporal Path Syntax#Completeness Report Scopes]]
    #[test]
    fn nested_scopes_merge_outward() {
        let (inner, outer) = collect(|| {
            record(PathCompleteness::StoppedAtBound { max_hops: 3 });
            collect(|| record(PathCompleteness::StoppedAtCap { max_hops: 15 })).1
        });
        assert_eq!(inner, Some(PathCompleteness::StoppedAtCap { max_hops: 15 }));
        assert_eq!(outer, Some(PathCompleteness::StoppedAtCap { max_hops: 15 }));
        // outside a scope nothing is kept
        record(PathCompleteness::Exhaustive);
        assert_eq!(collect(|| ()).1, None);
    }
}
