//! The reply memo hands a ⊥ requester the target's OWN state handle rather
//! than a fresh deep copy: after the adoption round, the adopter and its donor
//! hold one allocation. Pointer-observable and byte-invisible — the battery
//! pins the bytes, this pins the sharing.
//!
//! Exactly one node is ever blocked, for one round, so under a memo that
//! still deep-copies every node pointer stays pairwise distinct — the memo's
//! private allocation is held by no other node. A duplicate pointer can only
//! come from the memo handing out a peer's own handle.

use sim::smr::{ClientModel, ManualBlock, Proto, SmrScenario, run_smr_observed};
use sim::{BlockSchedule, Config};

#[test]
fn an_adopter_lands_on_a_peers_own_allocation() {
    let n = 16;
    let scenario = SmrScenario {
        n,
        seed: 11,
        cfg: Config::default(),
        sigma: 1.0,
        proto: Proto::Compact {
            t_commit_rounds: 1_000,
        },
        injections: vec![],
        max_rounds: 12,
        schedule: BlockSchedule::FreshPerRound { fraction: 0.0 },
        traffic: None,
        client_model: ClientModel::Unique,
        certs: false,
        halt_on_violation: false,
        merge_policy: Default::default(),
        repeated_commit: Default::default(),
        manual_blocks: vec![ManualBlock {
            node: 3,
            from_round: 5,
            to_round: Some(6),
        }],
        partition: None,
    };
    let mut saw_shared = false;
    run_smr_observed(&scenario, &mut |st| {
        let mut ptrs: Vec<usize> = (0..n)
            .filter_map(|i| st.node_state(i))
            .map(|s| std::ptr::from_ref(s) as usize)
            .collect();
        ptrs.sort_unstable();
        saw_shared |= ptrs.windows(2).any(|w| w[0] == w[1]);
    });
    assert!(
        saw_shared,
        "the adoption round never left the adopter on a peer's own allocation"
    );
}
