//! `mask_schedule` precomputes the full per-round blocked-mask matrix a
//! networked node derives locally from the shared blocking seed — every host
//! must get the identical matrix, per-variant semantics must match the
//! driver-loop behavior of `compile_blocking`/`round_mask`.

use rand::SeedableRng;
use rand_chacha::ChaCha12Rng;
use sim::{BlockSchedule, BlockTarget, BlockWindow, mask_schedule};

fn rng(seed: u64) -> ChaCha12Rng {
    ChaCha12Rng::seed_from_u64(seed)
}

fn popcount(mask: &[bool]) -> usize {
    mask.iter().filter(|b| **b).count()
}

#[test]
fn identical_across_hosts_and_calls() {
    let sched = BlockSchedule::FreshPerRound { fraction: 0.25 };
    let a = mask_schedule(&sched, 16, 40, &mut rng(9));
    let b = mask_schedule(&sched, 16, 40, &mut rng(9));
    assert_eq!(a, b, "same seed, same schedule matrix on every host");
    assert_eq!(a.len(), 40);
    assert!(a.iter().all(|m| m.len() == 16));
}

#[test]
fn permanent_draws_once_and_never_changes() {
    let sched = BlockSchedule::Permanent { fraction: 0.25 };
    let m = mask_schedule(&sched, 8, 20, &mut rng(1));
    assert_eq!(popcount(&m[0]), 2, "floor(0.25·8) = 2");
    assert!(m.iter().all(|row| *row == m[0]), "fixed set every round");
}

#[test]
fn fresh_per_round_resamples_each_round() {
    let sched = BlockSchedule::FreshPerRound { fraction: 0.25 };
    let m = mask_schedule(&sched, 16, 50, &mut rng(2));
    assert!(m.iter().all(|row| popcount(row) == 4));
    assert!(
        m.iter().any(|row| *row != m[0]),
        "50 fresh draws of 4-of-16 cannot all coincide"
    );
}

#[test]
fn windows_block_exactly_their_rounds() {
    let sched = BlockSchedule::Windows(vec![BlockWindow {
        start_round: 5,
        rounds: 5,
        target: BlockTarget::Nodes(vec![1, 3]),
    }]);
    let m = mask_schedule(&sched, 6, 12, &mut rng(3));
    for (i, row) in m.iter().enumerate() {
        let round = i + 1;
        if (5..=9).contains(&round) {
            assert_eq!(popcount(row), 2, "round {round}");
            assert!(row[1] && row[3]);
        } else {
            assert_eq!(popcount(row), 0, "round {round}");
        }
    }
}

#[test]
fn per_round_fractions_pad_tail_with_zero() {
    let sched = BlockSchedule::PerRoundFractions(vec![1.0, 0.5, 0.0]);
    let m = mask_schedule(&sched, 4, 6, &mut rng(4));
    let counts: Vec<usize> = m.iter().map(|r| popcount(r)).collect();
    assert_eq!(counts, vec![4, 2, 0, 0, 0, 0]);
}

#[test]
fn sticky_holds_membership_and_shrinks_within_set() {
    let sched = BlockSchedule::PerRoundSticky {
        background: 0.0,
        targets: vec![0.5, 0.5, 0.25],
    };
    let m = mask_schedule(&sched, 8, 3, &mut rng(5));
    assert_eq!(popcount(&m[0]), 4);
    assert_eq!(m[1], m[0], "steady target: no redraw, identical set");
    assert_eq!(popcount(&m[2]), 2);
    for (i, &released) in m[2].iter().enumerate() {
        assert!(
            !released || m[1][i],
            "release samples only among the blocked (node {i})"
        );
    }
}
