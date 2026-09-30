//! Tiramemsu query executor (`lat.md/query#Physical Planning`): turns a
//! [`tm_ir::IrQuery`] into one parameterised SQL statement over the bitemporal
//! `triple` table, runs it on the right connection and decodes the ObjectIds into
//! typed values.
//!
//! The pipeline is: validate → bind parameters → route (paths to the native
//! `tm_path` operator) → resolve views and encode constants on the executing
//! connection → normalise (Empty propagation) → generate SQL → run → decode.
//! `tm-exec` reaches SQLite only through `tm-core`'s [`tm_core::Executor`] and
//! needs the host capabilities `functions` and `vtab` ([`host`]).
//!
//! ```
//! use tiramemsu::{Db, OpenOptions, TxOptions, Valid, Value};
//! use tm_ir::builder::IrBuilder;
//!
//! let dir = tempfile::tempdir().unwrap();
//! let db = Db::open(dir.path().join("x.db"), OpenOptions::default())?;
//! let v = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
//! db.transact(TxOptions::default(), |tx| {
//!     tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?;
//!     Ok(())
//! })?;
//! let b = IrBuilder::sparql();
//! let q = b.query(b.triple("?a", "v:worksAt", "?c"));
//! let r = db.now().execute_ir(&q, &tm_ir::Params::new())?;
//! assert_eq!(r.get(0, "c"), Some(&v("acme")));
//! # Ok::<(), tiramemsu::Error>(())
//! ```
#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod decode;
pub mod error;
pub mod exec;
pub mod host;
pub mod native;
pub mod plan;
pub mod result;
pub mod scan;
pub mod sqlgen;
pub mod udf;
pub mod virtual_pred;

use std::sync::{Arc, Mutex};

use tm_core::{Executor, Result};
use tm_ir::validate::{output_vars, validate};
use tm_ir::{IrQuery, Params};

pub use decode::{CacheMode, TermCache};
pub use exec::{ExecContext, Prepared};
pub use native::{LftjConfig, NativeKind, NativeOperator, OperatorRegistry, PlannerOptions};
pub use result::{ExecStats, Explain, QueryResult, RegionInfo, RegionKind, ResultValue, RouteNote};

type Hook = Arc<dyn Fn() + Send + Sync>;

/// The query engine of one database: native operators, planner options and the
/// shared term cache.
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
    /// functions and native operators on it.
    pub fn install(&self, exec: &mut dyn Executor) -> Result<()> {
        host::install(exec, &self.registry)
    }

    /// Validates `q`, binds `params` and checks routing, without any I/O.
    pub fn prepare(&self, q: &IrQuery, params: &Params) -> Result<Prepared> {
        validate(q)?;
        let query = plan::bind::bind(q, params)?;
        plan::route::precheck(&query, &self.registry)?;
        let columns = output_vars(&query.root);
        Ok(Prepared { query, columns })
    }

    /// Runs a prepared query on `exec`.
    pub fn execute(
        &self,
        exec: &mut dyn Executor,
        cache: CacheMode,
        p: &Prepared,
    ) -> Result<QueryResult> {
        exec::run(self, ExecContext { exec, cache }, p)
    }

    /// Explains a prepared query on `exec` without running it.
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
