//! Host capabilities (decision D22): the query engine needs `functions` and
//! `vtab`, checks them before registering anything, and registers its SQL helper
//! functions and native operators through the host's hooks on every connection.

use tm_core::{Capabilities, Executor, Result};

use crate::error::missing_capability;
use crate::native::OperatorRegistry;
use crate::udf;

/// Fails with `MissingCapability` naming the first missing capability
/// (`functions`, then `vtab`). Never degrades silently.
///
/// # Example
///
/// ```
/// use tm_core::Capabilities;
/// use tm_exec::host::check_capabilities;
///
/// let none = Capabilities::default();
/// assert!(check_capabilities(none).is_err());
/// ```
// @lat: [[architecture#Executor]]
pub fn check_capabilities(c: Capabilities) -> Result<()> {
    if !c.functions {
        return Err(missing_capability("functions"));
    }
    if !c.vtab {
        return Err(missing_capability("vtab"));
    }
    Ok(())
}

/// Checks the capabilities of `exec`, then registers every helper function and
/// every native operator on it.
///
/// # Errors
///
/// `MissingCapability` if the host cannot register functions or virtual tables.
pub fn install(exec: &mut dyn Executor, ops: &OperatorRegistry) -> Result<()> {
    check_capabilities(exec.capabilities())?;
    let reg = exec
        .registry()
        .ok_or_else(|| missing_capability("functions"))?;
    for f in udf::scalar_functions() {
        reg.register_scalar(f)?;
    }
    for f in udf::aggregate_functions() {
        reg.register_aggregate(f)?;
    }
    ops.register_all(reg)
}
