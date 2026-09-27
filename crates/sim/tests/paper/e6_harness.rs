//! The E6 certificate harness over a live stepped run: forests fed from each
//! server's executed prefix, notifications on the resubmission-ack channel,
//! Thm 6 verification at useful servers, and the §5 p. 29 staleness physics.

use protocol::certificates::Certificate;
use sim::Config;
use sim::certs::CertHarness;
use sim::smr::{ClientModel, Proto, SmrState};

const N: usize = 32;

/// Wait-policy client actor: submits its next command only after the
/// previous one was acked (§4 base behavior, p. 25).
struct Actor {
    client: u32,
    next_op: u64,
    issued: u64,
}

impl Actor {
    fn pump(&mut self, state: &mut SmrState, harness: &mut CertHarness) {
        if harness.acked_sn(self.client) == self.issued {
            let op = self.next_op;
            self.next_op += 1;
            state.inject(self.client, op, None).expect("inject");
            self.issued += 1;
            harness.record_issue(self.client, self.issued, op);
        }
    }
}

fn compact_state(seed: u64, op: u64) -> SmrState {
    let mut state = SmrState::new(
        N,
        Config::default(),
        Proto::Compact {
            t_commit_rounds: 25,
        },
        1.0,
        seed,
        &[],
        &[],
        ClientModel::Unique,
        None,
    );
    state.inject(1, op, None).expect("inject");
    state
}

/// Servers with executed entries in `state` (executed length ≥ 1).
fn executed_count(state: &SmrState) -> usize {
    state
        .executed_seqs()
        .iter()
        .filter(|s| !s.is_empty())
        .count()
}

#[test]
fn roots_consistent_flags_divergence_and_rebuild_recovers() {
    let no_block = vec![false; N];
    // Run A with server 0 held blocked: the rest executes op 100, server 0
    // stays empty — the harness then holds A-forests on exactly those servers.
    let mut hold_zero = vec![false; N];
    hold_zero[0] = true;
    let mut a = compact_state(1, 100);
    while executed_count(&a) < N - 1 {
        assert!(a.round() < 400, "never saw partial execution coverage");
        a.step_masked(&hold_zero);
    }
    assert!(a.executed_seqs()[0].is_empty(), "blocked server executed");
    let mut harness = CertHarness::new(N);
    harness.observe_round(&a);
    assert!(harness.roots_consistent(), "single-run feed is consistent");

    // Run B commits a different op; observing it fills the lagging servers
    // with B-forests, so equal-length forests now disagree on roots.
    let mut b = compact_state(2, 200);
    while executed_count(&b) < N {
        assert!(b.round() < 400, "run B never fully executed");
        b.step_masked(&no_block);
    }
    harness.observe_round(&b);
    assert!(
        !harness.roots_consistent(),
        "oracle must flag divergent forests"
    );

    // A fresh state's shorter (empty) executed sequences hit the
    // adoption-shrink rebuild: every forest is replayed from scratch.
    let fresh = compact_state(3, 300);
    harness.observe_round(&fresh);
    assert!(
        harness.roots_consistent(),
        "rebuild-by-replay restores consistency"
    );
    assert!(
        harness
            .server_states()
            .iter()
            .all(|s| s.forest().roots().is_empty()),
        "rebuild starts from empty forests"
    );
}

#[test]
fn certificates_verify_across_a_live_run_and_staleness_behaves() {
    let mut state = SmrState::new(
        N,
        Config::default(),
        Proto::Compact {
            t_commit_rounds: 25,
        },
        1.0,
        4242,
        &[],
        &[],
        ClientModel::Unique,
        None,
    );
    let mut harness = CertHarness::new(N);
    let mut actors = vec![
        Actor {
            client: 10,
            next_op: 100,
            issued: 0,
        },
        Actor {
            client: 11,
            next_op: 200,
            issued: 0,
        },
    ];
    for a in &actors {
        harness.register_client(a.client);
    }

    let no_block = vec![false; N];
    let mut stale: Option<Certificate> = None;
    let mut stale_at_sn = 0;
    for _ in 0..400 {
        for a in &mut actors {
            a.pump(&mut state, &mut harness);
        }
        state.step_masked(&no_block);
        harness.observe_round(&state);
        // Every round: servers agreeing on m agree on roots.
        assert!(harness.roots_consistent(), "root divergence");

        // Capture a bare-newest certificate the moment client 10 has one.
        if stale.is_none() && harness.acked_sn(10) >= 1 {
            stale_at_sn = harness.acked_sn(10);
            stale = harness.client(10).build_certificate(stale_at_sn);
        }
    }

    // Both clients made progress under sustained load.
    assert!(
        harness.acked_sn(10) >= stale_at_sn + 2,
        "need ≥2 later commits"
    );
    assert!(harness.acked_sn(11) >= 3);

    // Thm 6: every committed command verifies at every covering useful server.
    for a in &actors {
        for sn in 1..=harness.acked_sn(a.client) {
            let cert = harness
                .client(a.client)
                .build_certificate(sn)
                .unwrap_or_else(|| panic!("client {} sn {sn}", a.client));
            let tally = harness.verify_everywhere(a.client, &cert, &no_block);
            assert!(
                tally.covered > 0,
                "client {} sn {sn} covered nowhere",
                a.client
            );
            assert_eq!(
                tally.accepted, tally.covered,
                "client {} sn {sn}: {}/{} accepted",
                a.client, tally.accepted, tally.covered
            );
        }
    }

    // §5 p. 29: the captured bare-newest certificate went stale — ≥2 further
    // commits of client 10 evicted it from every last-two window.
    let stale = stale.expect("captured a newest certificate");
    assert!(matches!(stale, Certificate::Newest { .. }));
    let tally = harness.verify_everywhere(10, &stale, &no_block);
    assert!(tally.covered > 0);
    assert_eq!(
        tally.accepted, 0,
        "stale newest cert must be rejected everywhere"
    );
}
