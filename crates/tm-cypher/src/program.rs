//! The compiled program and the compile entry point (design Decision 3).

use std::collections::{BTreeMap, BTreeSet};

use crate::ast::Query;
use crate::error::CResult;
use crate::parse::adapter;
use crate::sema::check::{self, CheckOpts};
use crate::value::CypherValue;
use crate::vocab::Vocab;

/// Query parameters.
pub type CypherParams = BTreeMap<String, CypherValue>;

/// A read-only snapshot taken at compile time: the vocabulary, the handle's default
/// view, and whether writes are allowed.
#[derive(Clone, Debug)]
pub struct CompileCtx {
    /// `@vocab` and the prefix table.
    pub vocab: Vocab,
    /// The handle's view, the default of every pattern.
    pub view: tm_ir::View,
    /// True for a write handle.
    pub writable: bool,
}

/// A checked query, ready to run.
#[derive(Clone, Debug)]
pub struct CypherProgram {
    /// The original text.
    pub text: String,
    /// The AST.
    pub query: Query,
    /// The compile-time snapshot.
    pub ctx: CompileCtx,
    /// True when the query has a write clause.
    pub has_write: bool,
}

/// Parses, checks and packages a query.
pub fn compile(text: &str, params: &CypherParams, ctx: &CompileCtx) -> CResult<CypherProgram> {
    let query = adapter::parse(text)?;
    let names: BTreeSet<String> = params.keys().cloned().collect();
    check::check(
        &query,
        &CheckOpts {
            params: &names,
            writable: ctx.writable,
        },
    )?;
    let has_write = check::first_write(&query).is_some();
    Ok(CypherProgram {
        text: text.to_string(),
        query,
        ctx: ctx.clone(),
        has_write,
    })
}
