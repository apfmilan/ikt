# Proposal: market data, FX, Revolut and iOS research

Status: proposal. Checked 2026-11-02 against official docs (curl). `[Sn]` = sources at the end. UNVERIFIED = not confirmed from a primary source.

## 1. Stock market-data APIs compared

| Provider | Auth | Free tier | Cheapest useful paid tier | US / LSE / Xetra | Daily / intraday | WebSocket | ISIN lookup | Terms |
|---|---|---|---|---|---|---|---|---|
| **EODHD** | `api_token` query [S1] | 20 calls/day, 20 req/min, 1 yr EOD, no intraday [S2] | $19.99/mo EOD All-World: 100,000 calls/day, 1,000 req/min, 30+ yr EOD, 15-min delayed live. $29.99/mo adds intraday + WS [S2] | Yes / `.LSE` / `.XETRA` [S1] | Yes / 1m, 5m, 1h [S3] | 50 tickers, US + 18 EU markets incl. London and XETRA ($29.99 tier) [S4] | Yes, Search API [S5] | Personal use only [S6] |
| Twelve Data | `Authorization: apikey <key>` or `apikey` query [S7] | 8 credits/min, 800/day, "testing, evaluation, or development purposes only … cannot be … used in production systems" [S8] | Grow $79: "cannot be programmatically processed". Only Ultra ($999/mo) allows "programmatic processing" [S8] | Yes / EU real-time from Pro $229 via add-ons / UNVERIFIED [S8] | Yes / 1min–8h [S7] | `wss://ws.twelvedata.com` [S7] | ISIN add-on from $24,000/yr [S8] | **Excluded: licence** |
| Tiingo | Token (UNVERIFIED) | 50 req/h, 1,000/day, 500 symbols/mo [S9] | Power $30/mo, 10,000 req/h [S9] | US + China only. `supported_tickers.csv` has no LSE/XETRA [S10] | Yes / UNVERIFIED | UNVERIFIED | No | "Internal Use Only" [S9]. **Excluded: no EU** |
| Finnhub | `token` query or `X-Finnhub-Token` header [S11] | 60 calls/min (UNVERIFIED, JS page); 30 calls/s hard cap [S11] | UNVERIFIED | `/quote` is "for US stocks"; international is Enterprise only [S11] | `/stock/candle` = "Premium Access Required" [S11] | "US Full SIP, LSE, Forex and Crypto" [S11] | UNVERIFIED | **Excluded: candles are premium** |
| Alpha Vantage | `apikey` query [S12] | 25 req/day [S13] | $49.99/mo = 75 req/min [S14] | Yes / `TSCO.LON` / `MBG.DEX` [S12] | Daily free (compact = 100 bars; full = premium) / intraday premium [S12] | None documented | Not documented | Premium = personal; commercial = contact sales [S12] |
| Massive (ex-Polygon) | `apiKey` query [S15] | 5 calls/min, EOD, 2 yr [S16] | Starter $29: 15-min delayed, 5 yr. Advanced $199: real-time + WS [S16] | U.S. only [S17] | Yes / yes | Advanced tier | UNVERIFIED | "Individual use only" [S16]. **US-only** |

## 2. Recommendation

### Default provider: EODHD

- Development: Free tier (build and check the symbol mapping).
- Live, daily-bar strategies: **EOD Historical All-World, $19.99/mo** (≈6,555 HUF at the ECB cross of 327.90 HUF/USD).
- $29.99/mo (≈9,834 HUF) only if an intraday strategy shows edge after costs.
- **Cost warning:** at 25,000 HUF capital, $19.99/mo is 26% of capital per month. The "edge > costs" check must include the data subscription as a fixed cost.

Daily OHLCV (I checked this live with the `demo` token on AAPL.US):

```
GET https://eodhd.com/api/eod/{CODE}.{EXCH}?api_token=KEY&fmt=json&period=d&from=YYYY-MM-DD&to=YYYY-MM-DD
→ [{"date":"2026-10-30","open":346.715,"high":349.19,"low":334.35,"close":344.63,"adjusted_close":344.63,"volume":67841555}]
```

Doc caveats [S1]: CSV is the default; no date range returns full history; an unknown ticker returns 404 and is still billed; a misspelled `fmt`, `period` or `order` silently falls back to the default, so validate these on the client.

Intraday OHLCV [S3]:

```
GET https://eodhd.com/api/intraday/{CODE}.{EXCH}?api_token=KEY&interval=1m|5m|1h&from=UNIX&to=UNIX&fmt=json
→ [{"timestamp":1793377800,"gmtoffset":0,"datetime":"2026-10-30 16:30:00","open":…,"high":…,"low":…,"close":…,"volume":…}]
```

- Max window: 1m 120 days, 5m 600 days, 1h 7,200 days. Cost: 5 calls per request.
- Data is "delayed and finalized about 2-3 hours after the market close". **Not for live signals.**
- Live prices: `GET /api/real-time/{ticker}?fmt=json` (15-min delayed; checked live: `{code,timestamp,gmtoffset,open,high,low,close,volume,previousClose,change,change_p}`), or WebSocket `wss://ws.eodhistoricaldata.com/ws/{us|eu}?api_token=` [S4].
- Responses carry `X-RateLimit-*` headers [S18].

**Data-quality caveat [S6]:** EODHD calls its prices "indicative and not appropriate for trading purposes" (aggregated from "100+ sources … via VWAP", not exchange feeds). Cross-check against T212 `Position.currentPrice` (held instruments) and `Fill.price`. Abstain on stale bars or divergence above 1% (proposed default).

### Fallback: Alpha Vantage (free key, daily only)

```
GET https://www.alphavantage.co/query?function=TIME_SERIES_DAILY&symbol=TSCO.LON&apikey=KEY
→ {"Meta Data":{…,"5. Time Zone":"US/Eastern"},"Time Series (Daily)":{"2026-10-30":{"1. open":"258.2800","2. high":…,"3. low":…,"4. close":…,"5. volume":"5741772"}}}
```

Shape checked live with `demo` on IBM (values above are IBM's); all values are strings. 25 req/day covers a last-close cross-check of about 20 core symbols, not scanning. Massive Basic is a US-only alternative.

### Mapping Trading 212 symbols

- T212 `TradableInstrument` fields: `ticker`, `isin`, `name`, `shortName`, `type` (STOCK/ETF/…), `currencyCode`, `workingScheduleId`, `extendedHours`, `maxOpenQuantity`, `addedOn`.
- There is **no exchange or MIC field**. The venue can only be found by matching `workingScheduleId` to `/equity/metadata/exchanges` (`Exchange.name`, `workingSchedules[].id`).
- The spec only shows `*_US_EQ` examples. Suffix rules such as `VUSAl_EQ` are UNVERIFIED. **Never parse the ticker string.**
- Mapping: `GET https://eodhd.com/api/search/{ISIN}?api_token=KEY&fmt=json` returns `[{Code,Exchange,Name,Type,Country,Currency,ISIN,isPrimary,previousClose,previousCloseDate}]` [S5]. Keep the row where `Exchange` matches the T212 venue (US/LSE/XETRA) **and** `Currency` equals `currencyCode`: one UCITS ISIN has listings on several venues and currencies.
- Zero or several matches: mark `UNMAPPED`, do not trade (fail closed). Store mappings with manual override and `verified_at`.
- **Pence risk:** LSE lines may quote in GBP or GBX. Whether T212 `currencyCode` ("ISO 4217") ever returns GBX is UNVERIFIED. Reject prices about 100× off the T212 price.

## 3. FX for EUR/HUF and USD/HUF

**ECB** [S19], checked live: HTTP 200, `text/xml`, gesmes envelope, EUR base, dot decimals: `<Cube time='2026-10-30'>…<Cube currency='USD' rate='1.1172'/>…<Cube currency='HUF' rate='366.33'/>`. Latest day only; `eurofxref-hist-90d.xml` gives history. Published "around 16:00 CET every working day, except on TARGET closing days", "for information purposes only" [S20]. USD/HUF = 366.33 / 1.1172 = **327.90**.

**MNB** has an official SOAP service [S21] (operations `GetCurrentExchangeRates`, `GetExchangeRates`, `GetCurrencies`, `GetCurrencyUnits`, `GetDateInterval`, `GetInfo`). Live result is escaped XML inside SOAP: `<Day date="2026-10-30"><Rate unit="1" curr="EUR">365,8</Rate>…<Rate unit="1" curr="USD">326,64</Rate>`. HUF per `unit` (JPY = 100), comma decimals, no cross needed. **Transport risk:** WSDL `soap:address` is `http://`; HTTPS POST returned 404, plain HTTP 200, so no TLS integrity.

**Recommendation:** ECB (HTTPS) primary. Fetch and store MNB daily as a second source; reject the day if they differ by more than 1% (proposed). Use MNB for tax-style reports (its legal role in Hungarian tax conversion is UNVERIFIED; confirm with a tax adviser). Neither is a transaction rate: realised P&L uses broker amounts (T212 `Fill.walletImpact`, Bitvavo EUR fills).

## 4. Revolut

- developer.revolut.com lists four APIs: Business, Merchant, Open Banking and Revolut X Crypto Exchange [S22]. **No API is documented for trading stocks or ETFs in the Revolut app.**
- **Correction to the brief:** an official crypto trading API exists, the Revolut X REST API [S23]:
  - Base `https://revx.revolut.com/api/`; Ed25519-signed headers `X-Revx-API-Key`, `X-Revx-Timestamp`, `X-Revx-Signature`.
  - `POST /1.0/orders`: `market`, `limit`, `conditional`, `tpsl`; required `client_order_id` UUID "for idempotency"; 10 req/s, 1,000/day. No withdrawal endpoints in the spec.
  - Public, no key: `/1.0/public/candles/{symbol}`, `/1.0/public/tickers`, `/2.0/public/order-book/{symbol}`.
  - UNVERIFIED: read-only key scopes; availability in Hungary.
- The requirement stays signal-only. **Do not store any Revolut key.**
- Signal-only can: compute signals from our data (EODHD; Revolut X public endpoints or Bitvavo for crypto), push instrument, side, suggested size, reason and expiry, and record trades the user enters by hand as "manual, unverified".
- It cannot: read Revolut balances, positions or fills; place or cancel orders; confirm executions; verify Revolut prices or fees.

## 5. iOS

- **Web Push:** iOS/iPadOS 16.4 added Web Push for Home Screen web apps [S24]. Needs a manifest with `display` `standalone` or `fullscreen`, and a permission request "in response to direct user interaction". Uses APNs; no Apple Developer Program needed; allow `*.push.apple.com`. Badging API also supported. EU: Apple reversed its 2024 plan to drop Home Screen web apps in the EU (seen only in search results; current status UNVERIFIED). Test on a device in Hungary.
- **Widgets: a PWA cannot provide them.** WidgetKit: "Widgets … use a widget extension you add to your Xcode project", sharing an app group with the containing app [S25]. That needs a native app built in Xcode, and Xcode 26.x needs macOS Sequoia 15.6+ [S26]. This container has no macOS.

## Sources

- S1 https://eodhd.com/financial-apis/api-for-historical-data-and-volumes
- S2 https://eodhd.com/pricing
- S3 https://eodhd.com/financial-apis/intraday-historical-data-api
- S4 https://eodhd.com/financial-apis/new-real-time-data-api-websockets
- S5 https://eodhd.com/financial-apis/search-api-for-stocks-etfs-mutual-funds
- S6 https://eodhd.com/financial-apis/commercial-vs-personal-license-use
- S7 https://twelvedata.com/docs
- S8 https://twelvedata.com/pricing
- S9 https://www.tiingo.com/about/pricing
- S10 https://apimedia.tiingo.com/docs/tiingo/daily/supported_tickers.zip
- S11 https://finnhub.io/docs/api/stock-candles
- S12 https://www.alphavantage.co/documentation/
- S13 https://www.alphavantage.co/support/
- S14 https://www.alphavantage.co/premium/
- S15 https://massive.com/docs/rest/stocks/aggregates/custom-bars.md
- S16 https://massive.com/pricing
- S17 https://massive.com/docs/rest/stocks/overview.md
- S18 https://eodhd.com/financial-apis/api-limits
- S19 https://www.ecb.europa.eu/stats/eurofxref/eurofxref-daily.xml
- S20 https://www.ecb.europa.eu/stats/policy_and_exchange_rates/euro_reference_exchange_rates/html/index.en.html
- S21 https://www.mnb.hu/arfolyamok.asmx?wsdl
- S22 https://developer.revolut.com/docs/x-api/revolut-x-crypto-exchange-rest-api
- S23 https://developer.revolut.com/docs/api/revolut-x-crypto-exchange.yml
- S24 https://webkit.org/blog/13878/web-push-for-web-apps-on-ios-and-ipados/
- S25 https://developer.apple.com/documentation/widgetkit/developing-a-widgetkit-strategy
- S26 https://developer.apple.com/support/xcode/
