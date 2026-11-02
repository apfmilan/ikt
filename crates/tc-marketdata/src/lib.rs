//! Market data adapters that are not brokers:
//! * [`ecb`] — ECB euro reference FX rates (EUR base, includes HUF) for HUF reporting.
//! * [`eodhd`] — EODHD daily bars for Trading 212 stocks/ETFs (Trading 212 has no market data).
//!
//! Neither source executes trades; both are read-only HTTPS APIs.

pub mod ecb;
pub mod eodhd;

pub use ecb::{parse_ecb_xml, EcbFx, FxError, FxTable, ECB_DAILY_URL};
pub use eodhd::{select_listing, EodhdClient, EodhdConfig, EodhdListing, EODHD_BASE_URL};
