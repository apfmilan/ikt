//! ECB euro foreign exchange reference rates.
//!
//! Source: https://www.ecb.europa.eu/stats/eurofxref/eurofxref-daily.xml (checked 2026-11-02:
//! HTTP 200, `text/xml`, gesmes envelope, EUR base, dot decimals, includes HUF). Rates are
//! published around 16:00 CET on TARGET working days and are "for information purposes only";
//! realized P&L always uses broker amounts, these rates only translate values into HUF.

use chrono::NaiveDate;
use rust_decimal::Decimal;
use std::collections::BTreeMap;
use std::time::Duration;
use tc_broker::{BrokerError, BrokerResult};
use tc_core::{parse_decimal, Currency};

pub const ECB_DAILY_URL: &str = "https://www.ecb.europa.eu/stats/eurofxref/eurofxref-daily.xml";

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FxError {
    #[error("malformed ECB XML: {0}")]
    Malformed(String),
    #[error("ECB XML lacks required currency {0}")]
    MissingCurrency(&'static str),
}

/// Daily reference rates: units of a currency per 1 EUR (`EUR` itself = 1).
#[derive(Clone, Debug, PartialEq)]
pub struct FxTable {
    pub as_of: NaiveDate,
    pub source: &'static str,
    pub per_eur: BTreeMap<Currency, Decimal>,
}

impl FxTable {
    pub fn per_eur(&self, c: Currency) -> Option<Decimal> {
        self.per_eur.get(&c).copied()
    }

    /// Units of `to` for 1 unit of `from` (cross via EUR).
    pub fn rate(&self, from: Currency, to: Currency) -> Option<Decimal> {
        if from == to {
            return Some(Decimal::ONE);
        }
        let f = self.per_eur(from)?;
        let t = self.per_eur(to)?;
        if f.is_zero() {
            return None;
        }
        Some(t / f)
    }

    pub fn convert(&self, amount: Decimal, from: Currency, to: Currency) -> Option<Decimal> {
        Some(amount * self.rate(from, to)?)
    }

    /// HUF per 1 unit of `c`.
    pub fn huf_per(&self, c: Currency) -> Option<Decimal> {
        self.rate(c, Currency::HUF)
    }

    pub fn to_huf(&self, amount: Decimal, c: Currency) -> Option<Decimal> {
        self.convert(amount, c, Currency::HUF)
    }

    /// Calendar days since publication.
    pub fn age_days(&self, today: NaiveDate) -> i64 {
        (today - self.as_of).num_days()
    }
}

/// Attributes of every `<Cube ...>` tag, in document order.
fn cube_tags(xml: &str) -> Vec<BTreeMap<String, String>> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(i) = rest.find("<Cube") {
        rest = &rest[i + 5..];
        let Some(end) = rest.find('>') else { break };
        let tag = &rest[..end];
        rest = &rest[end..];
        let mut attrs = BTreeMap::new();
        let mut s = tag;
        while let Some(eq) = s.find('=') {
            let key = s[..eq]
                .trim()
                .rsplit(char::is_whitespace)
                .next()
                .unwrap_or_default()
                .to_owned();
            let after = s[eq + 1..].trim_start();
            let Some(q) = after.chars().next().filter(|c| *c == '\'' || *c == '"') else {
                break;
            };
            let body = &after[1..];
            let Some(close) = body.find(q) else { break };
            attrs.insert(key, body[..close].to_owned());
            s = &body[close + 1..];
        }
        if !attrs.is_empty() {
            out.push(attrs);
        }
    }
    out
}

/// Parse `eurofxref-daily.xml`. Requires a `time` cube and HUF + USD rates.
pub fn parse_ecb_xml(xml: &str) -> Result<FxTable, FxError> {
    if !xml.contains("gesmes:Envelope") {
        return Err(FxError::Malformed("not a gesmes envelope".into()));
    }
    let tags = cube_tags(xml);
    let time = tags
        .iter()
        .find_map(|a| a.get("time"))
        .ok_or_else(|| FxError::Malformed("no time attribute".into()))?;
    let as_of = NaiveDate::parse_from_str(time, "%Y-%m-%d")
        .map_err(|e| FxError::Malformed(format!("time {time}: {e}")))?;
    let mut per_eur = BTreeMap::new();
    per_eur.insert(Currency::EUR, Decimal::ONE);
    for a in &tags {
        let (Some(c), Some(r)) = (a.get("currency"), a.get("rate")) else {
            continue;
        };
        let currency = Currency::new(c).map_err(|e| FxError::Malformed(e.to_string()))?;
        let rate = parse_decimal(r).map_err(|e| FxError::Malformed(e.to_string()))?;
        if rate <= Decimal::ZERO {
            return Err(FxError::Malformed(format!("non-positive rate for {c}")));
        }
        per_eur.insert(currency, rate);
    }
    if !per_eur.contains_key(&Currency::HUF) {
        return Err(FxError::MissingCurrency("HUF"));
    }
    if !per_eur.contains_key(&Currency::USD) {
        return Err(FxError::MissingCurrency("USD"));
    }
    Ok(FxTable {
        as_of,
        source: "ecb",
        per_eur,
    })
}

/// HTTP fetcher for the ECB daily file.
#[derive(Debug, Clone)]
pub struct EcbFx {
    url: String,
    http: reqwest::Client,
}

impl EcbFx {
    pub fn new(url: impl Into<String>) -> BrokerResult<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .user_agent("trading-center/2")
            .build()
            .map_err(|e| BrokerError::Network(e.without_url().to_string()))?;
        Ok(EcbFx {
            url: url.into(),
            http,
        })
    }

    pub fn official() -> BrokerResult<Self> {
        EcbFx::new(ECB_DAILY_URL)
    }

    pub async fn fetch(&self) -> BrokerResult<FxTable> {
        let resp = self.http.get(&self.url).send().await.map_err(|e| {
            if e.is_timeout() {
                BrokerError::Timeout
            } else {
                BrokerError::Network(e.without_url().to_string())
            }
        })?;
        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| BrokerError::Network(e.without_url().to_string()))?;
        if !status.is_success() {
            return Err(BrokerError::Rejected {
                status: status.as_u16(),
                code: "ecb".into(),
                message: body.chars().take(200).collect(),
            });
        }
        parse_ecb_xml(&body).map_err(|e| BrokerError::Decode(e.to_string()))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn attribute_parser_accepts_both_quote_styles() {
        let xml = r#"<gesmes:Envelope><Cube><Cube time="2026-10-30"><Cube currency="USD" rate="1.1"/><Cube currency='HUF' rate='366.3'/></Cube></Cube></gesmes:Envelope>"#;
        let t = parse_ecb_xml(xml).unwrap();
        assert_eq!(t.per_eur.len(), 3);
    }

    #[test]
    fn rejects_missing_huf_and_bad_rates() {
        let no_huf = r#"<gesmes:Envelope><Cube time='2026-10-30'><Cube currency='USD' rate='1.1'/></Cube></gesmes:Envelope>"#;
        assert_eq!(parse_ecb_xml(no_huf), Err(FxError::MissingCurrency("HUF")));
        let neg = r#"<gesmes:Envelope><Cube time='2026-10-30'><Cube currency='USD' rate='-1'/></Cube></gesmes:Envelope>"#;
        assert!(parse_ecb_xml(neg).is_err());
        assert!(parse_ecb_xml("<html>maintenance</html>").is_err());
    }
}
