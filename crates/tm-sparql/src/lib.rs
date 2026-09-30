#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod construct;
pub mod dataset;
pub mod env;
pub mod error;
pub mod lower;
pub mod parse;
pub mod results;
pub mod terms;
pub mod update;

use tm_core::Result;

use crate::env::Env;
use crate::lower::QueryPlan;
use crate::parse::Parsed;

/// A prepared request: a lowered query or a checked update.
///
/// The variant follows the text: `SELECT`, `ASK` and `CONSTRUCT` give
/// [`Prepared::Query`]; `INSERT`, `DELETE`, `CREATE`, `CLEAR` and `DROP` give
/// [`Prepared::Update`].
#[derive(Clone, Debug)]
pub enum Prepared {
    /// A lowered query.
    Query(Box<QueryPlan>),
    /// A checked update request.
    Update(update::UpdatePlan),
}

/// Parses, checks and lowers `text` against `env`, without touching a database.
///
/// Use this when you want the IR (to inspect, cache or run on your own
/// executor). With the `tiramemsu` facade, `View::sparql` calls it for you.
/// Prepared plans are values: `env` is read only during this call, so a plan
/// keeps the view, vocabulary and prefixes it was prepared with.
///
/// # Errors
///
/// - `Error::Parse` (dialect SPARQL, with a line and column when known) for
///   invalid syntax, an undeclared prefix, a relative IRI or a malformed `tm:`
///   time IRI.
/// - `Error::Unsupported { feature }` for a construct outside the v1 subset. The
///   feature names are the constants of [`error`], for example
///   [`error::DESCRIBE`], and `ADD`, `MOVE`, `COPY` and `LOAD` are named
///   before anything else in an update is checked.
///
/// # Example
///
/// ```
/// use tm_ir::View;
/// use tm_sparql::{env::Env, prepare, Prepared};
///
/// let env = Env::new(View::NOW);
/// assert!(matches!(
///     prepare("ASK { v:alice v:worksAt v:acme }", &env)?,
///     Prepared::Query(_)
/// ));
/// assert!(matches!(
///     prepare("INSERT DATA { v:alice v:worksAt v:acme }", &env)?,
///     Prepared::Update(_)
/// ));
/// // an error names the problem
/// assert!(prepare("SELECT ?x WHERE {", &env).is_err());
/// # Ok::<(), tm_core::Error>(())
/// ```
pub fn prepare(text: &str, env: &Env) -> Result<Prepared> {
    match parse::parse(text, env)? {
        Parsed::Query(q) => Ok(Prepared::Query(Box::new(lower::lower_query_with(
            &q,
            env,
            parse::assoc_hints(text),
        )?))),
        Parsed::Update(u) => {
            // ADD, MOVE, COPY and LOAD are named before anything else is checked
            if let Some(kw) = parse::operation_keywords(text)
                .into_iter()
                .find(|k| matches!(k.as_str(), "LOAD" | "ADD" | "MOVE" | "COPY"))
            {
                return Err(error::unsupported(kw));
            }
            Ok(Prepared::Update(update::plan_update(
                &u,
                env,
                parse::assoc_hints(text),
            )?))
        }
    }
}
