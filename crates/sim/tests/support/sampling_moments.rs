//! Empirical validation of Weighted{weights, range} sampling: the engine's
//! draws must match the intended mixture-of-uniform-slices law in mean,
//! variance, and per-bucket mass. Seeded, so deterministic — a failure is a
//! sampler bug, not noise.

use sim::{Config, Init, SimState};

struct Law {
    mean: f64,
    var: f64,
    bucket_p: Vec<f64>,
    slice: Vec<(u64, u64)>,
}

/// Exact moments of the intended law: bucket i (prob w_i/Σw) uniform on
/// [lo_i, hi_i) with lo_i = ⌊R·i/m⌋.
fn law(weights: &[f64], range: u64) -> Law {
    let m = weights.len() as u128;
    let total: f64 = weights.iter().sum();
    let slice: Vec<(u64, u64)> = (0..weights.len())
        .map(|i| {
            let lo = (range as u128 * i as u128 / m) as u64;
            let hi = (range as u128 * (i as u128 + 1) / m) as u64;
            (lo, hi)
        })
        .collect();
    let bucket_p: Vec<f64> = weights.iter().map(|w| w / total).collect();
    let mut mean = 0.0;
    let mut ex2 = 0.0;
    for (p, (lo, hi)) in bucket_p.iter().zip(&slice) {
        let (lo, d) = (*lo as f64, (hi - lo) as f64);
        // uniform on {lo .. lo+d-1}: E = lo + (d-1)/2,
        // E[X²] = lo² + lo(d-1) + (d-1)(2d-1)/6
        mean += p * (lo + (d - 1.0) / 2.0);
        ex2 += p * (lo * lo + lo * (d - 1.0) + (d - 1.0) * (2.0 * d - 1.0) / 6.0);
    }
    Law {
        mean,
        var: ex2 - mean * mean,
        bucket_p,
        slice,
    }
}

fn sample(weights: Vec<f64>, range: u64, n: usize, seed: u64) -> Vec<u64> {
    let state = SimState::new(
        n,
        Config::default(),
        &Init::Weighted { weights, range },
        seed,
        true,
        None,
    );
    state
        .trace()
        .expect("recording")
        .initial
        .iter()
        .map(|v| v.expect("weighted init leaves nobody undecided"))
        .collect()
}

fn check(weights: &[f64], range: u64, seed: u64) {
    let n = 1_000_000;
    let l = law(weights, range);
    let xs = sample(weights.to_vec(), range, n, seed);

    let mean = xs.iter().map(|&x| x as f64).sum::<f64>() / n as f64;
    let var = xs.iter().map(|&x| (x as f64 - mean).powi(2)).sum::<f64>() / (n as f64 - 1.0);

    let sd = l.var.sqrt();
    let mean_tol = 4.0 * sd / (n as f64).sqrt();
    assert!(
        (mean - l.mean).abs() <= mean_tol,
        "mean off: sample {mean}, law {} (tol {mean_tol}) for w={weights:?} R={range}",
        l.mean
    );
    assert!(
        (var / l.var - 1.0).abs() <= 0.02,
        "variance off: sample {var}, law {} for w={weights:?} R={range}",
        l.var
    );

    // per-bucket occupancy vs law, ~5σ binomial bounds; zero-weight exact
    let mut counts = vec![0usize; weights.len()];
    for &x in &xs {
        let idx = l
            .slice
            .iter()
            .position(|(lo, hi)| x >= *lo && x < *hi)
            .expect("value outside every slice");
        counts[idx] += 1;
    }
    for (i, (&c, &p)) in counts.iter().zip(&l.bucket_p).enumerate() {
        if p == 0.0 {
            assert_eq!(c, 0, "zero-weight bucket {i} must stay empty");
            continue;
        }
        let exp = n as f64 * p;
        let sigma = (n as f64 * p * (1.0 - p)).sqrt();
        assert!(
            (c as f64 - exp).abs() <= 5.0 * sigma,
            "bucket {i}: {c} samples vs expected {exp} (5σ = {})",
            5.0 * sigma
        );
    }
}

#[test]
fn dense_uniform_matches_discrete_uniform_moments() {
    check(&[1.0; 8], 8, 42);
}

#[test]
fn sparse_uniform_matches_uniform_moments_over_2_32() {
    check(&[1.0, 1.0, 1.0, 1.0], 1 << 32, 43);
}

#[test]
fn skewed_weights_match_mixture_moments() {
    check(&[1.0, 2.0, 3.0, 4.0], 1_000_000, 44);
}

#[test]
fn bimodal_with_uneven_slices_matches_moments() {
    // 4098 / 4 is fractional — exercises the floor slice boundaries
    check(&[3.0, 1.0, 1.0, 3.0], 4098, 45);
}

#[test]
fn zero_weight_bucket_moments_and_support() {
    check(&[1.0, 0.0, 2.0], 300_000, 46);
}
