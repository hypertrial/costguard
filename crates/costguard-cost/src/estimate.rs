//! Lognormal cost estimates in log-space (product = sum of mu/sigma).

const DEFAULT_CV: f64 = 0.5;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Estimate {
    pub mu: f64,
    pub sigma: f64,
}

impl Estimate {
    pub fn from_point(value: f64, cv: Option<f64>) -> Self {
        let cv = cv.unwrap_or(DEFAULT_CV);
        let sigma = (1.0 + cv * cv).ln().sqrt();
        Self {
            mu: value.ln(),
            sigma,
        }
    }

    pub fn from_range(p10: f64, p90: f64) -> Self {
        let z = normal_quantile(0.9) - normal_quantile(0.1);
        let sigma = (p90.ln() - p10.ln()) / z;
        let mu = (p10.ln() + p90.ln()) / 2.0;
        Self { mu, sigma }
    }

    pub fn quantile(&self, p: f64) -> f64 {
        (self.mu + self.sigma * normal_quantile(p)).exp()
    }

    pub fn median(&self) -> f64 {
        self.quantile(0.5)
    }

    pub fn interval(&self, coverage: f64) -> (f64, f64) {
        let tail = (1.0 - coverage) / 2.0;
        (self.quantile(tail), self.quantile(1.0 - tail))
    }

    pub fn variance(&self) -> f64 {
        let sigma2 = self.sigma * self.sigma;
        ((2.0 * self.mu + sigma2).exp()) * (sigma2.exp() - 1.0)
    }

    pub fn mean(&self) -> f64 {
        (self.mu + 0.5 * self.sigma * self.sigma).exp()
    }
}

/// Sum independent lognormals via moment matching (Fenton–Wilkinson).
pub fn sum_lognormals(estimates: &[Estimate]) -> Estimate {
    if estimates.is_empty() {
        return Estimate::from_point(1.0, Some(0.5));
    }
    let mut sum_mean = 0.0;
    let mut sum_var = 0.0;
    for estimate in estimates {
        sum_mean += estimate.mean();
        sum_var += estimate.variance();
    }
    if sum_mean <= 0.0 {
        return Estimate::from_point(0.001, Some(0.5));
    }
    let cv = (sum_var.sqrt() / sum_mean).clamp(0.01, 2.0);
    Estimate::from_point(sum_mean, Some(cv))
}

pub fn excess_multiplier(multiplier: Estimate) -> Estimate {
    let p10 = (multiplier.quantile(0.1) - 1.0).max(0.0);
    let p50 = (multiplier.median() - 1.0).max(0.0);
    let p90 = (multiplier.quantile(0.9) - 1.0).max(0.0);
    if p50 <= 0.0 {
        return Estimate::from_point(0.001, Some(0.5));
    }
    if p90 > p10 && p10 > 0.0 {
        Estimate::from_range(p10.max(0.001), p90)
    } else {
        Estimate::from_point(p50, Some(0.3))
    }
}

pub fn gb_months_from_bytes_runs(bytes: Estimate, runs: Estimate) -> f64 {
    let bytes_per_month = bytes * runs;
    bytes_per_month.median() / 1_000_000_000.0
}

impl std::ops::Mul for Estimate {
    type Output = Self;

    fn mul(self, rhs: Self) -> Self {
        Self {
            mu: self.mu + rhs.mu,
            sigma: (self.sigma * self.sigma + rhs.sigma * rhs.sigma).sqrt(),
        }
    }
}

impl std::ops::Div for Estimate {
    type Output = Self;

    fn div(self, rhs: Self) -> Self {
        Self {
            mu: self.mu - rhs.mu,
            sigma: (self.sigma * self.sigma + rhs.sigma * rhs.sigma).sqrt(),
        }
    }
}

/// Acklam inverse normal CDF approximation (no external deps).
#[allow(clippy::excessive_precision)]
fn normal_quantile(p: f64) -> f64 {
    if p <= 0.0 {
        return f64::NEG_INFINITY;
    }
    if p >= 1.0 {
        return f64::INFINITY;
    }

    const A: [f64; 6] = [
        -3.969683028665376e+01,
        2.209460984245205e+02,
        -2.759285469016765e+02,
        1.383577518672690e+02,
        -3.066479806614716e+01,
        2.506628277459239e+00,
    ];
    const B: [f64; 5] = [
        -5.447609879822406e+01,
        1.615858368580409e+02,
        -1.556989775598873e+02,
        6.680131188771972e+01,
        -1.328068155288572e+01,
    ];
    const C: [f64; 6] = [
        -7.784894002430293e-03,
        -3.223964580411865e-01,
        -2.400758277161838e+00,
        -2.549732539343734e+00,
        4.374664141464968e+00,
        2.938163982698783e+00,
    ];
    const D: [f64; 4] = [
        7.784695709041462e-03,
        3.224671290700398e-01,
        2.445134137142996e+00,
        3.754408661907416e+00,
    ];

    const P_LOW: f64 = 0.02425;
    const P_HIGH: f64 = 1.0 - P_LOW;

    if p < P_LOW {
        let q = (-2.0 * p.ln()).sqrt();
        (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    } else if p > P_HIGH {
        let q = (-2.0 * (1.0 - p).ln()).sqrt();
        -(((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    } else {
        let q = p - 0.5;
        let r = q * q;
        (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q
            / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
    }
}

/// Round to two significant figures for display.
pub fn round_sig2(value: f64) -> f64 {
    if !value.is_finite() || value == 0.0 {
        return value;
    }
    let magnitude = value.abs().log10().floor();
    let scale = 10_f64.powf(1.0 - magnitude);
    (value * scale).round() / scale
}

/// Human-readable USD/month (e.g. "~$400/mo ($90–$1.9k)").
pub fn format_usd_interval(p10: f64, p50: f64, p90: f64) -> String {
    format!(
        "~{}/mo ({}–{})",
        format_usd(p50),
        format_usd(p10),
        format_usd(p90)
    )
}

pub fn format_usd(value: f64) -> String {
    let rounded = round_sig2(value);
    if rounded >= 1_000_000.0 {
        format!("${:.0}M", rounded / 1_000_000.0)
    } else if rounded >= 10_000.0 {
        format!("${:.0}k", rounded / 1_000.0)
    } else if rounded >= 1_000.0 {
        format!("${:.1}k", rounded / 1_000.0)
    } else {
        format!("${:.0}", rounded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_of_lognormals_sums_log_params() {
        let a = Estimate::from_point(100.0, None);
        let b = Estimate::from_point(10.0, None);
        let product = a * b;
        assert!((product.median() - 1000.0).abs() < 50.0);
    }

    #[test]
    fn from_range_contains_endpoints() {
        let est = Estimate::from_range(10.0, 1000.0);
        assert!((est.quantile(0.1) - 10.0).abs() < 1.0);
        assert!((est.quantile(0.9) - 1000.0).abs() < 50.0);
    }

    #[test]
    fn interval_80_percent() {
        let est = Estimate::from_point(100.0, Some(0.5));
        let (lo, hi) = est.interval(0.8);
        assert!(lo < est.median());
        assert!(hi > est.median());
    }

    #[test]
    fn sum_lognormals_moment_matches() {
        let estimates = vec![
            Estimate::from_point(100.0, Some(0.2)),
            Estimate::from_point(200.0, Some(0.2)),
        ];
        let total = super::sum_lognormals(&estimates);
        assert!((total.mean() - 300.0).abs() < 30.0);
    }

    #[test]
    fn format_usd_readable() {
        assert_eq!(format_usd(412.37), "$410");
        assert_eq!(format_usd(1900.0), "$1.9k");
    }
}
