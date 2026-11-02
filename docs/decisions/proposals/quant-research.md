# Proposal: Quant Research

Role: Quant Researcher. Date: 2026-11-02. Status: proposal. Every edge here is a hypothesis until it passes section 4. No return is promised.

## 0. Cost baseline

- **Bitvavo taker round trip**: `C_rt = 50 bps + spread + 2*slip`, about 60 bps for BTC/ETH and 80-120 bps for liquid alts. Exclude markets above 150 bps. Maker round trip is 30 bps, with uncertain fills.
- **Trading 212**: zero commission and a 0.15% FX fee are assumed, **not verified**. No quotes in the API; assume 10 bps ETF spread.
- **Reject** any strategy whose mean gross trade is below `2*C_rt`. This rules out bars under 1h.
- **Scale**: 1R = 0.63 EUR. At +0.2R and 5 trades/month, that is about 0.6 EUR/month. Year 1 validates the process; it is not about P&L.
- **Blocker**: T212 needs an EOD data provider. Bitvavo goes first.

## 1. Strategy families

Notation: ATR is Wilder ATR; `sigma_n` is the stdev of log returns. Signals are computed at bar close t and filled at t+1. All strategies are long-only spot; a "short" signal means cash.

**S1. Trend with volatility targeting** (Bitvavo 1d: BTC, ETH and the liquid universe; T212 1d: broad ETFs)
- Score: `s = (1/3)*sum_{L in 20,60,120} clip(ln(C_t/C_{t-L}) / (sigma_60*sqrt(L)), -1, 1)`.
- Entry: `s >= 0.3`, `C > SMA_100`, TREND_UP. Exit: `s < 0`, trail hit, or TREND_DOWN/CRISIS.
- Stop: `entry - 2.5*ATR_20` (native stopLoss). Trail: `max(H since entry) - 3*ATR_20`. No take-profit.
- Rebalance weekly, only if the weight is more than 25% off target.
- Holding 3-12 weeks; 6-12 round trips/yr per asset. Cost sensitivity low. Win rate 30-45%; the edge is in the right tail.
- Fails in: ranges, V-reversals, gaps through the stop.

**S2. Donchian breakout** (Bitvavo 4h and 1d)
- Entry: `C_t > max(H_{t-N..t-1})` (N=20 on 1d, 55 on 4h), with `z_V >= 1`, `ATR_14/ATR_100 >= 1`, and TREND_UP or TRANSITION.
- Stop: `entry - 2*ATR_14`. Trail `3*ATR_14` from +1R. Exit if `C < min(L over N/2 bars)`.
- Take-profit: sell 50% at +2R, only if the position is at least `2*N_min_eff`. Time stop: 10 bars below +0.5R.
- Holding days to 3 weeks; 15-30 round trips/yr per market. Cost sensitivity medium: 55-65% of breakouts fail.
- Fails in: ranges and thin weekends.

**S3. Range mean reversion** (BTC/ETH on Bitvavo; ETFs on T212)
- Entry: crypto `z = (C - SMA_20)/stdev(C,20) <= -2` in RANGE; ETF `RSI_2 <= 10` and `C > SMA_200`. Use a post-only limit at the bid; if unfilled after 1 bar, abstain.
- Exit at `z >= 0` (ETF: `C > SMA_5`). Stop `entry - 1.5*ATR_14`. Time stop 5 bars.
- 20-40 round trips/yr. Cost sensitivity **high**: gross edge is 50-150 bps, so it needs at least 60% maker fills.
- Fails in: a range turning into a trend. Regime-detector lag is the main risk.

**S4. Cross-sectional momentum rotation**
- Universe (point-in-time): listed 90+ days, median quote volume at least 1M EUR/day, spread at most 20 bps.
- Rank `m_i = ln(C_{t-7}/C_{t-97}) / sigma_{i,90}`. Every 14 days hold the top K with `m > 0`, `K = min(3, floor(E/(1.5*N_min_eff)))`.
- Cash if BTC is in TREND_DOWN or CRISIS. Swap only if `m_new - m_old > 0.25`. Disaster stop `3*ATR_20`.
- T212 dual momentum (monthly): best of S&P 500 ETF and ex-US ETF if its 12-1 month return beats cash; else a bond ETF. 2-4 switches/yr.
- Fails in: momentum crashes and correlated sell-offs.
- Survivorship: /markets lists only live markets. Snapshot it from now on; haircut Sharpe by 0.3 until then.

The long-term ETF core is DCA, outside this framework.

## 2. Regime detection

Computed per market and for a market proxy (BTC-EUR; S&P 500 ETF).

Inputs: ADX_14; `t_beta` = 50-bar OLS slope of ln C over its standard error; `p_sigma` = 365-bar percentile of annualized sigma_20.

States, in priority order:
1. **CRISIS**: `p_sigma >= 0.95`, `|r|/sigma_EWMA >= 4`, or breadth stress (section 6).
2. **TREND_UP**: `ADX >= 25`, `t_beta >= 2`, `C > SMA_100`.
3. **TREND_DOWN**: `ADX >= 25`, `t_beta <= -2`.
4. **RANGE**: `ADX <= 20`, `|t_beta| < 1`, `p_sigma <= 0.80`.
5. **TRANSITION**: anything else.

A new state must hold 2 closes. CRISIS starts immediately and ends after 3 calm closes. Persist states and inputs per bar.

| | TREND_UP | TRANSITION | RANGE | TREND_DOWN | CRISIS |
|---|---|---|---|---|---|
| S1 | enter | hold | hold, trail 2 ATR | exit | exit |
| S2 | enter | enter | off | exit | exit |
| S3 | off | hold | enter | exit | exit |
| S4 | enter | hold | hold | cash | cash |

Exits are allowed in every state.

## 3. Signal fusion and abstention

**Agent record** (immutable): direction `d_i` (+1 enter, 0, -1 exit), gross edge `mu_i` (bps over horizon `h_i`), uncertainty `s_i`, calibrated win probability `p_i`, `as_of`, `ttl`, `model_version`.

**Edge estimates.** `mu_i` is the mean out-of-sample (OOS) gross trade return for the strategy x regime bucket; `s_i = SD/sqrt(n)`. Small n means wide uncertainty, so the system abstains.

**Calibration.** Isotonic regression on OOS data only. An agent no better than the base-rate Brier score gets weight 0.

**Fusion.**
- Weight: `w_i = rho_i/s_i^2`, where `rho_i` in [0,1] is reliability (new agent: 0.5).
- Edge: `mu_hat = sum(w_i*d_i*mu_i)/sum(w_i)`.
- Uncertainty: `sigma_hat^2 = (1/sum(w_i))*(1+(n-1)*rho_bar)`, with error correlation `rho_bar` defaulting to 0.5. Agents share price data, so they are not independent.

**Costs.** `C_hat` = live /account fees for both legs + book spread `(ask-bid)/mid*1e4` + `2*(book-walk VWAP vs mid + 5 bps)` + FX where currencies differ.

**Entry rule.** All three must hold:
- `mu_hat - C_hat - k*sigma_hat > 0`, with k = 1.0 (configurable).
- Agreement `A = sum(w_i*[d_i = sign(mu_hat)])/sum(w_i) >= 0.65`.
- The risk engine approves.

Report expected net edge `mu_hat - C_hat` in bps and confidence `Phi((mu_hat - C_hat)/sigma_hat)`.

**Example.** `mu = 250`, `C = 60`, n = 30, SD = 800 gives sigma = 146. Then 250 - 60 - 146 = +44: trade. With n = 15, sigma = 207 and the result is -17: abstain.

**Conflicts.** Exits from the owning strategy or risk engine need no quorum. Any enter-vs-exit conflict blocks entry.

**Staleness.**
- Past `as_of + ttl` (next close + 10 min on 1d, + 5 min on 4h), weight is 0.
- Reject if the book is over 2 s old, a candle is missing, or clock skew exceeds 1 s.
- Re-evaluate if price moved more than `0.5*ATR_14` or 100 bps since `as_of`.
- Abstain if live weight is under 50% of design weight.

Log each abstention with a reason: `EDGE_BELOW_COST`, `LOW_AGREEMENT`, `STALE`, `MIN_ORDER_RISK`, `REGIME_GATE`.

## 4. Validation protocol

**Data.** Bitvavo candles, our own book snapshots, point-in-time universe snapshots. T212 needs split- and dividend-adjusted EOD bars.

**Walk-forward.** 1d: 24-month in-sample (IS), 6-month OOS, 6-month step. 4h: 12-month IS, 3-month OOS. Embargo = max holding period. Only concatenated OOS counts.

**Search.** At most 20 pre-registered parameter sets per family. Pick a plateau, not the peak. Log every trial; the count N feeds deflation.

**Fill model.** Next open. S3 limits fill only if the next low trades 1 tick through. Taker 25 bps per side. Recorded half-spread per side (fallback 5 bps BTC/ETH, 20 bps others). Slippage = book walk + 5 bps. Gapped stops fill at `min(stop, open) - 0.25*ATR_14`.

**Metrics** (net): net and annualized return; Sharpe, Sortino, Calmar; max drawdown and duration; win rate; `PF = sum(gains)/|sum(losses)|`; expectancy `p*W_avg - (1-p)*L_avg` in bps and R; exposure, turnover, holding time, cost share.

**Deflated Sharpe Ratio.**
- `SR0 = sqrt(V[SR_trials])*((1-g)*Phi^-1(1-1/N) + g*Phi^-1(1-1/(N*e)))`, with g = 0.5772.
- `DSR = Phi((SR-SR0)*sqrt(T-1)/sqrt(1 - g3*SR + (g4-1)/4*SR^2))`, where g3 and g4 are return skewness and kurtosis.
- Require `DSR >= 0.95`. If there are too few trials, use Bonferroni: t-stat of the mean trade `>= Phi^-1(1-0.05/N)`.

**Promotion** (all on OOS): 30+ trades (1d, pooled across markets) or 100+ (4h); `PF >= 1.3`; expectancy at least +0.15R; Sharpe at least 0.7; max drawdown at most 25%; 60%+ of folds positive; OOS/IS Sharpe at least 0.5; `PF >= 1.05` at 2x costs; `DSR >= 0.95`.

**Incubation** (no paper mode exists): first 20 trades or 60 days at f = 0.5%, `rho = 0.5`.

**Demotion** (any one): live drawdown over 1.5x OOS max; after 10+ trades, live mean R under OOS mean minus 2 SE; realized cost over 1.5x modeled; 3 negative months with 6+ trades.

Return requires a fresh OOS pass. Regime-gated inactivity is not degradation. Re-run walk-forward quarterly.

## 5. Position sizing

**Inputs.** `E` = trading equity in EUR, excluding the ETF core. Risk per trade `f = 1%`, ceiling `f_max = 1.25%`. Stop distance `D = P - stop`.

**Size.**
- By risk: `N_risk = f*E*P/D`.
- By volatility: `N_vol = E*(0.20/K)/sigma_i`. This assumes correlation 1, which is conservative.
- Final: `N = min(N_risk, N_vol, 0.5*E, cash)`.
- Limits: open risk at most 3% of E, gross exposure at most 100%.

**Minimum order.**
- `N_min = max(minOrderInQuoteAsset, minOrderInBaseAsset*P)`.
- The exit at the stop must also clear the minimum: `N_min_eff = 1.05*N_min/(1 - D/P)`.
- If `N >= N_min_eff`, trade N, rounded down to quantityDecimals.
- Otherwise compute `f' = N_min_eff*(D/P)/E`. Trade `N_min_eff` if `f' <= f_max`; otherwise **abstain** (`MIN_ORDER_RISK`).

**Example** at E = 63 EUR:
- BTC, 6% stop: N = 10.50 EUR, above `N_min_eff` = 5.59. Trade.
- Alt, 15% stop: N = 4.20 EUR, below 6.18. f' = 1.47%. Abstain.
- So only stops of 13% or less are tradable until deposits grow E.

**Trading 212.** `Q = floor_step(N/P_ext)`, sent as a limit at `P_ext*(1 + 30 bps)`. The v0 spec has no step or min-quantity field: **verify**. On a 400, abstain. Never retry blindly: orders are not idempotent.

**Drawdown throttle** (time-weighted, deposits excluded). Above 10%, halve f. Above 20%, no new entries until manual review.

## 6. News, sentiment and new listings (free data only)

**Listings.** Poll GET /markets every 60 s, diff against the stored set, and record `first_seen` and status changes. No trading for 7 days; then the 90-day universe filter applies. Listing momentum stays research-only until 30+ OOS events exist (likely years).

**Shocks.**
- `z_V = (ln V - mean(ln V,30))/sd(ln V,30)`; `z_r = r/sigma_EWMA`.
- If `z_V >= 3` or `|z_r| >= 4`, block entries for 2 bars and tighten trails.
- If 30%+ of the universe has `z_r <= -3`, enter CRISIS.
- Reject orders when spread exceeds 3x its 7-day median or depth is under 10x notional.
- No earnings dates without a provider, so ETFs only.

**LLM sentiment.** It classifies free headlines and exchange announcements. Weight at most 10%; never the sole trigger; never opens a position. It may only veto or reduce, on a hack, delisting, regulatory action or outage. Outputs are cached with prompt and model versions for replay. Weight rises only after 100+ events show OOS information coefficient above 0 with t >= 2.

## Open questions

1. Which EOD data provider for T212?
2. T212's actual FX fee and quantity steps?
3. Is research telemetry on unpromoted strategies the forbidden paper mode?
