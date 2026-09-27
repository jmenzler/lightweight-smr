//! Boundary interning: nodes whose checkpoints the fork oracle has proven
//! content-equal hold ONE allocation instead of n. The oracle's classes and
//! the allocation classes are the same partition by construction, so what has
//! to be pinned at engine level is that the collapse really happens and that
//! it does not blind the oracle it rides on. That the classes themselves stay
//! apart under real divergence is the oracle's own property, pinned by
//! `engine_rec`'s fork-signature tests; that a wrong pairing is refused rather
//! than silently merged is pinned protocol-side on `share_checkpoint`.

use sim::smr::{
    ClientModel, Proto, SmrScenario, SmrState, TrafficPhase, point_mass_pmf, run_smr_lean_observed,
};
use sim::{BlockSchedule, Config};

const N: usize = 32;
const T: usize = 20;

fn cell() -> SmrScenario {
    SmrScenario {
        n: N,
        seed: 615_200,
        cfg: Config::default(),
        sigma: 1.0,
        proto: Proto::Recovery {
            t_window_rounds: T as u64,
            resend_until_acked: false,
            prefix_mismatch: sim::smr::PrefixMismatch::Abort,
        },
        injections: vec![],
        max_rounds: 100,
        schedule: BlockSchedule::FreshPerRound { fraction: 0.1 },
        traffic: Some(vec![TrafficPhase {
            from_round: 1,
            arrivals_pmf: point_mass_pmf(1),
        }]),
        client_model: ClientModel::Unique,
        certs: true,
        manual_blocks: Vec::new(),
        partition: None,
        halt_on_violation: false,
        merge_policy: Default::default(),
        repeated_commit: Default::default(),
    }
}

/// The payoff: on a healthy run every node's checkpoint is content-equal, so
/// one allocation carries the population's cloned state and its whole §5
/// window map instead of n copies of them. Reported per boundary against the
/// fork count the same signatures produce, because a collapse that also
/// collapsed the oracle would be a regression, not a win.
#[test]
fn a_healthy_boundary_collapses_the_population_onto_few_checkpoints() {
    let s = cell();
    let mut distinct_at = vec![0usize; s.max_rounds + 1];
    let report = run_smr_lean_observed(&s, &mut |state: &SmrState| {
        let acc = state
            .memory_accounting()
            .expect("a recovery run reports an accounting");
        distinct_at[acc.round] = acc.distinct_checkpoints;
    });
    let rows = report.recovery.expect("recovery block").rounds;

    // Anti-vacuity: before anything has been minted, every node holds its own
    // genesis allocation, so the walk really is counting allocations.
    assert_eq!(
        distinct_at[1], N,
        "the pre-boundary population must hold one checkpoint each"
    );

    // The boundary latches its fork count into the NEXT round's row, so the
    // last boundary has no row to land on.
    let boundaries: Vec<usize> = (T..s.max_rounds).step_by(T).collect();
    assert!(!boundaries.is_empty(), "the run must cross a boundary");
    for r in boundaries {
        let distinct = distinct_at[r];
        assert!(
            rows[r].cp_fork_k >= 1,
            "round {r}: interning left the fork oracle reporting nothing"
        );
        // A boundary can leave more than one: the freshly minted class plus
        // whatever a ⊥ node still carries from an earlier window. What it may
        // not leave is a copy per node.
        assert!(
            distinct * 4 <= N,
            "round {r}: {distinct} checkpoint allocations over {N} nodes is no collapse"
        );
    }
    assert!(
        distinct_at[T + 1..].contains(&1),
        "the population never reached a single shared checkpoint: {distinct_at:?}"
    );
}
