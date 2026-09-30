//! Tiramemsu SPARQL front end (`lat.md/query#Front Ends#SPARQL`): parses SPARQL 1.1
//! plus the SPARQL 1.2 triple-term, reifier and annotation syntax with `spargebra`
//! and lowers the algebra to the `tm-ir` logical IR.
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
#[derive(Clone, Debug)]
pub enum Prepared {
    /// A lowered query.
    Query(Box<QueryPlan>),
    /// A checked update request.
    Update(update::UpdatePlan),
}

/// Parses and lowers `text` against `env`.
pub fn prepare(text: &str, env: &Env) -> Result<Prepared> {
    match parse::parse(text, env)? {
        Parsed::Query(q) => Ok(Prepared::Query(Box::new(lower::lower_query_with(
            &q,
            env,
            parse::assoc_hints(text),
        )?))),
        Parsed::Update(u) => Ok(Prepared::Update(update::plan_update(
            &u,
            env,
            parse::assoc_hints(text),
        )?)),
    }
}
