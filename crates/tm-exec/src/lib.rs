#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod decode;
pub mod error;
pub mod exec;
pub mod host;
pub mod native;
pub mod path;
pub mod plan;
pub mod result;
pub mod scan;
pub mod sqlgen;
pub mod udf;
pub mod udf_fn;
pub mod virtual_pred;

use std::sync::{Arc, Mutex};

use tm_core::{Executor, Result};
use tm_ir::validate::{output_vars, validate};
use tm_ir::{IrQuery, Params};

pub use decode::{CacheMode, TermCache};
pub use exec::{ExecContext, Prepared};
pub use native::{LftjConfig, NativeKind, NativeOperator, OperatorRegistry, PlannerOptions};
pub use path::{Hop, Path, PathEngine, PathOperator, PathOptions, PathRequest, PathRow};
pub use result::{ExecStats, Explain, QueryResult, RegionInfo, RegionKind, ResultValue, RouteNote};

type Hook = Arc<dyn Fn() + Send + Sync>;

/// The query engine of one database: native operators, planner options and the
/// shared term cache.
///
/// One engine serves every connection of a database. The usual flow is
/// [`install`](QueryEngine::install) once per connection, then
/// [`prepare`](QueryEngine::prepare) a query (pure, no I/O) and
/// [`execute`](QueryEngine::execute) or [`explain`](QueryEngine::explain) it on
/// a connection inside a read transaction. The `tiramemsu` facade does all of
/// this for you.
///
/// # Example
///
/// ```
/// use tm_exec::{OperatorRegistry, PlannerOptions, QueryEngine};
/// use tm_ir::{builder::IrBuilder, Params};
///
/// let engine = QueryEngine::new(PlannerOptions::default(), OperatorRegistry::new(), 1024);
/// let b = IrBuilder::sparql();
/// let p = engine.prepare(&b.query(b.triple("?s", "v:p", "?o")), &Params::new())?;
/// assert_eq!(p.columns().len(), 2);
/// // A `$name` that was not supplied is rejected before any SQL exists.
/// let q = b.query(b.triple("?s", "v:p", "$missing"));
/// assert!(engine.prepare(&q, &Params::new()).is_err());
/// # Ok::<(), tm_core::Error>(())
/// ```
pub struct QueryEngine {
    registry: OperatorRegistry,
    options: PlannerOptions,
    cache: TermCache,
    hook: Mutex<Option<Hook>>,
}

impl std::fmt::Debug for QueryEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QueryEngine")
            .field("registry", &self.registry)
            .field("options", &self.options)
            .finish()
    }
}

impl QueryEngine {
    /// An engine with these operators, options and term-cache capacity.
    pub fn new(
        options: PlannerOptions,
        registry: OperatorRegistry,
        term_cache_capacity: usize,
    ) -> QueryEngine {
        QueryEngine {
            registry,
            options,
            cache: TermCache::new(term_cache_capacity),
            hook: Mutex::new(None),
        }
    }

    /// Checks the host capabilities of a connection and registers the SQL helper
    /// functions and native operators on it. Call it once per connection, before
    /// running queries on it.
    ///
    /// # Errors
    ///
    /// `MissingCapability` if the host lacks `functions` or `vtab`.
    pub fn install(&self, exec: &mut dyn Executor) -> Result<()> {
        host::install(exec, &self.registry)
    }

    /// Validates `q`, binds `params` and checks routing, without any I/O.
    ///
    /// # Errors
    ///
    /// `InvalidQuery` for structurally invalid IR or a missing parameter, and
    /// `Unsupported` for a path pattern when no path operator is registered.
    pub fn prepare(&self, q: &IrQuery, params: &Params) -> Result<Prepared> {
        validate(q)?;
        let query = plan::bind::bind(q, params)?;
        plan::route::precheck(&query, &self.registry)?;
        // graph joins add internal variables to the plan, never to the result
        let columns = output_vars(&q.root);
        Ok(Prepared { query, columns })
    }

    /// Runs a prepared query on `exec`.
    ///
    /// Call it inside a read transaction so planning, the statement and
    /// decoding share one snapshot. Use [`CacheMode::Shared`] on a reader and
    /// [`CacheMode::Scoped`] on the writer inside a speculation.
    pub fn execute(
        &self,
        exec: &mut dyn Executor,
        cache: CacheMode,
        p: &Prepared,
    ) -> Result<QueryResult> {
        exec::run(self, ExecContext { exec, cache }, p)
    }

    /// Explains a prepared query on `exec` without running it: routing, SQL
    /// text, bound parameters and `EXPLAIN QUERY PLAN`. Use it to check that a
    /// query uses the index you expect.
    pub fn explain(&self, exec: &mut dyn Executor, p: &Prepared) -> Result<Explain> {
        exec::explain(self, exec, p)
    }

    /// The shared term cache.
    pub fn term_cache(&self) -> &TermCache {
        &self.cache
    }

    /// The registered native operators.
    pub fn registry(&self) -> &OperatorRegistry {
        &self.registry
    }

    /// The path engine, when the registered path operator has one.
    pub fn path_engine(&self) -> Option<&Arc<PathEngine>> {
        self.registry.path().and_then(|o| o.engine())
    }

    /// The planner options.
    pub fn options(&self) -> &PlannerOptions {
        &self.options
    }

    /// Test hook run after planning and before the SQL statement (snapshot tests).
    #[doc(hidden)]
    pub fn set_test_hook(&self, f: Option<Arc<dyn Fn() + Send + Sync>>) {
        *self.hook.lock().unwrap_or_else(|p| p.into_inner()) = f;
    }

    pub(crate) fn run_hook(&self) {
        let h = self.hook.lock().unwrap_or_else(|p| p.into_inner()).clone();
        if let Some(h) = h {
            h();
        }
    }
}
