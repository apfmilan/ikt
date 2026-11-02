use chrono::{DateTime, Utc};
use std::sync::Mutex;

/// Time source. Production uses [`SystemClock`]; tests and backtests inject [`FixedClock`].
pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// Manually advanced clock for deterministic tests.
#[derive(Debug)]
pub struct FixedClock(Mutex<DateTime<Utc>>);

impl FixedClock {
    pub fn new(t: DateTime<Utc>) -> Self {
        FixedClock(Mutex::new(t))
    }

    pub fn set(&self, t: DateTime<Utc>) {
        if let Ok(mut g) = self.0.lock() {
            *g = t;
        }
    }

    pub fn advance(&self, d: chrono::Duration) {
        if let Ok(mut g) = self.0.lock() {
            *g += d;
        }
    }
}

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
            .lock()
            .map(|g| *g)
            .unwrap_or_else(|p| *p.into_inner())
    }
}
