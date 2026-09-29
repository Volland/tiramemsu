//! The pool of read-only executors (only when the host declares `reader_pool`).

use std::sync::{Condvar, Mutex};

use tm_core::{Executor, Result};

/// Read-only executors on the same WAL file. Each read runs in one read transaction.
pub(crate) struct ReaderPool {
    idle: Mutex<Vec<Box<dyn Executor>>>,
    available: Condvar,
    size: usize,
}

impl ReaderPool {
    pub(crate) fn new(readers: Vec<Box<dyn Executor>>) -> ReaderPool {
        let size = readers.len();
        ReaderPool {
            idle: Mutex::new(readers),
            available: Condvar::new(),
            size,
        }
    }

    pub(crate) fn size(&self) -> usize {
        self.size
    }

    /// Runs `f` on an idle reader inside `begin_read` ... `commit`, so it observes
    /// one committed snapshot. Blocks while every reader is busy.
    pub(crate) fn read<R>(&self, f: impl FnOnce(&mut dyn Executor) -> Result<R>) -> Result<R> {
        let mut exec = {
            let mut idle = self.idle.lock().unwrap_or_else(|p| p.into_inner());
            loop {
                if let Some(e) = idle.pop() {
                    break e;
                }
                idle = self.available.wait(idle).unwrap_or_else(|p| p.into_inner());
            }
        };
        let r = (|| {
            exec.begin_read()?;
            match f(exec.as_mut()) {
                Ok(r) => {
                    exec.commit()?;
                    Ok(r)
                }
                Err(e) => {
                    let _ = exec.rollback();
                    Err(e)
                }
            }
        })();
        self.idle
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(exec);
        self.available.notify_one();
        r
    }
}
