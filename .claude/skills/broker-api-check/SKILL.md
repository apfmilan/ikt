---
name: broker-api-check
description: Re-verify Trading 212 or Bitvavo API behaviour against official documentation before changing a connector. Use whenever connector code, order types, rate limits or auth are touched.
---
# Verify broker API against official docs

Trading 212
- `curl -sSL "https://docs.trading212.com/_bundle/api.json?download" -o /tmp/t212.json`
- List endpoints + rate limits: `node -e 'const s=require("/tmp/t212.json");for(const[p,o]of Object.entries(s.paths))for(const[m,x]of Object.entries(o))console.log(m,p,(x.description||"").match(/Rate limit: [^\n]*/)?.[0]||"")'`
- Compare against `crates/tc-broker-t212/src/endpoints.rs` (paths, limits) and request/response structs.

Bitvavo
- Pages: https://docs.bitvavo.com/docs/rest-api/create-order/ , /docs/rest-api/introduction/ (signing), /docs/rate-limits/ , /docs/errors/
- Public endpoint smoke check (no key): `curl -s https://api.bitvavo.com/v2/markets?market=BTC-EUR`
- Compare against `crates/tc-broker-bitvavo/src/`.

Rules: never call private/order endpoints from a dev machine or CI. Record any doc change in the connector's module docs with the date checked.
