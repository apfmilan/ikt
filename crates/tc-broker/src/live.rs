//! Process-wide live-trading guard.
//!
//! Order placement/cancellation against a real broker host is refused unless
//! [`arm`] was called. Only the engine binary calls `arm`, and only when
//! `TC_LIVE_TRADING=enabled` is set in its environment. Test builds can never arm.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::BrokerError;

static ARMED: AtomicBool = AtomicBool::new(false);

/// Hosts that execute real-money orders.
pub const LIVE_HOSTS: &[&str] = &["live.trading212.com", "api.bitvavo.com", "ws.bitvavo.com"];

/// Arm live trading for this process. Returns `false` (and stays disarmed) in test builds or
/// when the environment flag is absent.
pub fn arm(env_flag: Option<&str>) -> bool {
    if cfg!(test) || env_flag != Some("enabled") {
        return false;
    }
    ARMED.store(true, Ordering::SeqCst);
    true
}

/// Immediately disarm (kill switch FREEZE, shutdown).
pub fn disarm() {
    ARMED.store(false, Ordering::SeqCst);
}

pub fn is_armed() -> bool {
    ARMED.load(Ordering::SeqCst)
}

/// Whether a base URL points at a real broker.
pub fn is_live_host(base_url: &str) -> bool {
    let host = base_url
        .split("://")
        .nth(1)
        .unwrap_or(base_url)
        .split(['/', ':'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    LIVE_HOSTS.iter().any(|h| host == *h)
}

/// Call before every state-changing request (place/cancel).
pub fn check_live_allowed(base_url: &str) -> Result<(), BrokerError> {
    if is_live_host(base_url) && !is_armed() {
        return Err(BrokerError::LiveDisabled);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tests_can_never_arm() {
        assert!(!arm(Some("enabled")));
        assert!(!is_armed());
        assert!(matches!(
            check_live_allowed("https://live.trading212.com/api/v0"),
            Err(BrokerError::LiveDisabled)
        ));
        assert!(matches!(
            check_live_allowed("https://api.bitvavo.com/v2"),
            Err(BrokerError::LiveDisabled)
        ));
        assert!(check_live_allowed("http://127.0.0.1:4545").is_ok());
        assert!(check_live_allowed("https://demo.trading212.com/api/v0").is_ok());
    }

    #[test]
    fn host_parsing() {
        assert!(is_live_host("https://API.bitvavo.com/v2/order"));
        assert!(!is_live_host("https://api.bitvavo.com.evil.example/v2"));
        assert!(!is_live_host("http://localhost:8080"));
    }
}
