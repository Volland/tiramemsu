//! The native-operator interface: operators that SQL reaches as table-valued
//! functions (`tm_path`, later LFTJ), their registry, and the planner options.

use std::sync::Arc;

use tm_core::{HostRegistry, Result};

/// The kind of a native operator.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum NativeKind {
    /// The path operator behind `tm_path(start, path, mode, max_hops, view)`,
    /// returning `(start, "end", hops, path_json)`.
    Path,
    /// A leapfrog-triejoin operator (M4).
    Lftj,
}

/// A native operator. `add-path-engine` (M3) implements the `Path` kind.
pub trait NativeOperator: Send + Sync {
    /// Its kind.
    fn kind(&self) -> NativeKind;
    /// The SQL name of its table-valued function (`"tm_path"`).
    fn tvf_name(&self) -> &'static str;
    /// Registers the eponymous virtual table through the host's `vtab` hook.
    fn register(&self, host: &mut dyn HostRegistry) -> Result<()>;
}

/// The registered native operators of one database.
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

    /// The LFTJ operator, if registered (never in M1).
    pub fn lftj(&self) -> Option<&Arc<dyn NativeOperator>> {
        self.ops.iter().find(|o| o.kind() == NativeKind::Lftj)
    }

    /// Registers every operator on one connection.
    pub fn register_all(&self, host: &mut dyn HostRegistry) -> Result<()> {
        self.ops.iter().try_for_each(|o| o.register(host))
    }
}

/// LFTJ routing (M4). With `enabled` and no operator, cyclic BGPs still go to SQL.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct LftjConfig {
    /// Route cyclic BGPs to LFTJ when an operator exists (default false).
    pub enabled: bool,
    /// Minimum estimated rows before LFTJ is considered.
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
