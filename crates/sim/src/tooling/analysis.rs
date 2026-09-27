//! Pre-registered estimators as pure, unit-tested functions.

use crate::smr::SpreadPoint;

fn gamma(p: &SpreadPoint) -> Option<f64> {
    (p.useful_total > 0).then(|| f64::from(p.useful_holders) / f64::from(p.useful_total))
}

/// First contiguous run of Lemma-3.1 base points (useful ≥ c_win·log₂ n, γ ≤ 1/3), inclusive indices.
pub fn growth_window(spread: &[SpreadPoint], c_win: f64, n: usize) -> Option<(usize, usize)> {
    let threshold = c_win * (n as f64).log2();
    let in_window = |p: &SpreadPoint| {
        f64::from(p.useful_holders) >= threshold && gamma(p).is_some_and(|g| g <= 1.0 / 3.0)
    };
    let start = spread.iter().position(in_window)?;
    let end = spread[start..]
        .iter()
        .position(|p| !in_window(p))
        .map_or(spread.len() - 1, |off| start + off - 1);
    Some((start, end))
}

/// Geometric mean of γ_{t+1}/γ_t over in-window base points; None below 3 ratios.
pub fn geometric_mean_growth(spread: &[SpreadPoint], window: (usize, usize)) -> Option<f64> {
    let (start, end) = window;
    let mut log_sum = 0.0;
    let mut count = 0usize;
    for base in start..=end.min(spread.len().saturating_sub(2)) {
        let (g0, g1) = (gamma(&spread[base])?, gamma(&spread[base + 1])?);
        if g0 > 0.0 && g1 > 0.0 {
            log_sum += (g1 / g0).ln();
            count += 1;
        }
    }
    (count >= 3).then(|| (log_sum / count as f64).exp())
}

pub fn ols_fit(xs: &[f64], ys: &[f64]) -> Option<(f64, f64, f64)> {
    assert_eq!(xs.len(), ys.len(), "paired samples");
    let n = xs.len();
    if n < 2 {
        return None;
    }
    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
    let (mx, my) = (mean(xs), mean(ys));
    let sxx: f64 = xs.iter().map(|x| (x - mx).powi(2)).sum();
    if sxx == 0.0 {
        return None;
    }
    let sxy: f64 = xs.iter().zip(ys).map(|(x, y)| (x - mx) * (y - my)).sum();
    let slope = sxy / sxx;
    let intercept = my - slope * mx;
    let ss_tot: f64 = ys.iter().map(|y| (y - my).powi(2)).sum();
    let ss_res: f64 = xs
        .iter()
        .zip(ys)
        .map(|(x, y)| (y - (slope * x + intercept)).powi(2))
        .sum();
    let r2 = if ss_tot == 0.0 {
        1.0
    } else {
        1.0 - ss_res / ss_tot
    };
    Some((slope, intercept, r2))
}
