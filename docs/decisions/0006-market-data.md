# 0006. Market data and FX sources

Status: Accepted · Date: 2026-11-02

## Context
`proposals/market-data-research.md`. Trading 212's API has no quotes/candles (verified in its
OpenAPI spec). Bitvavo public endpoints provide candles/tickers without keys.

## Options (stocks/ETFs)
- **EODHD** (official API, .US/.LSE/.XETRA, ISIN search; free 20 calls/day, paid from $19.99/month).
- **Alpha Vantage** (free 25 req/day) — fallback only.
- Twelve Data (licence restricts programmatic use below enterprise), Tiingo/Polygon (US only),
  Finnhub (candles premium) — rejected.

## Scores
| Option | Security | Correctness | Performance | Maintainability | UX | Cost | Total |
|---|---|---|---|---|---|---|---|
| EODHD | 8 | 7 | 7 | 8 | 7 | 6 | 43 |
| Alpha Vantage | 8 | 6 | 4 | 7 | 6 | 9 | 40 |

## Decision
- Crypto: Bitvavo public REST (candles, ticker/book, ticker/24h).
- Stocks/ETFs: EODHD daily bars (`tc-marketdata::EodhdClient`), key entered in onboarding.
  `*_US_EQ` tickers map to `<SYM>.US`; UCITS ETFs need an explicit `data_symbol` (ISIN-based mapping
  with unique-listing check; ambiguous → not traded).
- FX: ECB euro reference rates (HUF included). MNB not used (its service is HTTP-only).

## Consequences
The EODHD subscription is a fixed cost (~26 % of starting capital per month on the paid plan); the
free tier suffices for the ETF DCA core and the 10-stock daily universe. This is a user decision
listed in docs/STATUS.md.
