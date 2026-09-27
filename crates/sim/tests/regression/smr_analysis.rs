use sim::analysis::{geometric_mean_growth, growth_window, ols_fit};
use sim::smr::SpreadPoint;

fn pt(round: usize, holders: u32, total: u32) -> SpreadPoint {
    SpreadPoint {
        round,
        useful_holders: holders,
        useful_total: total,
        pos_min: None,
        pos_med: None,
        pos_max: None,
    }
}

#[test]
fn growth_window_starts_at_threshold_and_excludes_saturated_bases() {
    // n = 1024, c_win = 1 → threshold 10 holders; γ ≤ 1/3 of 1000 useful.
    let spread = vec![
        pt(1, 4, 1000),   // below threshold
        pt(2, 12, 1000),  // in window
        pt(3, 30, 1000),  // in window
        pt(4, 90, 1000),  // in window
        pt(5, 300, 1000), // in window (γ = 0.3 ≤ 1/3)
        pt(6, 700, 1000), // γ > 1/3 — saturated base, excluded
        pt(7, 1000, 1000),
    ];
    assert_eq!(growth_window(&spread, 1.0, 1024), Some((1, 4)));
    assert_eq!(
        growth_window(&spread, 100.0, 1024),
        None,
        "never reaches c_win·log2 n"
    );
}

#[test]
fn geometric_mean_uses_in_window_bases_and_needs_three_ratios() {
    let spread = vec![
        pt(1, 10, 1000),
        pt(2, 20, 1000),
        pt(3, 40, 1000),
        pt(4, 80, 1000),
        pt(5, 160, 1000),
    ];
    // Bases 0..=3 (γ ≤ 1/3 fails only at none), ratios all exactly 2.
    let g = geometric_mean_growth(&spread, (0, 3)).unwrap();
    assert!((g - 2.0).abs() < 1e-12);
    assert_eq!(
        geometric_mean_growth(&spread[..3], (0, 1)),
        None,
        "fewer than 3 ratios is inconclusive"
    );
}

#[test]
fn growth_window_none_below_threshold() {
    // n = 1024, c_win = 1 → threshold 10 holders; nothing ever reaches it.
    let spread = vec![pt(1, 2, 1000), pt(2, 5, 1000), pt(3, 9, 1000)];
    assert_eq!(growth_window(&spread, 1.0, 1024), None);
}

#[test]
fn geometric_mean_skips_zero_gamma_bases() {
    // Bases 1 and 3 have zero holders → only 1 valid ratio of 4 → None.
    let spread = vec![
        pt(1, 10, 1000),
        pt(2, 0, 1000),
        pt(3, 40, 1000),
        pt(4, 0, 1000),
        pt(5, 160, 1000),
    ];
    assert_eq!(geometric_mean_growth(&spread, (0, 4)), None);
}

#[test]
fn ols_rejects_degenerate_inputs() {
    assert_eq!(ols_fit(&[], &[]), None, "empty sample");
    assert_eq!(ols_fit(&[1.0], &[2.0]), None, "single point");
    assert_eq!(
        ols_fit(&[3.0, 3.0, 3.0], &[1.0, 2.0, 3.0]),
        None,
        "zero x-variance has no defined slope"
    );
}

#[test]
fn ols_r2_unity_only_on_constant_series() {
    let xs = [1.0, 2.0, 3.0, 4.0];
    let (slope, intercept, r2) = ols_fit(&xs, &[5.0, 5.0, 5.0, 5.0]).unwrap();
    assert_eq!((slope, intercept, r2), (0.0, 5.0, 1.0), "ss_tot = 0 branch");

    let (_, _, r2_noisy) = ols_fit(&xs, &[5.0, 7.1, 8.9, 11.0]).unwrap();
    assert!(r2_noisy < 1.0, "non-constant residuals must cost r²");
}

#[test]
fn ols_recovers_exact_line() {
    let xs = [1.0, 2.0, 3.0, 4.0];
    let ys = [5.0, 7.0, 9.0, 11.0]; // y = 2x + 3
    let (slope, intercept, r2) = ols_fit(&xs, &ys).unwrap();
    assert!((slope - 2.0).abs() < 1e-12);
    assert!((intercept - 3.0).abs() < 1e-12);
    assert!((r2 - 1.0).abs() < 1e-12);
    assert_eq!(
        ols_fit(&xs[..1], &ys[..1]),
        None,
        "need at least two points"
    );
}
