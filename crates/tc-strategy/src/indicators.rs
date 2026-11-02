//! Technical indicators on `f64` series (statistics, not balances). All functions return
//! `None` when there is not enough data instead of guessing.

/// Simple moving average of the last `n` values.
pub fn sma(x: &[f64], n: usize) -> Option<f64> {
    if n == 0 || x.len() < n {
        return None;
    }
    Some(x[x.len() - n..].iter().sum::<f64>() / n as f64)
}

/// Sample standard deviation of the last `n` values.
pub fn stdev(x: &[f64], n: usize) -> Option<f64> {
    if n < 2 || x.len() < n {
        return None;
    }
    let w = &x[x.len() - n..];
    let m = w.iter().sum::<f64>() / n as f64;
    let v = w.iter().map(|a| (a - m).powi(2)).sum::<f64>() / (n as f64 - 1.0);
    Some(v.sqrt())
}

/// Log returns `ln(c_t / c_{t-1})`.
pub fn log_returns(c: &[f64]) -> Vec<f64> {
    c.windows(2)
        .map(|w| {
            if w[0] > 0.0 && w[1] > 0.0 {
                (w[1] / w[0]).ln()
            } else {
                0.0
            }
        })
        .collect()
}

/// Wilder's true range series (first element uses high-low).
pub fn true_range(h: &[f64], l: &[f64], c: &[f64]) -> Vec<f64> {
    (0..h.len())
        .map(|i| {
            if i == 0 {
                h[0] - l[0]
            } else {
                let pc = c[i - 1];
                (h[i] - l[i]).max((h[i] - pc).abs()).max((l[i] - pc).abs())
            }
        })
        .collect()
}

/// Wilder-smoothed average (RMA) over the whole series; returns the final value.
fn wilder(x: &[f64], n: usize) -> Option<f64> {
    if n == 0 || x.len() < n {
        return None;
    }
    let mut a = x[..n].iter().sum::<f64>() / n as f64;
    for v in &x[n..] {
        a = (a * (n as f64 - 1.0) + v) / n as f64;
    }
    Some(a)
}

/// Wilder ATR(n), last value.
pub fn atr(h: &[f64], l: &[f64], c: &[f64], n: usize) -> Option<f64> {
    if h.len() < n + 1 {
        return None;
    }
    wilder(&true_range(h, l, c)[1..], n)
}

/// RSI(n) with Wilder smoothing, last value.
pub fn rsi(c: &[f64], n: usize) -> Option<f64> {
    if c.len() < n + 1 {
        return None;
    }
    let d: Vec<f64> = c.windows(2).map(|w| w[1] - w[0]).collect();
    let gains: Vec<f64> = d.iter().map(|x| x.max(0.0)).collect();
    let losses: Vec<f64> = d.iter().map(|x| (-x).max(0.0)).collect();
    let g = wilder(&gains, n)?;
    let l = wilder(&losses, n)?;
    if l == 0.0 {
        return Some(100.0);
    }
    Some(100.0 - 100.0 / (1.0 + g / l))
}

/// ADX(n), last value (Wilder).
pub fn adx(h: &[f64], l: &[f64], c: &[f64], n: usize) -> Option<f64> {
    let len = h.len();
    if len < 2 * n + 1 {
        return None;
    }
    let mut pdm = Vec::with_capacity(len - 1);
    let mut ndm = Vec::with_capacity(len - 1);
    for i in 1..len {
        let up = h[i] - h[i - 1];
        let down = l[i - 1] - l[i];
        pdm.push(if up > down && up > 0.0 { up } else { 0.0 });
        ndm.push(if down > up && down > 0.0 { down } else { 0.0 });
    }
    let tr = &true_range(h, l, c)[1..];
    // Wilder smoothing producing series.
    let smooth = |x: &[f64]| -> Vec<f64> {
        let mut out = Vec::with_capacity(x.len());
        let mut a = x[..n].iter().sum::<f64>();
        out.push(a);
        for v in &x[n..] {
            a = a - a / n as f64 + v;
            out.push(a);
        }
        out
    };
    let str_ = smooth(tr);
    let sp = smooth(&pdm);
    let sn = smooth(&ndm);
    let dx: Vec<f64> = (0..str_.len())
        .map(|i| {
            if str_[i] == 0.0 {
                return 0.0;
            }
            let pdi = 100.0 * sp[i] / str_[i];
            let ndi = 100.0 * sn[i] / str_[i];
            let s = pdi + ndi;
            if s == 0.0 {
                0.0
            } else {
                100.0 * (pdi - ndi).abs() / s
            }
        })
        .collect();
    wilder(&dx, n)
}

/// Highest value of `x[len-1-n .. len-1]` (the `n` bars before the last).
pub fn prior_max(x: &[f64], n: usize) -> Option<f64> {
    if x.len() < n + 1 {
        return None;
    }
    x[x.len() - 1 - n..x.len() - 1]
        .iter()
        .copied()
        .reduce(f64::max)
}

/// Lowest value of the last `n` values (inclusive of the last bar).
pub fn min_last(x: &[f64], n: usize) -> Option<f64> {
    if n == 0 || x.len() < n {
        return None;
    }
    x[x.len() - n..].iter().copied().reduce(f64::min)
}

/// t-statistic of the OLS slope of `ln(c)` over the last `n` bars.
pub fn trend_tstat(c: &[f64], n: usize) -> Option<f64> {
    if n < 3 || c.len() < n || c[c.len() - n..].iter().any(|v| *v <= 0.0) {
        return None;
    }
    let y: Vec<f64> = c[c.len() - n..].iter().map(|v| v.ln()).collect();
    let nf = n as f64;
    let xm = (nf - 1.0) / 2.0;
    let ym = y.iter().sum::<f64>() / nf;
    let sxx: f64 = (0..n).map(|i| (i as f64 - xm).powi(2)).sum();
    let sxy: f64 = (0..n).map(|i| (i as f64 - xm) * (y[i] - ym)).sum();
    let beta = sxy / sxx;
    let alpha = ym - beta * xm;
    let rss: f64 = (0..n)
        .map(|i| (y[i] - alpha - beta * i as f64).powi(2))
        .sum();
    let se = (rss / (nf - 2.0) / sxx).sqrt();
    if se == 0.0 {
        return Some(if beta > 0.0 {
            f64::INFINITY
        } else if beta < 0.0 {
            f64::NEG_INFINITY
        } else {
            0.0
        });
    }
    Some(beta / se)
}

/// Percentile rank of the last value within the last `n` values, in [0, 1].
pub fn percentile_rank(x: &[f64], n: usize) -> Option<f64> {
    if n < 2 || x.len() < n {
        return None;
    }
    let w = &x[x.len() - n..];
    let last = *w.last()?;
    let below = w.iter().filter(|v| **v < last).count();
    Some(below as f64 / (n - 1) as f64)
}

/// Rolling series of annualised volatility from `window`-bar stdev of log returns.
pub fn rolling_vol(c: &[f64], window: usize, bars_per_year: f64) -> Vec<f64> {
    let r = log_returns(c);
    if r.len() < window {
        return vec![];
    }
    (window..=r.len())
        .filter_map(|end| stdev(&r[..end], window).map(|s| s * bars_per_year.sqrt()))
        .collect()
}

/// EWMA volatility of log returns (lambda 0.94), last value.
pub fn ewma_vol(c: &[f64]) -> Option<f64> {
    let r = log_returns(c);
    if r.len() < 20 {
        return None;
    }
    let lambda: f64 = 0.94;
    let mut var = r[..20].iter().map(|x| x * x).sum::<f64>() / 20.0;
    for x in &r[20..r.len() - 1] {
        var = lambda * var + (1.0 - lambda) * x * x;
    }
    Some(var.sqrt())
}

/// Z-score of the last value of `ln(x)` vs the previous `n` values (volume shocks).
pub fn log_zscore(x: &[f64], n: usize) -> Option<f64> {
    if x.len() < n + 1 || x[x.len() - n - 1..].iter().any(|v| *v <= 0.0) {
        return None;
    }
    let hist: Vec<f64> = x[x.len() - 1 - n..x.len() - 1]
        .iter()
        .map(|v| v.ln())
        .collect();
    let m = hist.iter().sum::<f64>() / n as f64;
    let sd = stdev(&hist, n)?;
    if sd == 0.0 {
        return None;
    }
    Some(((x[x.len() - 1]).ln() - m) / sd)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn sma_stdev() {
        let x = [1.0, 2.0, 3.0, 4.0];
        assert!(approx(sma(&x, 2).unwrap(), 3.5, 1e-12));
        assert!(approx(stdev(&x, 4).unwrap(), 1.2909944487, 1e-9));
        assert!(sma(&x, 5).is_none());
    }

    #[test]
    fn rsi_extremes() {
        let up: Vec<f64> = (1..=30).map(|i| i as f64).collect();
        assert!(approx(rsi(&up, 14).unwrap(), 100.0, 1e-9));
        let mut zz = vec![];
        for i in 0..40 {
            zz.push(if i % 2 == 0 { 10.0 } else { 11.0 });
        }
        let r = rsi(&zz, 14).unwrap();
        assert!(r > 40.0 && r < 60.0, "{r}");
    }

    #[test]
    fn atr_constant_range() {
        let h = vec![11.0; 30];
        let l = vec![9.0; 30];
        let c = vec![10.0; 30];
        assert!(approx(atr(&h, &l, &c, 14).unwrap(), 2.0, 1e-9));
    }

    #[test]
    fn adx_trend_vs_chop() {
        let n = 80;
        let c: Vec<f64> = (0..n).map(|i| 100.0 + i as f64).collect();
        let h: Vec<f64> = c.iter().map(|x| x + 0.5).collect();
        let l: Vec<f64> = c.iter().map(|x| x - 0.5).collect();
        let trend = adx(&h, &l, &c, 14).unwrap();
        let c2: Vec<f64> = (0..n)
            .map(|i| if i % 2 == 0 { 100.0 } else { 101.0 })
            .collect();
        let h2: Vec<f64> = c2.iter().map(|x| x + 0.5).collect();
        let l2: Vec<f64> = c2.iter().map(|x| x - 0.5).collect();
        let chop = adx(&h2, &l2, &c2, 14).unwrap();
        assert!(trend > 50.0, "trend adx {trend}");
        assert!(chop < 20.0, "chop adx {chop}");
    }

    #[test]
    fn tstat_sign() {
        let up: Vec<f64> = (0..60)
            .map(|i| 100.0 * (1.0 + 0.01 * i as f64 + 0.002 * ((i * 7) % 3) as f64))
            .collect();
        assert!(trend_tstat(&up, 50).unwrap() > 2.0);
        let down: Vec<f64> = up.iter().rev().copied().collect();
        assert!(trend_tstat(&down, 50).unwrap() < -2.0);
    }

    #[test]
    fn percentile_and_donchian() {
        let x = [1.0, 2.0, 3.0, 4.0, 5.0];
        assert!(approx(percentile_rank(&x, 5).unwrap(), 1.0, 1e-12));
        assert_eq!(prior_max(&x, 3), Some(4.0));
        assert_eq!(min_last(&x, 2), Some(4.0));
    }

    #[test]
    fn volume_shock() {
        let mut v = vec![];
        for i in 0..31 {
            v.push(100.0 + (i % 5) as f64);
        }
        v.push(1000.0);
        assert!(log_zscore(&v, 30).unwrap() > 3.0);
    }
}
