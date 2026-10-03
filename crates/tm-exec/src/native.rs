//! The native-operator interface: operators that SQL reaches as table-valued
//! functions (`tm_path`, `tm_lftj`), their registry, and the planner options.

use std::sync::Arc;

use tm_core::{HostRegistry, Result};

/// The kind of a native operator.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum NativeKind {
    /// The path operator behind `tm_path(start, path, mode, max_hops, view)`,
    /// returning `(start, "end", hops, path_json)`.
    Path,
    /// The leapfrog-triejoin operator behind `tm_lftj(spec)`, returning `c0 … c31`
    /// ([`LftjOperator`](crate::LftjOperator)).
    Lftj,
}

/// A native operator. `add-path-engine` (M3) implements the `Path` kind.
///
/// Implement this to expose a custom algorithm to SQL as an eponymous
/// table-valued function. The planner routes only [`NativeKind`] shapes it
/// knows, so a new kind also needs planner support.
pub trait NativeOperator: Send + Sync {
    /// Its kind.
    fn kind(&self) -> NativeKind;
    /// The SQL name of its table-valued function (`"tm_path"`).
    fn tvf_name(&self) -> &'static str;
    /// Registers the eponymous virtual table through the host's `vtab` hook.
    fn register(&self, host: &mut dyn HostRegistry) -> Result<()>;
    /// The path engine behind a `Path` operator (`None` for other kinds and for test
    /// operators), so the facade can serve `View::path` without SQL.
    fn engine(&self) -> Option<&Arc<crate::path::PathEngine>> {
        None
    }
}

/// The registered native operators of one database.
///
/// # Example
///
/// ```
/// use tm_exec::OperatorRegistry;
///
/// let reg = OperatorRegistry::new();
/// assert!(reg.path().is_none() && reg.lftj().is_none());
/// ```
#[derive(Clone, Default)]
pub struct OperatorRegistry {
    ops: Vec<Arc<dyn NativeOperator>>,
}

impl std::fmt::Debug for OperatorRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<&str> = self.ops.iter().map(|o| o.tvf_name()).collect();
        f.debug_struct("OperatorRegistry")
            .field("ops", &names)
            .finish()
    }
}

impl OperatorRegistry {
    /// An empty registry.
    pub fn new() -> OperatorRegistry {
        OperatorRegistry::default()
    }

    /// Adds an operator (a later one of the same kind wins).
    pub fn add(&mut self, op: Arc<dyn NativeOperator>) {
        self.ops.retain(|o| o.kind() != op.kind());
        self.ops.push(op);
    }

    /// The path operator, if registered.
    pub fn path(&self) -> Option<&Arc<dyn NativeOperator>> {
        self.ops.iter().find(|o| o.kind() == NativeKind::Path)
    }

    /// The LFTJ operator, if registered.
    pub fn lftj(&self) -> Option<&Arc<dyn NativeOperator>> {
        self.ops.iter().find(|o| o.kind() == NativeKind::Lftj)
    }

    /// Registers every operator on one connection.
    pub fn register_all(&self, host: &mut dyn HostRegistry) -> Result<()> {
        self.ops.iter().try_for_each(|o| o.register(host))
    }
}

/// LFTJ routing (opt-in). A region goes to the native cyclic-join operator only
/// when all of these hold, and otherwise stays in SQL with the reason as its
/// [`RouteNote`](crate::RouteNote):
///
/// 1. `enabled` is set (else `CyclicLftjDisabled`);
/// 2. an LFTJ operator is registered (else `LftjUnavailable`; the `tiramemsu`
///    facade registers [`LftjOperator`](crate::LftjOperator) when `enabled`);
/// 3. the region is a cyclic join of stored-triple patterns only, with at most 32
///    output variables (else `LftjUnsupportedShape`);
/// 4. the estimate policy agrees: some pattern of the region matches at least
///    `min_rows_estimate` statements in its own view, counted with a capped scan
///    in the planning snapshot (else `LftjBelowEstimate`). `0` skips the estimate.
///
/// Results are identical on both routes; only speed differs.
///
/// # Example
///
/// ```
/// use tm_exec::{LftjConfig, PlannerOptions};
///
/// assert!(!LftjConfig::default().enabled);
/// let opts = PlannerOptions {
///     lftj: LftjConfig { enabled: true, min_rows_estimate: 0 },
/// };
/// assert!(opts.lftj.enabled);
/// ```
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct LftjConfig {
    /// Route pure cyclic BGPs to LFTJ when an operator exists and the estimate
    /// agrees (default false).
    pub enabled: bool,
    /// The estimate a region needs before LFTJ takes it: the largest number of
    /// statements one of its patterns matches (default 100 000; 0 = always).
    pub min_rows_estimate: u64,
}

impl Default for LftjConfig {
    fn default() -> Self {
        LftjConfig {
            enabled: false,
            min_rows_estimate: 100_000,
        }
    }
}

/// Planner options (`OpenOptions::planner`).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct PlannerOptions {
    /// LFTJ routing.
    pub lftj: LftjConfig,
}
