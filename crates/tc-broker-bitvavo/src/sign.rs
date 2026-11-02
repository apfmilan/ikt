//! Request signing per https://docs.bitvavo.com/docs/rest-api/introduction/ (checked 2026-11-02):
//! `Bitvavo-Access-Signature = hex(HMAC-SHA256(secret, timestamp + METHOD + path + body))`
//! where `path` includes the `/v2` prefix and the query string, and `body` is the exact
//! request body string (empty for GET/DELETE without body).

use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Compute the hex-encoded HMAC-SHA256 signature.
pub fn sign(
    secret: &str,
    timestamp_ms: i64,
    method: &str,
    path_and_query: &str,
    body: &str,
) -> String {
    // HMAC accepts keys of any length; `new_from_slice` cannot fail for HMAC.
    let mut mac = match HmacSha256::new_from_slice(secret.as_bytes()) {
        Ok(m) => m,
        Err(_) => return String::new(),
    };
    mac.update(timestamp_ms.to_string().as_bytes());
    mac.update(method.as_bytes());
    mac.update(path_and_query.as_bytes());
    mac.update(body.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// Official example from the Bitvavo REST introduction page.
    #[test]
    fn official_doc_example() {
        let body = r#"{"market":"BTC-EUR","side":"buy","price":"5000","amount":"1.23","orderType":"limit"}"#;
        assert_eq!(
            sign("bitvavo", 1548172481125, "POST", "/v2/order", body),
            "44d022723a20973a18f7ee97398b9fdd405d2d019c8d39e24b8cc0dcb39ca016"
        );
    }
}
