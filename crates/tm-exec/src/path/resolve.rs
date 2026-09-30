//! Atom resolution: the letters of a DFA become dictionary ids for one call. A
//! predicate missing from the dictionary can match nothing (its letter is dead).

use tm_core::{Executor, ObjectId, Result, TermReader, Value};

use super::automaton::{Dfa, Dir, LetterKind};
use super::row::HopKind;

/// What one letter fetches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fetch {
    /// A predicate that is not in the dictionary: matches nothing.
    Dead,
    /// Statements of one predicate; `rv` filters by relationship-view membership.
    Pred {
        /// The predicate id.
        p: ObjectId,
        /// Required relationship-view membership, when the expression has a wildcard.
        rv: Option<bool>,
    },
    /// Relationship-view statements of every predicate except `excl`.
    Other {
        /// Predicates with a letter of their own in this direction (raw ids).
        excl: Vec<i64>,
    },
    /// A virtual layer hop.
    Virtual(HopKind),
}

/// A resolved letter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RLetter {
    /// What to fetch.
    pub fetch: Fetch,
    /// The direction.
    pub dir: Dir,
}

/// The resolved alphabet of a DFA.
#[derive(Clone, Debug)]
pub struct Resolved {
    /// One entry per DFA letter.
    pub letters: Vec<RLetter>,
    /// True when some letter needs the relationship-view filter.
    pub needs_rv: bool,
}

/// Resolves every letter of `dfa` against the dictionary on `exec`.
pub fn resolve(exec: &mut dyn Executor, dfa: &Dfa) -> Result<Resolved> {
    let mut enc = |iri: &str| TermReader::encode(exec, &Value::iri(iri));
    let mut letters = Vec::with_capacity(dfa.letters.len());
    let mut needs_rv = false;
    for l in &dfa.letters {
        let fetch = match &l.kind {
            LetterKind::Pred { iri, rv } => match enc(iri)? {
                Some(p) => {
                    needs_rv |= rv.is_some();
                    Fetch::Pred { p, rv: *rv }
                }
                None => Fetch::Dead,
            },
            LetterKind::Other => {
                needs_rv = true;
                let mut excl = Vec::new();
                for m in &dfa.letters {
                    if let (LetterKind::Pred { iri, .. }, true) = (&m.kind, m.dir == l.dir) {
                        if let Some(p) = enc(iri)? {
                            excl.push(p.raw());
                        }
                    }
                }
                excl.sort_unstable();
                excl.dedup();
                Fetch::Other { excl }
            }
            LetterKind::VSubject => Fetch::Virtual(HopKind::Subject),
            LetterKind::VObject => Fetch::Virtual(HopKind::Object),
            LetterKind::VPredicate => Fetch::Virtual(HopKind::Predicate),
        };
        letters.push(RLetter { fetch, dir: l.dir });
    }
    Ok(Resolved { letters, needs_rv })
}
