//! The pool of read-only executors (only when the host declares `reader_pool`).

use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use tm_core::{budget, Error, Executor, Result};

/// How often a reader wait wakes to look at a cancellation token.
const CANCEL_POLL: Duration = Duration::from_millis(10);

/// Read-only executors on the same WAL file. Each read runs in one read transaction.
pub(crate) struct ReaderPool {
    idle: Mutex<Vec<Box<dyn Executor>>>,
    available: Condvar,
    size: usize,
    /// The default reader acquisition timeout (`OpenOptions::reader_timeout`).
    timeout: Option<Duration>,
}

impl ReaderPool {
    pub(crate) fn new(readers: Vec<Box<dyn Executor>>, timeout: Option<Duration>) -> ReaderPool {
        let size = readers.len();
        ReaderPool {
            idle: Mutex::new(readers),
            available: Condvar::new(),
            size,
            timeout,
        }
    }

    pub(crate) fn size(&self) -> usize {
        self.size
    }

    /// Takes an idle reader. Waits while every reader is busy: without limit by
    /// default, else until the reader timeout (`PoolTimeout`), the operation's
    /// deadline (`DeadlineExceeded`) or its cancellation (`Cancelled`).
    fn acquire(&self) -> Result<Box<dyn Executor>> {
        let interrupt = budget::interrupt();
        let timeout = budget::reader_timeout().or(self.timeout);
        let until = timeout.map(|t| Instant::now() + t);
        let mut idle = self.idle.lock().unwrap_or_else(|p| p.into_inner());
        loop {
            if let Some(e) = idle.pop() {
                return Ok(e);
            }
            if let Some(i) = &interrupt {
                i.check()?;
            }
            let now = Instant::now();
            let mut wait = None;
            if let (Some(until), Some(timeout)) = (until, timeout) {
                if now >= until {
                    return Err(Error::PoolTimeout { timeout });
                }
                wait = Some(until - now);
            }
            if let Some(i) = &interrupt {
                let cap = match i.deadline() {
                    Some(d) => d.saturating_duration_since(now),
                    None => Duration::MAX,
                };
                let cap = if i.has_cancel() {
                    cap.min(CANCEL_POLL)
                } else {
                    cap
                };
                wait = Some(wait.map_or(cap, |w: Duration| w.min(cap)));
            }
            idle = match wait {
                None => self.available.wait(idle).unwrap_or_else(|p| p.into_inner()),
                Some(w) => {
                    self.available
                        .wait_timeout(idle, w)
                        .unwrap_or_else(|p| p.into_inner())
                        .0
                }
            };
        }
    }

    /// Runs `f` on an idle reader inside `begin_read` ... `commit`, so it observes
    /// one committed snapshot. Blocks while every reader is busy (see `acquire`).
    /// Under an operation budget the reader carries its stop conditions while `f`
    /// runs; they are removed before the snapshot ends and the reader is returned.
    pub(crate) fn read<R>(&self, f: impl FnOnce(&mut dyn Executor) -> Result<R>) -> Result<R> {
        let mut exec = self.acquire()?;
        let interrupt = budget::interrupt();
        let armed = interrupt.is_some();
        if armed {
            exec.set_interrupt(interrupt);
        }
        let r = catch_unwind(AssertUnwindSafe(|| {
            exec.begin_read()?;
            let r = f(exec.as_mut());
            if armed {
                exec.set_interrupt(None);
            }
            let r = r?;
            exec.commit()?;
            Ok(r)
        }));
        if armed {
            exec.set_interrupt(None);
        }
        if !matches!(&r, Ok(Ok(_))) {
            let _ = exec.rollback();
        }
        self.idle
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(exec);
        self.available.notify_one();
        match r {
            Ok(r) => r,
            Err(panic) => resume_unwind(panic),
        }
    }
}
