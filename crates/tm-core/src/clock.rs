//! Wall clocks for transaction instants.

use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Source of wall-clock time in epoch milliseconds.
pub trait Clock: Send + Sync {
    /// The current time in epoch milliseconds.
    fn now_ms(&self) -> i64;
}

/// The system clock.
#[derive(Copy, Clone, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }
}

/// A clock that only moves when told to (tests).
#[derive(Debug, Default)]
pub struct ManualClock(AtomicI64);

impl ManualClock {
    /// A clock showing `ms`.
    pub fn new(ms: i64) -> ManualClock {
        ManualClock(AtomicI64::new(ms))
    }

    /// Sets the time.
    pub fn set(&self, ms: i64) {
        self.0.store(ms, Ordering::SeqCst);
    }

    /// Moves the time by `delta` milliseconds (may be negative).
    pub fn advance(&self, delta: i64) {
        self.0.fetch_add(delta, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}
