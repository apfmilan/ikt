//! Token-bucket rate limiter per endpoint class. Brokers publish per-endpoint limits
//! (Trading 212: e.g. limit orders 1 req / 2 s; Bitvavo: 1000 weight / min).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy)]
pub struct Limit {
    /// Bucket capacity (burst).
    pub capacity: u32,
    /// Refill period for the whole capacity.
    pub period: Duration,
}

impl Limit {
    pub const fn per(capacity: u32, period: Duration) -> Self {
        Limit { capacity, period }
    }
}

#[derive(Debug)]
struct Bucket {
    tokens: f64,
    last: Instant,
    limit: Limit,
    blocked_until: Option<Instant>,
}

/// Async-friendly limiter: `acquire` sleeps until a token is available.
#[derive(Debug, Default)]
pub struct RateLimiter {
    buckets: Mutex<HashMap<&'static str, Bucket>>,
}

impl RateLimiter {
    pub fn new(limits: &[(&'static str, Limit)]) -> Self {
        let now = Instant::now();
        let buckets = limits
            .iter()
            .map(|(k, l)| {
                (
                    *k,
                    Bucket {
                        tokens: l.capacity as f64,
                        last: now,
                        limit: *l,
                        blocked_until: None,
                    },
                )
            })
            .collect();
        RateLimiter {
            buckets: Mutex::new(buckets),
        }
    }

    /// Time to wait before a token for `key` is available (and consume it if zero).
    fn try_take(&self, key: &'static str, weight: u32) -> Duration {
        let Ok(mut map) = self.buckets.lock() else {
            return Duration::from_millis(100);
        };
        let Some(b) = map.get_mut(key) else {
            return Duration::ZERO;
        };
        let now = Instant::now();
        if let Some(until) = b.blocked_until {
            if now < until {
                return until - now;
            }
            b.blocked_until = None;
        }
        let rate = b.limit.capacity as f64 / b.limit.period.as_secs_f64();
        b.tokens = (b.tokens + (now - b.last).as_secs_f64() * rate).min(b.limit.capacity as f64);
        b.last = now;
        let w = weight as f64;
        if b.tokens >= w {
            b.tokens -= w;
            Duration::ZERO
        } else {
            Duration::from_secs_f64((w - b.tokens) / rate)
        }
    }

    pub async fn acquire(&self, key: &'static str, weight: u32) {
        loop {
            let wait = self.try_take(key, weight);
            if wait.is_zero() {
                return;
            }
            tokio::time::sleep(wait).await;
        }
    }

    /// Block a bucket after a 429 until the broker's reset time.
    pub fn block_for(&self, key: &'static str, d: Duration) {
        if let Ok(mut map) = self.buckets.lock() {
            if let Some(b) = map.get_mut(key) {
                b.blocked_until = Some(Instant::now() + d);
                b.tokens = 0.0;
            }
        }
    }

    /// Non-blocking check used by the scheduler to skip work instead of queueing it.
    pub fn would_wait(&self, key: &'static str, weight: u32) -> Duration {
        let Ok(map) = self.buckets.lock() else {
            return Duration::from_millis(100);
        };
        let Some(b) = map.get(key) else {
            return Duration::ZERO;
        };
        let now = Instant::now();
        if let Some(until) = b.blocked_until {
            if now < until {
                return until - now;
            }
        }
        let rate = b.limit.capacity as f64 / b.limit.period.as_secs_f64();
        let tokens = (b.tokens + (now - b.last).as_secs_f64() * rate).min(b.limit.capacity as f64);
        if tokens >= weight as f64 {
            Duration::ZERO
        } else {
            Duration::from_secs_f64((weight as f64 - tokens) / rate)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn bucket_enforces_period() {
        let rl = RateLimiter::new(&[("limit_order", Limit::per(1, Duration::from_secs(2)))]);
        let t0 = tokio::time::Instant::now();
        rl.acquire("limit_order", 1).await;
        assert!(rl.would_wait("limit_order", 1) > Duration::from_millis(1500));
        // Unknown keys are unlimited.
        rl.acquire("other", 1).await;
        assert!(t0.elapsed() < Duration::from_millis(10));
    }

    #[test]
    fn block_after_429() {
        let rl = RateLimiter::new(&[("orders", Limit::per(50, Duration::from_secs(60)))]);
        rl.block_for("orders", Duration::from_secs(60));
        assert!(rl.would_wait("orders", 1) > Duration::from_secs(59));
    }
}
