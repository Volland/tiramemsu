//! The environment a SPARQL text is prepared against.

use tm_core::vocab;
use tm_ir::View;

/// What [`prepare`](crate::prepare) needs to know about the caller's view and database.
///
/// The facade builds one per call from the live database settings. Build your own
/// when you prepare text outside a `tiramemsu::Db`: start from [`Env::new`] and set
/// `vocab`, `prefixes`, `speculative` or `now_ms` as needed.
///
/// # Example
///
/// ```
/// use tm_ir::View;
/// use tm_sparql::{env::Env, prepare};
///
/// let mut env = Env::new(View::NOW);
/// env.prefixes.push(("ex".into(), "https://example.org/".into()));
/// // `ex:` needs no PREFIX line, and `v:` is the default vocabulary
/// prepare("SELECT ?x WHERE { ?x ex:knows v:alice }", &env)?;
/// assert!(env.is_current());
/// # Ok::<(), tm_core::Error>(())
/// ```
#[derive(Clone, Debug)]
pub struct Env {
    /// The view the text was submitted on (the default of every pattern).
    pub base_view: View,
    /// The database `@vocab` IRI, the meaning of `v:`.
    pub vocab: String,
    /// The database prefix table, `(name, iri)`.
    pub prefixes: Vec<(String, String)>,
    /// True inside a speculative transaction.
    pub speculative: bool,
    /// The wall-clock start of the query in epoch milliseconds (`NOW()`).
    pub now_ms: i64,
}

impl Env {
    /// The default environment on `base_view`: the default vocabulary, no extra
    /// prefixes, not speculative, `NOW()` fixed at `now_ms`.
    pub fn new(base_view: View) -> Env {
        Env {
            base_view,
            vocab: vocab::V.to_string(),
            prefixes: Vec::new(),
            speculative: false,
            now_ms: 0,
        }
    }

    /// True when updates are allowed: the plain current view, not speculative.
    pub fn is_current(&self) -> bool {
        self.base_view == View::NOW && !self.speculative
    }
}
