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

/// The migrations of this build: format 1 to 2 adds the text-index bookkeeping,
/// format 2 to 3 the saved-answer tables, format 3 to 4 the transaction-date guards.
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        from: 1,
        apply: v1_to_v2,
    },
    Migration {
        from: 2,
        apply: v2_to_v3,
    },
    Migration {
        from: 3,
        apply: v3_to_v4,
    },
];

/// Format 3 to 4 adds clock validation without rewriting historical rows.
fn v3_to_v4(exec: &mut dyn Executor) -> Result<()> {
    for stmt in super::split_statements(super::DDL_DATE_GUARDS) {
        exec.execute_batch(&stmt)?;
    }
    Ok(())
}

/// Format 1 to 2: the `meta` rows of the derived text index (OpenSpec change
/// `add-text-retrieval`). `text_index` is the index version (0: not built) and
/// `text_stale` the lowest statement eid a host without FTS5 wrote while the
/// index was built (0: none). The FTS5 table itself (`term_fts`, reserved by
/// format 1) is created only when the index is built, on a host with FTS5, so the
/// migration runs on every host and touches no graph row.
// @lat: [[storage#Text Index]]
fn v1_to_v2(exec: &mut dyn Executor) -> Result<()> {
    for key in ["text_index", "text_stale"] {
        exec.execute(
            "INSERT INTO meta(key, value) SELECT ?1, 0 WHERE NOT EXISTS \
             (SELECT 1 FROM meta WHERE key = ?1)",
            &[SqlValue::from(key)],
        )?;
    }
    Ok(())
}

/// Format 2 to 3: the derived saved-answer records (OpenSpec change
/// `add-saved-answer-invalidation`), `saved_answer` and `saved_answer_dep`. They
/// are empty until an answer is saved; no graph row is read or written.
// @lat: [[storage#Saved Answers]]
fn v2_to_v3(exec: &mut dyn Executor) -> Result<()> {
    for stmt in super::split_statements(super::DDL_SAVED_ANSWERS) {
        exec.execute_batch(&stmt)?;
    }
    Ok(())
}

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
