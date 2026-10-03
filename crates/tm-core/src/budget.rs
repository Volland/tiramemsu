//! Operation budgets: deadlines, cancellation and result limits for one operation
//! (OpenSpec change `add-query-budgets`).
//!
//! The facade enters a [`Meter`] on the calling thread for the duration of one
//! operation with [`scope`]. The engine polls [`check`] between steps (path frontier
//! expansion, decoding) and charges what it produces with [`charge_rows`] and
//! [`charge_bytes`]; a host that can interrupt a running statement receives the
//! [`Interrupt`] through [`Executor::set_interrupt`](crate::Executor::set_interrupt).
//! Outside a scope every function here is a no-op, so code that never sets a budget
//! behaves exactly as before.
//!
//! ```
//! use tm_core::budget::{self, CancelToken, Interrupt, Meter, ResultLimit};
//! use tm_core::Error;
//!
//! let mut meter = Meter::default();
//! meter.max_rows = Some(2);
//! let r = budget::scope(meter, || {
//!     budget::charge_rows(2)?;
//!     budget::charge_rows(1)
//! });
//! assert!(matches!(r, Err(Error::ResultLimitExceeded { limit: ResultLimit::Rows(2) })));
//! assert!(budget::charge_rows(1_000).is_ok()); // no scope: no budget
//!
//! let token = CancelToken::new();
//! let meter = Meter::new(Interrupt::new(None, Some(token.clone())));
//! token.cancel();
//! assert!(matches!(budget::scope(meter, budget::check), Err(Error::Cancelled)));
//! ```

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::error::{Error, Result};
use crate::exec::SqlError;
use crate::value::Value;

/// A cancellation flag shared between threads. Clones share the flag: hand one to
/// the operation (in a budget) and keep one to call [`CancelToken::cancel`] from
/// any thread. Cancelling is permanent; use a new token for the next operation.
#[derive(Clone, Debug, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    /// A token that is not cancelled.
    pub fn new() -> CancelToken {
        CancelToken::default()
    }

    /// Requests cancellation. An operation holding this token stops at its next
    /// check and fails with [`Error::Cancelled`]; one that has not started fails
    /// as soon as it starts.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    /// True once [`CancelToken::cancel`] was called.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// The stop conditions of one operation: an absolute deadline and a cancellation
/// token. Cheap to clone and `Send + Sync`, so a host can poll it from a progress
/// callback while a statement runs.
#[derive(Clone, Debug, Default)]
pub struct Interrupt {
    /// The deadline and the timeout it was computed from (for the error).
    deadline: Option<(Instant, Duration)>,
    cancel: Option<CancelToken>,
}

impl Interrupt {
    /// Stop conditions starting now: the deadline is `timeout` from now.
    pub fn new(timeout: Option<Duration>, cancel: Option<CancelToken>) -> Interrupt {
        Interrupt {
            deadline: timeout.map(|t| (Instant::now() + t, t)),
            cancel,
        }
    }

    /// True when there is a deadline or a token, so the interrupt can ever trip.
    pub fn is_set(&self) -> bool {
        self.deadline.is_some() || self.cancel.is_some()
    }

    /// The absolute deadline, if any.
    pub fn deadline(&self) -> Option<Instant> {
        self.deadline.map(|(d, _)| d)
    }

    /// True when a cancellation token is attached.
    pub fn has_cancel(&self) -> bool {
        self.cancel.is_some()
    }

    /// True once the token is cancelled or the deadline has passed.
    pub fn tripped(&self) -> bool {
        self.check().is_err()
    }

    /// `Err(Cancelled)` once the token is cancelled, `Err(DeadlineExceeded)` once
    /// the deadline has passed, else `Ok`. Cancellation wins when both hold.
    pub fn check(&self) -> Result<()> {
        if self.cancel.as_ref().is_some_and(CancelToken::is_cancelled) {
            return Err(Error::Cancelled);
        }
        if let Some((d, timeout)) = self.deadline {
            if Instant::now() >= d {
                return Err(Error::DeadlineExceeded { timeout });
            }
        }
        Ok(())
    }
}

/// The result budget an operation exceeded, with its configured limit.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum ResultLimit {
    /// The row budget (`max_rows`).
    Rows(u64),
    /// The decoded-byte budget (`max_bytes`).
    Bytes(u64),
}

impl std::fmt::Display for ResultLimit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResultLimit::Rows(n) => write!(f, "{n} rows"),
            ResultLimit::Bytes(n) => write!(f, "{n} decoded bytes"),
        }
    }
}

/// The budget state of one running operation: its stop conditions, the reader
/// acquisition timeout, the result limits and what has been charged so far.
#[derive(Clone, Debug, Default)]
pub struct Meter {
    /// Deadline and cancellation.
    pub interrupt: Interrupt,
    /// How long to wait for a read connection (`None`: the database default).
    pub reader_timeout: Option<Duration>,
    /// The most rows the whole operation may produce.
    pub max_rows: Option<u64>,
    /// The most decoded result bytes the whole operation may produce.
    pub max_bytes: Option<u64>,
    rows: u64,
    bytes: u64,
    polls: u32,
}

impl Meter {
    /// A meter with these stop conditions, no reader timeout and no result limits;
    /// set the public fields for the rest.
    pub fn new(interrupt: Interrupt) -> Meter {
        Meter {
            interrupt,
            ..Meter::default()
        }
    }

    /// Rows and decoded bytes charged so far.
    pub fn used(&self) -> (u64, u64) {
        (self.rows, self.bytes)
    }
}

thread_local! {
    /// The meter of the operation running on this thread, if it has a budget.
    static CURRENT: RefCell<Option<Meter>> = const { RefCell::new(None) };
}

/// Restores the previous meter when a scope ends, also on unwinding.
struct Restore(Option<Meter>);

impl Drop for Restore {
    fn drop(&mut self) {
        let prev = self.0.take();
        CURRENT.with(|c| *c.borrow_mut() = prev);
    }
}

/// Runs `f` with `meter` as the budget of this thread's operation, then restores
/// the previous one (also when `f` panics).
pub fn scope<R>(meter: Meter, f: impl FnOnce() -> R) -> R {
    let prev = CURRENT.with(|c| c.borrow_mut().replace(meter));
    let _restore = Restore(prev);
    f()
}

fn with<R>(f: impl FnOnce(&mut Meter) -> R) -> Option<R> {
    CURRENT.with(|c| c.borrow_mut().as_mut().map(f))
}

/// True inside a [`scope`].
pub fn active() -> bool {
    CURRENT.with(|c| c.borrow().is_some())
}

/// The stop conditions of the current operation, when it has any.
pub fn interrupt() -> Option<Interrupt> {
    with(|m| m.interrupt.clone()).filter(Interrupt::is_set)
}

/// The reader acquisition timeout of the current operation, if it sets one.
pub fn reader_timeout() -> Option<Duration> {
    with(|m| m.reader_timeout).flatten()
}

/// What the current operation has charged so far, `(rows, bytes)`.
pub fn used() -> Option<(u64, u64)> {
    with(|m| m.used())
}

/// Fails with `Cancelled` or `DeadlineExceeded` when the current operation must
/// stop; `Ok` outside a scope.
pub fn check() -> Result<()> {
    with(|m| m.interrupt.check()).unwrap_or(Ok(()))
}

/// [`check`] for tight loops: reads the clock on every 64th call only.
pub fn poll() -> Result<()> {
    with(|m| {
        m.polls = m.polls.wrapping_add(1);
        if m.polls % 64 == 0 {
            m.interrupt.check()
        } else {
            Ok(())
        }
    })
    .unwrap_or(Ok(()))
}

/// Charges `n` result rows to the current operation; fails with
/// `ResultLimitExceeded` past `max_rows`. Also polls the stop conditions.
pub fn charge_rows(n: u64) -> Result<()> {
    with(|m| {
        m.rows = m.rows.saturating_add(n);
        if let Some(max) = m.max_rows {
            if m.rows > max {
                return Err(Error::ResultLimitExceeded {
                    limit: ResultLimit::Rows(max),
                });
            }
        }
        m.polls = m.polls.wrapping_add(1);
        if m.polls % 64 == 0 {
            m.interrupt.check()?;
        }
        Ok(())
    })
    .unwrap_or(Ok(()))
}

/// Charges `n` decoded result bytes to the current operation; fails with
/// `ResultLimitExceeded` past `max_bytes`.
pub fn charge_bytes(n: u64) -> Result<()> {
    with(|m| {
        m.bytes = m.bytes.saturating_add(n);
        match m.max_bytes {
            Some(max) if m.bytes > max => Err(Error::ResultLimitExceeded {
                limit: ResultLimit::Bytes(max),
            }),
            _ => Ok(()),
        }
    })
    .unwrap_or(Ok(()))
}

/// The bytes a decoded cell is charged: 8 for the cell itself plus the UTF-8
/// length of every string it carries (IRI, lexical form, language tag, datatype).
pub fn value_bytes(v: &Value) -> u64 {
    let text = match v {
        Value::Iri(s) | Value::Str(s) | Value::Decimal(s) => s.len(),
        Value::LangStr { lex, lang } => lex.len() + lang.len(),
        Value::Typed { lex, datatype } => lex.len() + datatype.len(),
        _ => 0,
    };
    8 + text as u64
}

/// Turns the host error of an interrupted statement (`SQLITE_INTERRUPT`) into the
/// typed error of the current operation's stop condition; any other error, or an
/// interrupt the current operation did not cause, is returned unchanged.
pub fn map_interrupt(e: Error) -> Error {
    if e.sql().is_some_and(|s| s.code == SqlError::INTERRUPT) {
        if let Some(Err(typed)) = with(|m| m.interrupt.check()) {
            return typed;
        }
    }
    e
}
