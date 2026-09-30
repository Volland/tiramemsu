//! `ProbeHost`: the rusqlite host with every capability, recording every SQL
//! string its connections run (to assert that a query ran no SQL, or read no
//! dictionary term). Capabilities can be masked to test the capability check.

use std::path::Path;
use std::sync::{Arc, Mutex};

use tiramemsu::{Capabilities, Executor, Host, HostOptions, Result, RusqliteHost, SqlValue};
use tm_core::HostRegistry;

#[derive(Clone, Default)]
pub struct Log(pub Arc<Mutex<Vec<String>>>);

impl Log {
    pub fn clear(&self) {
        self.0.lock().unwrap().clear();
    }
    pub fn all(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
    /// Statements other than transaction control.
    pub fn statements(&self) -> Vec<String> {
        self.all()
            .into_iter()
            .filter(|s| !matches!(s.as_str(), "BEGIN" | "COMMIT" | "ROLLBACK"))
            .collect()
    }
}

#[derive(Clone)]
pub struct ProbeHost {
    pub inner: RusqliteHost,
    pub log: Log,
    pub caps: Capabilities,
}

impl ProbeHost {
    pub fn new() -> ProbeHost {
        let inner = RusqliteHost::new();
        ProbeHost {
            caps: inner.capabilities(),
            inner,
            log: Log::default(),
        }
    }

    pub fn with_caps(mut self, f: impl FnOnce(&mut Capabilities)) -> ProbeHost {
        f(&mut self.caps);
        self
    }
}

pub struct ProbeExec {
    inner: Box<dyn Executor>,
    log: Log,
    caps: Capabilities,
}

impl ProbeExec {
    fn rec(&self, s: &str) {
        self.log.0.lock().unwrap().push(s.to_string());
    }
}

impl Executor for ProbeExec {
    fn capabilities(&self) -> Capabilities {
        self.caps
    }
    fn execute(&mut self, sql: &str, params: &[SqlValue]) -> Result<usize> {
        self.rec(sql);
        self.inner.execute(sql, params)
    }
    fn query(
        &mut self,
        sql: &str,
        params: &[SqlValue],
        row: &mut dyn FnMut(&[SqlValue]) -> Result<()>,
    ) -> Result<()> {
        self.rec(sql);
        self.inner.query(sql, params, row)
    }
    fn execute_batch(&mut self, sql: &str) -> Result<()> {
        self.rec(sql);
        self.inner.execute_batch(sql)
    }
    fn begin_immediate(&mut self) -> Result<()> {
        self.rec("BEGIN");
        self.inner.begin_immediate()
    }
    fn begin_read(&mut self) -> Result<()> {
        self.rec("BEGIN");
        self.inner.begin_read()
    }
    fn commit(&mut self) -> Result<()> {
        self.rec("COMMIT");
        self.inner.commit()
    }
    fn rollback(&mut self) -> Result<()> {
        self.rec("ROLLBACK");
        self.inner.rollback()
    }
    fn savepoint(&mut self, name: &str) -> Result<()> {
        self.inner.savepoint(name)
    }
    fn rollback_to(&mut self, name: &str) -> Result<()> {
        self.inner.rollback_to(name)
    }
    fn release(&mut self, name: &str) -> Result<()> {
        self.inner.release(name)
    }
    fn registry(&mut self) -> Option<&mut dyn HostRegistry> {
        self.inner.registry()
    }
}

impl Host for ProbeHost {
    fn capabilities(&self) -> Capabilities {
        self.caps
    }
    fn open_writer(&self, path: &Path, opts: &HostOptions) -> Result<Box<dyn Executor>> {
        Ok(Box::new(ProbeExec {
            inner: self.inner.open_writer(path, opts)?,
            log: self.log.clone(),
            caps: self.caps,
        }))
    }
    fn open_reader(&self, path: &Path, opts: &HostOptions) -> Result<Box<dyn Executor>> {
        Ok(Box::new(ProbeExec {
            inner: self.inner.open_reader(path, opts)?,
            log: self.log.clone(),
            caps: self.caps,
        }))
    }
}
