//! Forward migrations between format versions.

// @lat: [[storage#Format Versioning]]

use crate::error::{Error, Result};
use crate::exec::{Executor, SqlValue};

/// One forward migration step from format version `from` to `from + 1`.
///
/// A migration may only add tables, columns, indexes, views or triggers, and must
/// never delete or rewrite rows of `triple`, `term` or `tx`.
#[derive(Copy, Clone)]
pub struct Migration {
    /// The version this step migrates from.
    pub from: i64,
    /// Applies the step inside the open's `BEGIN IMMEDIATE`.
    pub apply: fn(&mut dyn Executor) -> Result<()>,
}

impl std::fmt::Debug for Migration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Migration")
            .field("from", &self.from)
            .finish()
    }
}

/// The migrations of this build. Empty for format version 1.
pub const MIGRATIONS: &[Migration] = &[];

/// Checks `found` against `supported` and applies the pending migrations in order,
/// then records the new version. Runs inside the caller's transaction.
pub fn run_migrations(
    exec: &mut dyn Executor,
    found: i64,
    supported: i64,
    migrations: &[Migration],
) -> Result<()> {
    if found > supported {
        return Err(Error::FormatVersion { found, supported });
    }
    if found == supported {
        return Ok(());
    }
    let mut steps: Vec<&Migration> = migrations
        .iter()
        .filter(|m| m.from >= found && m.from < supported)
        .collect();
    steps.sort_by_key(|m| m.from);
    let mut v = found;
    for m in steps {
        if m.from != v {
            break;
        }
        (m.apply)(exec)?;
        v += 1;
    }
    if v != supported {
        return Err(Error::FormatVersion { found, supported });
    }
    exec.execute(
        "UPDATE meta SET value = ?1 WHERE key = 'format_version'",
        &[SqlValue::Integer(supported)],
    )?;
    Ok(())
}
