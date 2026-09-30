//! The compiled program and the compile entry point (design Decision 3).

use std::collections::{BTreeMap, BTreeSet};

use crate::ast::Query;
use crate::error::CResult;
use crate::parse::adapter;
use crate::sema::check::{self, CheckOpts};
use crate::value::CypherValue;
use crate::vocab::Vocab;

/// Query parameters: `$name` to value. Compile-time checking only needs the
/// names; the values are read at run time.
pub type CypherParams = BTreeMap<String, CypherValue>;

/// A read-only snapshot taken at compile time: the vocabulary, the handle's default
/// view, and whether writes are allowed.
///
/// The facade fills it from the live database. Set `writable: false` for a
/// read handle, so a query with a write clause fails at compile time before
/// anything runs. The vocabulary is fixed here, so names resolve the same way
/// even under `USE AS OF`.
#[derive(Clone, Debug)]
pub struct CompileCtx {
    /// `@vocab` and the prefix table.
    pub vocab: Vocab,
    /// The handle's view, the default of every pattern.
    pub view: tm_ir::View,
    /// True for a write handle.
    pub writable: bool,
}

/// A checked query, ready to run with [`exec::run`](crate::exec::run).
///
/// Holds the original text (for error spans), the AST and the compile-time
/// snapshot. It is cheap to clone and can be run more than once.
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

/// Parses, checks and packages a query, without touching a store.
///
/// Checks scopes, kinds, aggregates, parameters (every `$name` must be a key of
/// `params`) and, when `ctx.writable` is false, that there is no write clause.
///
/// # Errors
///
/// [`CypherError::Parse`](crate::CypherError::Parse) with the byte span of the
/// problem in `text` (extensions such as `USE` included);
/// [`CypherError::Unsupported`](crate::CypherError::Unsupported) for a construct
/// outside the v1 subset.
///
/// # Example
///
/// ```
/// use tm_cypher::{compile, CompileCtx, CypherError, CypherParams, Vocab};
/// use tm_ir::View;
///
/// let ctx = CompileCtx { vocab: Vocab::default(), view: View::NOW, writable: true };
/// let prog = compile("MATCH (n) SET n.seen = true", &CypherParams::new(), &ctx)?;
/// assert!(prog.has_write);
///
/// // FOREACH is outside v1 and says so
/// let err = compile("FOREACH (x IN [1] | CREATE ())", &CypherParams::new(), &ctx).unwrap_err();
/// assert!(matches!(err, CypherError::Unsupported { .. }));
/// # Ok::<(), CypherError>(())
/// ```
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
