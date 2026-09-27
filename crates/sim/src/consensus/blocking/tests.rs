use super::*;
use rand::SeedableRng;

#[test]
fn schedule_matrix_equals_driver_draw_sequence() {
    let n = 12;
    let rounds = 15;
    let variants: Vec<BlockSchedule> = vec![
        BlockSchedule::FreshPerRound { fraction: 0.25 },
        BlockSchedule::Permanent { fraction: 0.5 },
        BlockSchedule::Windows(vec![crate::BlockWindow {
            start_round: 3,
            rounds: 4,
            target: BlockTarget::SampleFraction(0.25),
        }]),
        BlockSchedule::PerRoundFractions(vec![0.5, 0.0, 0.25]),
        BlockSchedule::PerRoundSticky {
            background: 0.1,
            targets: vec![0.25, 0.25, 0.5, 0.0],
        },
    ];
    for sched in variants {
        let got = mask_schedule(&sched, n, rounds, &mut ChaCha12Rng::seed_from_u64(42));
        let mut rng = ChaCha12Rng::seed_from_u64(42);
        let blocking = compile_blocking(&sched, n, &mut rng);
        let mut sticky = vec![false; n];
        let want: Vec<Vec<bool>> = (1..=rounds)
            .map(|r| round_mask(&blocking, r, &mut sticky, n, &mut rng))
            .collect();
        assert_eq!(got, want, "variant {sched:?}");
    }
}

#[test]
fn fraction_count_is_the_largest_integer_within_the_budget() {
    for (fraction, n, expected) in [
        (0.0, 598, 0),
        (0.1, 598, 59),
        (0.25, 6, 1),
        (0.25, 8, 2),
        (0.58, 100, 58),
        (0.8333333333333333, 6, 4),
        (1.0, 6, 6),
    ] {
        let count = fraction_count(fraction, n);
        assert_eq!(count, expected, "fraction={fraction}, n={n}");
        assert!(count as f64 / n as f64 <= fraction);
        assert!(count == n || (count + 1) as f64 / n as f64 > fraction);
    }
}

#[test]
fn every_fractional_schedule_component_uses_the_hard_budget() {
    let compile = |schedule| {
        let mut rng = ChaCha12Rng::seed_from_u64(42);
        compile_blocking(&schedule, 6, &mut rng)
    };

    assert!(matches!(
        compile(BlockSchedule::FreshPerRound { fraction: 0.25 }),
        Blocking::Fresh { count: 1 }
    ));
    assert!(matches!(
        compile(BlockSchedule::Adaptive1Late {
            fraction: 0.25,
            policy: crate::AdaptivePolicy::BlockHolders,
        }),
        Blocking::Adaptive { count: 1 }
    ));
    assert!(matches!(
        compile(BlockSchedule::AdaptiveAlphaLate {
            fraction: 0.25,
            alpha: 1,
            policy: crate::AdaptivePolicy::BlockHolders,
        }),
        Blocking::AdaptiveAlpha { count: 1, alpha: 1 }
    ));

    let Blocking::Fixed(permanent) = compile(BlockSchedule::Permanent { fraction: 0.25 }) else {
        panic!("permanent schedule must compile to a fixed mask")
    };
    assert_eq!(permanent.iter().filter(|&&blocked| blocked).count(), 1);

    let Blocking::Windows(windows) = compile(BlockSchedule::Windows(vec![crate::BlockWindow {
        start_round: 1,
        rounds: 1,
        target: BlockTarget::SampleFraction(0.25),
    }])) else {
        panic!("window schedule must compile to window masks")
    };
    assert_eq!(windows[0].2.iter().filter(|&&blocked| blocked).count(), 1);

    let Blocking::PerRound { counts } = compile(BlockSchedule::PerRoundFractions(vec![0.25]))
    else {
        panic!("per-round schedule must compile to counts")
    };
    assert_eq!(counts, vec![1]);

    let Blocking::Sticky {
        background_count,
        counts,
    } = compile(BlockSchedule::PerRoundSticky {
        background: 0.25,
        targets: vec![0.25],
    })
    else {
        panic!("sticky schedule must compile to component counts")
    };
    assert_eq!(background_count, 1);
    assert_eq!(counts, vec![1]);
}
