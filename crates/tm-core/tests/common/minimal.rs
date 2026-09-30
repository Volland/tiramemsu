//! `MinimalHost`: wraps the rusqlite executor, declares every capability `false`,
//! records every SQL string it runs, and can inject host errors.

use std::path::Path;
use std::sync::{Arc, Mutex};

use tm_core::{Capabilities, Error, Executor, Host, HostOptions, Result, SqlError, SqlValue};
use tm_rusqlite::RusqliteHost;

/// An injected host error: the `skip + 1`-th statement containing `pattern` fails
/// with `code` (once).
#[derive(Clone, Debug)]
pub struct Fault {
    pub pattern: String,
    pub skip: usize,
    pub code: i32,
}

/// Shared recorder and fault list.
#[derive(Default, Debug)]
pub struct Probe {
    pub log: Mutex<Vec<String>>,
    pub faults: Mutex<Vec<Fault>>,
    /// While set, statements are neither recorded nor fault-checked (test-only SQL).
    pub quiet: std::sync::atomic::AtomicBool,
}

impl Probe {
    pub fn inject(&self, pattern: &str, skip: usize, code: i32) {
        self.faults.lock().unwrap().push(Fault {
            pattern: pattern.to_string(),
            skip,
            code,
        });
    }

    pub fn clear_log(&self) {
        self.log.lock().unwrap().clear();
    }

    pub fn count(&self, exact: &str) -> usize {
        self.log
            .lock()
            .unwrap()
            .iter()
            .filter(|s| s.trim() == exact)
            .count()
    }

    fn check(&self, sql: &str) -> Result<()> {
        if self.quiet.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok(());
        }
        self.log.lock().unwrap().push(sql.to_string());
        let mut faults = self.faults.lock().unwrap();
        let mut hit = None;
        for (i, f) in faults.iter_mut().enumerate() {
            if sql.contains(&f.pattern) {
                if f.skip == 0 {
                    hit = Some(i);
                    break;
                }
                f.skip -= 1;
            }
        }
        if let Some(i) = hit {
            let f = faults.remove(i);
            return Err(Error::Sqlite(SqlError::new(
                f.code,
                format!("injected fault on {}", f.pattern),
            )));
        }
        Ok(())
    }
}

/// A host that provides only the required executor operations.
#[derive(Clone)]
pub struct MinimalHost {
    pub inner: RusqliteHost,
    pub probe: Arc<Probe>,
}

impl MinimalHost {
    pub fn new() -> MinimalHost {
        MinimalHost {
            inner: RusqliteHost::new(),
            probe: Arc::new(Probe::default()),
        }
    }
}

impl Host for MinimalHost {
    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }

    fn open_writer(&self, path: &Path, opts: &HostOptions) -> Result<Box<dyn Executor>> {
        Ok(Box::new(MinimalExec {
            inner: self.inner.open_writer(path, opts)?,
            probe: self.probe.clone(),
        }))
    }

    fn open_reader(&self, _path: &Path, _opts: &HostOptions) -> Result<Box<dyn Executor>> {
        Err(Error::Sqlite(SqlError::new(
            SqlError::ERROR,
            "minimal host has no reader pool",
        )))
    }
}

struct MinimalExec {
    inner: Box<dyn Executor>,
    probe: Arc<Probe>,
}

impl Executor for MinimalExec {
    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }

    fn execute(&mut self, sql: &str, params: &[SqlValue]) -> Result<usize> {
        self.probe.check(sql)?;
        self.inner.execute(sql, params)
    }

    fn query(
        &mut self,
        sql: &str,
        params: &[SqlValue],
        row: &mut dyn FnMut(&[SqlValue]) -> Result<()>,
    ) -> Result<()> {
        self.probe.check(sql)?;
        self.inner.query(sql, params, row)
    }

    fn execute_batch(&mut self, sql: &str) -> Result<()> {
        self.probe.check(sql)?;
        self.inner.execute_batch(sql)
    }

    fn begin_immediate(&mut self) -> Result<()> {
        self.probe.check("BEGIN IMMEDIATE")?;
        self.inner.begin_immediate()
    }

    fn begin_read(&mut self) -> Result<()> {
        self.probe.check("BEGIN")?;
        self.inner.begin_read()
    }

    fn commit(&mut self) -> Result<()> {
        self.probe.check("COMMIT")?;
        self.inner.commit()
    }

    fn rollback(&mut self) -> Result<()> {
        self.probe.log.lock().unwrap().push("ROLLBACK".to_string());
        self.inner.rollback()
    }

    fn savepoint(&mut self, name: &str) -> Result<()> {
        self.probe.check(&format!("SAVEPOINT {name}"))?;
        self.inner.savepoint(name)
    }

    fn rollback_to(&mut self, name: &str) -> Result<()> {
        self.probe
            .log
            .lock()
            .unwrap()
            .push(format!("ROLLBACK TO {name}"));
        self.inner.rollback_to(name)
    }

    fn release(&mut self, name: &str) -> Result<()> {
        self.probe
            .log
            .lock()
            .unwrap()
            .push(format!("RELEASE {name}"));
        self.inner.release(name)
    }
}

const ALLOWED_CALLS: &[&str] = &[
    "and",
    "or",
    "not",
    "on",
    "where",
    "as",
    "select",
    "coalesce",
    "max",
    "count",
    "ifnull",
    "in",
    "exists",
    "values",
    "conflict",
    "triple",
    "term",
    "tx",
    "meta",
    "volatile",
    "pred_multi",
];
const ALLOWED_FROM: &[&str] = &[
    "sqlite_stat4",
    "triple",
    "term",
    "tx",
    "meta",
    "volatile",
    "pred_multi",
    "event",
    "sqlite_schema",
];

/// Checks that a recorded statement uses no user function, virtual table or FTS5.
pub fn check_core_sql(sql: &str) -> std::result::Result<(), String> {
    let upper = sql.to_ascii_uppercase();
    if upper.trim_start().starts_with("CREATE") {
        return Ok(());
    }
    for bad in ["RARRAY", "TM_PATH", "FTS", " MATCH ", "VEC_"] {
        if upper.contains(bad) {
            return Err(format!("forbidden {bad} in {sql}"));
        }
    }
    let b = sql.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_alphabetic() || b[i] == b'_' {
            let start = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            let word = sql[start..i].to_ascii_lowercase();
            let mut j = i;
            while j < b.len() && b[j] == b' ' {
                j += 1;
            }
            if j < b.len() && b[j] == b'(' && !ALLOWED_CALLS.contains(&word.as_str()) {
                return Err(format!("call {word}( in {sql}"));
            }
            if word == "from" || word == "join" {
                while j < b.len() && b[j] == b' ' {
                    j += 1;
                }
                let s = j;
                while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
                    j += 1;
                }
                let name = sql[s..j].to_ascii_lowercase();
                if !name.is_empty() && !ALLOWED_FROM.contains(&name.as_str()) {
                    return Err(format!("reads {name} in {sql}"));
                }
            }
        } else {
            i += 1;
        }
    }
    Ok(())
}
