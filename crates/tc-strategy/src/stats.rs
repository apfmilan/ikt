//! Small numerical helpers: normal CDF / inverse CDF, moments.

/// Standard normal CDF (Abramowitz–Stegun 7.1.26 via erf, |error| < 1.5e-7).
pub fn norm_cdf(x: f64) -> f64 {
    0.5 * (1.0 + erf(x / std::f64::consts::SQRT_2))
}

fn erf(x: f64) -> f64 {
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let x = x.abs();
    let t = 1.0 / (1.0 + 0.327_591_1 * x);
    let y = 1.0
        - (((((1.061_405_429 * t - 1.453_152_027) * t) + 1.421_413_741) * t - 0.284_496_736) * t
            + 0.254_829_592)
            * t
            * (-x * x).exp();
    sign * y
}

/// Inverse standard normal CDF (Acklam's rational approximation, rel. error < 1.2e-9).
pub fn norm_inv(p: f64) -> f64 {
    if p <= 0.0 {
        return f64::NEG_INFINITY;
    }
    if p >= 1.0 {
        return f64::INFINITY;
    }
    const A: [f64; 6] = [
        -3.969_683_028_665_376e1,
        2.209_460_984_245_205e2,
        -2.759_285_104_469_687e2,
        1.383_577_518_672_69e2,
        -3.066_479_806_614_716e1,
        2.506_628_277_459_239,
    ];
    const B: [f64; 5] = [
        -5.447_609_879_822_406e1,
        1.615_858_368_580_409e2,
        -1.556_989_798_598_866e2,
        6.680_131_188_771_972e1,
        -1.328_068_155_288_572e1,
    ];
    const C: [f64; 6] = [
        -7.784_894_002_430_293e-3,
        -3.223_964_580_411_365e-1,
        -2.400_758_277_161_838,
        -2.549_732_539_343_734,
        4.374_664_141_464_968,
        2.938_163_982_698_783,
    ];
    const D: [f64; 4] = [
        7.784_695_709_041_462e-3,
        3.224_671_290_700_398e-1,
        2.445_134_137_142_996,
        3.754_408_661_907_416,
    ];
    let plow = 0.02425;
    if p < plow {
        let q = (-2.0 * p.ln()).sqrt();
        (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    } else if p <= 1.0 - plow {
        let q = p - 0.5;
        let r = q * q;
        (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q
            / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
    } else {
        let q = (-2.0 * (1.0 - p).ln()).sqrt();
        -(((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    }
}

pub fn mean(x: &[f64]) -> f64 {
    if x.is_empty() {
        0.0
    } else {
        x.iter().sum::<f64>() / x.len() as f64
    }
}

/// Sample standard deviation (0 for fewer than 2 points).
pub fn sd(x: &[f64]) -> f64 {
    if x.len() < 2 {
        return 0.0;
    }
    let m = mean(x);
    (x.iter().map(|a| (a - m).powi(2)).sum::<f64>() / (x.len() as f64 - 1.0)).sqrt()
}

/// Skewness and (non-excess) kurtosis.
pub fn skew_kurt(x: &[f64]) -> (f64, f64) {
    let n = x.len() as f64;
    if n < 3.0 {
        return (0.0, 3.0);
    }
    let m = mean(x);
    let m2 = x.iter().map(|a| (a - m).powi(2)).sum::<f64>() / n;
    if m2 == 0.0 {
        return (0.0, 3.0);
    }
    let m3 = x.iter().map(|a| (a - m).powi(3)).sum::<f64>() / n;
    let m4 = x.iter().map(|a| (a - m).powi(4)).sum::<f64>() / n;
    (m3 / m2.powf(1.5), m4 / (m2 * m2))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cdf_and_inverse() {
        assert!((norm_cdf(0.0) - 0.5).abs() < 1e-7);
        assert!((norm_cdf(1.96) - 0.975).abs() < 1e-4);
        assert!((norm_inv(0.975) - 1.959964).abs() < 1e-5);
        assert!((norm_inv(0.01) + 2.326348).abs() < 1e-5);
        for p in [0.001, 0.2, 0.5, 0.8, 0.999] {
            assert!((norm_cdf(norm_inv(p)) - p).abs() < 1e-6);
        }
    }

    #[test]
    fn moments() {
        let x = [1.0, 2.0, 3.0, 4.0, 5.0];
        assert!((mean(&x) - 3.0).abs() < 1e-12);
        let (s, k) = skew_kurt(&x);
        assert!(s.abs() < 1e-12);
        assert!((k - 1.7).abs() < 1e-9);
    }
}
