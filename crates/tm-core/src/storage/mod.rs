//! The database file: schema, pragmas, initialisation, format checks and migrations.

pub mod meta;
pub mod migrate;
pub mod stats;

use std::path::Path;

use crate::error::{Error, Result};
use crate::exec::{Executor, Host, HostOptions, SqlValue};

/// The format version this build writes and reads.
pub const FORMAT_VERSION: i64 = 1;

/// The format-1 DDL: tables, indexes, the `event` view and the invariant triggers.
// @lat: [[storage#Schema]]
// @lat: [[storage#Invariant Triggers]]
pub const DDL_V1: &str = include_str!("ddl_v1.sql");

/// Names reserved by format 1 for later milestones (never created by format 1).
pub const RESERVED_NAMES: [&str; 2] = ["seal_key", "term_fts"];
/// Prefix reserved for retrieval tables (M7).
pub const RESERVED_PREFIX: &str = "vec_";

/// Splits a SQL script into statements (trigger bodies stay whole).
pub fn split_statements(sql: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = sql.chars().peekable();
    let mut in_str = false;
    while let Some(c) = chars.next() {
        if in_str {
            cur.push(c);
            if c == '\'' {
                in_str = false;
            }
            continue;
        }
        match c {
            '\'' => {
                in_str = true;
                cur.push(c);
            }
            '-' if chars.peek() == Some(&'-') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        cur.push('\n');
                        break;
                    }
                }
            }
            ';' => {
                cur.push(';');
                let t = cur.trim();
                let upper = t.to_ascii_uppercase();
                let is_trigger = upper.starts_with("CREATE TRIGGER");
                if !is_trigger || upper.trim_end_matches(';').trim_end().ends_with("END") {
                    if t != ";" {
                        out.push(t.to_string());
                    }
                    cur.clear();
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

/// Sets the per-connection pragmas of a writer connection.
pub fn configure_writer(exec: &mut dyn Executor) -> Result<()> {
    exec.execute_batch("PRAGMA synchronous = NORMAL; PRAGMA recursive_triggers = ON;")
}

/// Sets the per-connection pragmas of a read-only connection.
pub fn configure_reader(exec: &mut dyn Executor) -> Result<()> {
    exec.execute_batch("PRAGMA synchronous = NORMAL; PRAGMA recursive_triggers = ON;")
}

/// Opens (creating or migrating) the database at `path` and returns the writer.
pub fn open(host: &dyn Host, path: &Path, opts: &HostOptions) -> Result<Box<dyn Executor>> {
    open_with(host, path, opts, migrate::MIGRATIONS, FORMAT_VERSION)
}

/// [`open`] with an explicit migration list and supported version (tests only).
#[doc(hidden)]
pub fn open_with(
    host: &dyn Host,
    path: &Path,
    opts: &HostOptions,
    migrations: &[migrate::Migration],
    supported: i64,
) -> Result<Box<dyn Executor>> {
    let mut exec = host.open_writer(path, opts)?;
    configure_writer(exec.as_mut())?;
    exec.begin_immediate()?;
    match init_or_check(exec.as_mut(), path, migrations, supported) {
        Ok(()) => exec.commit()?,
        Err(e) => {
            let _ = exec.rollback();
            return Err(e);
        }
    }
    exec.execute_batch("PRAGMA journal_mode = WAL")?;
    stats::optimize_at_open(exec.as_mut())?;
    Ok(exec)
}

fn init_or_check(
    exec: &mut dyn Executor,
    path: &Path,
    migrations: &[migrate::Migration],
    supported: i64,
) -> Result<()> {
    let objects = exec
        .query_i64(
            "SELECT count(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
            &[],
        )?
        .unwrap_or(0);
    if objects == 0 {
        for stmt in split_statements(DDL_V1) {
            exec.execute_batch(&stmt)?;
        }
        for (k, v) in meta::INITIAL {
            exec.execute(
                "INSERT INTO meta(key, value) VALUES (?1, ?2)",
                &[SqlValue::from(k), SqlValue::Integer(v)],
            )?;
        }
        return Ok(());
    }
    let has_meta = exec
        .query_i64(
            "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = 'meta'",
            &[],
        )?
        .unwrap_or(0)
        > 0;
    let found = if has_meta {
        exec.query_i64("SELECT value FROM meta WHERE key = 'format_version'", &[])?
    } else {
        None
    };
    let Some(found) = found else {
        return Err(Error::ForeignFile(path.to_path_buf()));
    };
    migrate::run_migrations(exec, found, supported, migrations)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ddl_splits_into_statements() {
        let stmts = split_statements(DDL_V1);
        let triggers = stmts
            .iter()
            .filter(|s| s.starts_with("CREATE TRIGGER"))
            .count();
        assert_eq!(triggers, 9);
        assert!(stmts.iter().all(|s| s.ends_with(';')));
        let tables = stmts
            .iter()
            .filter(|s| s.starts_with("CREATE TABLE"))
            .count();
        assert_eq!(tables, 6);
        assert_eq!(
            stmts
                .iter()
                .filter(|s| s.starts_with("CREATE VIEW"))
                .count(),
            1
        );
    }
}
