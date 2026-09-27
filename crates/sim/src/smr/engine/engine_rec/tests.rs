use super::*;
use protocol::compact::{SharedState, Triage};
use std::sync::Arc;

fn client(client: u32, sn: u64) -> ClientCommand {
    ClientCommand { client, sn, op: sn }
}

fn recovery_node_with_committed_sn(committed: Option<(u32, u64)>) -> RecoveryNode {
    let mut node = RecoveryNode::new(Config::new(6, 3).expect("config"), 8, false);
    if let Some((client, sn)) = committed {
        node.share_checkpoint(&Arc::new(Checkpoint {
            s: SharedState::from_entries(Vec::new(), [(client, sn)].into()).into(),
            p: None,
            w: 0,
            certs: None,
        }));
    }
    node
}

#[test]
fn recovery_client_triage_requires_a_positive_next_sequence_number() {
    let node = recovery_node_with_committed_sn(None);

    assert_eq!(recovery_client_triage(&node, &client(1, 0)), Triage::Ignore);
    assert_eq!(
        recovery_client_triage(&node, &client(1, 1)),
        Triage::Amplify
    );
}

#[test]
fn recovery_client_triage_distinguishes_future_exact_and_stale_sequences() {
    let node = recovery_node_with_committed_sn(Some((1, 2)));

    assert_eq!(
        recovery_client_triage(&node, &client(1, 3)),
        Triage::Amplify
    );
    assert_eq!(recovery_client_triage(&node, &client(1, 4)), Triage::Ignore);
    assert_eq!(
        recovery_client_triage(&node, &client(1, 2)),
        Triage::AckCommitted
    );
    assert_eq!(recovery_client_triage(&node, &client(1, 1)), Triage::Ignore);
}

#[test]
fn recovery_client_triage_rejects_duplicate_log_entries_and_maximum_sequence_advancement() {
    let cc = client(1, 1);
    let mut duplicate = recovery_node_with_committed_sn(None);
    let reply = RecoveryReply {
        l_j: duplicate.log_perm_arc().cloned(),
        c_j: duplicate.checkpoint_shared(),
        r_j: RState::NoReset,
    };
    duplicate.step_chosen(
        &[reply.clone(), reply.clone(), reply],
        &[(cc, 1)],
        Some(&[0, 1, 2]),
    );
    let max = recovery_node_with_committed_sn(Some((2, u64::MAX)));

    assert_eq!(recovery_client_triage(&duplicate, &cc), Triage::Ignore);
    assert_eq!(
        recovery_client_triage(&max, &client(2, u64::MAX)),
        Triage::AckCommitted
    );
    assert_eq!(recovery_client_triage(&max, &client(2, 1)), Triage::Ignore);
}

#[test]
fn recovery_client_triage_does_not_ack_a_command_held_only_in_checkpoint_p() {
    let cc = client(7, 1);
    let mut node = recovery_node_with_committed_sn(None);
    let reply = RecoveryReply {
        l_j: node.log_perm_arc().cloned(),
        c_j: node.checkpoint_shared(),
        r_j: RState::NoReset,
    };
    node.step_chosen(
        &[reply.clone(), reply.clone(), reply],
        &[(cc, 1)],
        Some(&[0, 1, 2]),
    );
    node.end_window(1, 16);

    assert!(
        node.checkpoint()
            .p
            .as_ref()
            .expect("boundary carries command in P")
            .iter()
            .any(|timed| timed.entry == Entry::Cmd(cc))
    );
    assert_eq!(node.checkpoint().s.sn_get(cc.client), None);
    assert_eq!(recovery_client_triage(&node, &cc), Triage::Ignore);
}

#[test]
fn recovery_client_stage_waits_for_the_next_retry_after_a_real_boundary_commit() {
    let cc = client(7, 1);
    let t_window = 8;
    let mut state = SmrState::new(
        1,
        Config::new(6, 3).expect("config"),
        Proto::Recovery {
            t_window_rounds: t_window,
            resend_until_acked: true,
            prefix_mismatch: crate::smr::PrefixMismatch::Abort,
        },
        1.0,
        1,
        &[Injection {
            round: 1,
            client: cc.client,
            op: cc.op,
            target: Some(0),
        }],
        &[],
        ClientModel::Unique,
        None,
    );
    let no_block = [false];
    let mut p_only_round = None;
    let mut p_only_retries = 0;
    let mut committed_boundary = None;

    while state.round() < 64 && committed_boundary.is_none() {
        state.step_masked(&no_block);
        let round = state.round();
        let SmrNodes::Rec(nodes) = &state.nodes else {
            unreachable!();
        };
        let checkpoint = nodes[0].checkpoint();
        if checkpoint
            .p
            .as_ref()
            .is_some_and(|pre| pre.iter().any(|timed| timed.entry == Entry::Cmd(cc)))
            && checkpoint.s.sn_get(cc.client).is_none()
        {
            p_only_round = Some(round);
            p_only_retries += 1;
            assert_eq!(
                state.trackers[0]
                    .attempts
                    .last()
                    .map(|attempt| attempt.outcome),
                Some(AttemptOutcome::Ignored),
                "client retry must not treat P as committed S in round {round}"
            );
        }
        if checkpoint.s.sn_get(cc.client) == Some(cc.sn) {
            committed_boundary = Some(round);
            assert_eq!(
                state.trackers[0].committed_ack_round, None,
                "boundary mutation cannot ACK the client in the same step"
            );
        }
    }

    let p_only_round = p_only_round.expect("fixture reached a boundary with P but not S");
    assert!(p_only_retries > 0, "fixture made a P-only client retry");
    let committed_boundary = committed_boundary.expect("fixture committed at a later boundary");
    assert!(committed_boundary > p_only_round);
    let receivers_before_ack = state.trackers[0].amp_receivers.clone();
    state.step_masked(&no_block);
    let ack_round = state.round();

    assert!(!(ack_round as u64).is_multiple_of(t_window));
    assert_eq!(
        state.trackers[0]
            .attempts
            .last()
            .map(|attempt| attempt.outcome),
        Some(AttemptOutcome::AckCommitted)
    );
    assert_eq!(state.trackers[0].committed_ack_round, Some(ack_round));
    assert_eq!(state.trackers[0].amp_receivers, receivers_before_ack);

    let attempts_at_ack = state.trackers[0].attempts.len();
    state.step_masked(&no_block);
    assert_eq!(state.trackers[0].attempts.len(), attempts_at_ack);
}

#[test]
fn recovery_ack_evidence_is_local_until_checkpoint_adoption() {
    let cc = client(7, 1);
    let mut committed = recovery_node_with_committed_sn(None);
    let reply = RecoveryReply {
        l_j: committed.log_perm_arc().cloned(),
        c_j: committed.checkpoint_shared(),
        r_j: RState::NoReset,
    };
    committed.step_chosen(
        &[reply.clone(), reply.clone(), reply],
        &[(cc, 1)],
        Some(&[0, 1, 2]),
    );
    committed.end_window(1, 16);
    committed.end_window(2, 32);
    assert_eq!(
        recovery_client_triage(&committed, &cc),
        Triage::AckCommitted
    );

    let mut stale = recovery_node_with_committed_sn(None);
    assert_ne!(recovery_client_triage(&stale, &cc), Triage::AckCommitted);
    let adoption = RecoveryReply {
        l_j: committed.log_perm_arc().cloned(),
        c_j: committed.checkpoint_shared(),
        r_j: RState::NoReset,
    };
    stale.step_chosen(
        &[adoption.clone(), adoption.clone(), adoption],
        &[],
        Some(&[0, 1, 2]),
    );

    assert_eq!(stale.checkpoint().s.sn_get(cc.client), Some(cc.sn));
    assert_eq!(recovery_client_triage(&stale, &cc), Triage::AckCommitted);
}

#[test]
fn recovery_overlap_leaves_an_older_sequence_unacknowledged_after_successor_commit() {
    let node = recovery_node_with_committed_sn(Some((7, 2)));

    assert_eq!(recovery_client_triage(&node, &client(7, 1)), Triage::Ignore);
    assert_eq!(
        recovery_client_triage(&node, &client(7, 2)),
        Triage::AckCommitted
    );
    assert_eq!(
        recovery_client_triage(&node, &client(7, 3)),
        Triage::Amplify
    );
}

#[test]
fn recovery_client_stage_rejects_blocked_and_bottom_targets_before_triage() {
    let cc = client(1, 1);
    let mut state = SmrState::new(
        1,
        Config::new(6, 3).expect("config"),
        Proto::Recovery {
            t_window_rounds: 8,
            resend_until_acked: true,
            prefix_mismatch: crate::smr::PrefixMismatch::Abort,
        },
        1.0,
        1,
        &[Injection {
            round: 1,
            client: cc.client,
            op: cc.op,
            target: Some(0),
        }],
        &[],
        ClientModel::Unique,
        None,
    );
    let committed = Arc::new(Checkpoint {
        s: SharedState::from_entries(Vec::new(), [(cc.client, cc.sn)].into()).into(),
        p: None,
        w: 0,
        certs: None,
    });
    {
        let SmrNodes::Rec(nodes) = &mut state.nodes else {
            unreachable!();
        };
        nodes[0].share_checkpoint(&committed);
    }

    state.step_masked(&[true]);
    assert_eq!(
        state.trackers[0].attempts[0].outcome,
        AttemptOutcome::TargetBlocked
    );

    {
        let SmrNodes::Rec(nodes) = &mut state.nodes else {
            unreachable!();
        };
        nodes[0].step_chosen(&[], &[], None);
    }
    state.step_masked(&[false]);
    assert_eq!(
        state.trackers[0].attempts[1].outcome,
        AttemptOutcome::TargetBot
    );
}

fn entry(op: u64) -> Entry {
    Entry::Cmd(protocol::compact::ClientCommand {
        client: op as u32,
        sn: 1,
        op,
    })
}

// Empty suffices: these checkpoints forgot nothing, so the chain folds from the basis.
fn canon() -> CanonicalOrder {
    CanonicalOrder::new()
}

#[test]
fn healthy_boundary_does_not_retain_the_test_snapshot() {
    let mut state = SmrState::new_with_certs(
        16,
        Config::new(6, 3).expect("config"),
        Proto::Recovery {
            t_window_rounds: 10,
            resend_until_acked: false,
            prefix_mismatch: crate::smr::PrefixMismatch::Abort,
        },
        1.0,
        7,
        &[],
        &[],
        ClientModel::Unique,
        None,
        false,
        Default::default(),
        Default::default(),
    );
    for _ in 0..10 {
        state.step_fraction(0.0);
    }
    assert!(state.failure.is_none());
    assert!(
        state.rec_boundary_snapshot.is_none(),
        "healthy boundaries must release test-only Arc handles before mutation"
    );
}

#[test]
fn failed_boundary_preflight_mutates_no_node_at_the_boundary() {
    let n = 598;
    let mut state = SmrState::new_with_certs(
        n,
        Config::new(6, 3).expect("config"),
        Proto::Recovery {
            t_window_rounds: 20,
            resend_until_acked: true,
            prefix_mismatch: crate::smr::PrefixMismatch::Abort,
        },
        1.0,
        1,
        &[
            Injection {
                round: 2,
                client: 1,
                op: 1,
                target: None,
            },
            Injection {
                round: 2,
                client: 2,
                op: 2,
                target: None,
            },
        ],
        &[TrafficPhase {
            from_round: 1,
            arrivals_pmf: vec![0.0, 0.0, 1.0],
        }],
        ClientModel::Unique,
        None,
        true,
        Default::default(),
        Default::default(),
    );
    let mut failed_round = None;
    for round in 1..=200 {
        state.draw_arrivals();
        if state.step_fraction(0.1).failed() {
            failed_round = Some(round);
            break;
        }
    }
    let failed_round = failed_round.expect("fixture must reach a failed boundary");
    assert_eq!(failed_round % 20, 0, "failure must occur at a boundary");
    let before = state
        .rec_boundary_snapshot
        .as_ref()
        .expect("failed pre-boundary snapshot");
    assert!(before.iter().any(|node| node.r == RState::Bot));
    let SmrNodes::Rec(nodes) = &state.nodes else {
        unreachable!()
    };
    assert_eq!(before.len(), nodes.len());
    for (id, (before, after)) in before.iter().zip(nodes).enumerate() {
        assert!(
            Arc::ptr_eq(&before.checkpoint, &after.checkpoint_shared()),
            "node {id}: checkpoint changed after failed preflight"
        );
        assert!(
            Arc::ptr_eq(&before.state, after.state_arc()),
            "node {id}: shared state changed after failed preflight"
        );
        assert_eq!(
            before.r,
            after.reset_state(),
            "node {id}: R changed after failed preflight"
        );
        match (&before.log, after.log_perm_arc()) {
            (None, None) => {}
            (Some(before), Some(after)) => {
                assert!(
                    Arc::ptr_eq(before.entries(), after.entries()),
                    "node {id}: log changed after failed preflight"
                );
                assert!(
                    Arc::ptr_eq(before.perm(), after.perm()),
                    "node {id}: log index changed after failed preflight"
                );
            }
            _ => panic!("node {id}: log bottomness changed after failed preflight"),
        }
    }
}

fn cp(window: u64, ops: &[u64], sn: &[(u32, u64)], pre: Option<Vec<Timed>>) -> Checkpoint {
    Checkpoint {
        s: SharedState::from_entries(
            ops.iter().copied().map(entry).collect(),
            sn.iter().copied().collect(),
        )
        .into(),
        p: pre,
        w: window,
        certs: None,
    }
}

#[test]
fn a_fork_in_any_component_at_one_window_is_caught() {
    let clean = cp(1, &[1, 2], &[(1, 1)], None);
    for forked in [
        cp(1, &[1, 3], &[(1, 1)], None),
        cp(1, &[1, 2], &[(1, 2)], None),
        cp(1, &[1, 2], &[(1, 1)], Some(Vec::new())),
    ] {
        let mut rec = RecRunState::new(10, 2, false, crate::smr::PrefixMismatch::Abort);
        rec.check_lineage(0, &clean, &canon());
        assert!(
            rec.fork_ok,
            "the first checkpoint at a window is the record"
        );
        rec.check_lineage(1, &forked, &canon());
        assert!(!rec.fork_ok, "{forked:?} agreed with {clean:?}");
    }
}

#[test]
fn a_new_window_that_does_not_extend_the_last_one_is_caught() {
    for broken in [
        cp(2, &[1, 9, 3], &[], None),
        cp(2, &[1], &[], None),
        cp(2, &[], &[], None),
    ] {
        let mut rec = RecRunState::new(10, 2, false, crate::smr::PrefixMismatch::Abort);
        rec.check_lineage(0, &cp(1, &[1, 2], &[], None), &canon());
        rec.check_lineage(1, &broken, &canon());
        assert!(!rec.fork_ok, "{broken:?} passed as an extension");
    }
}

#[test]
fn a_clean_lineage_survives_re_presentation_and_extension() {
    let mut rec = RecRunState::new(10, 2, false, crate::smr::PrefixMismatch::Abort);
    let w1 = cp(1, &[1, 2], &[(1, 1)], None);
    for _ in 0..5 {
        rec.check_lineage(0, &w1, &canon());
        rec.check_lineage(1, &w1, &canon());
    }
    rec.check_lineage(0, &cp(2, &[1, 2, 3], &[(1, 1), (3, 1)], None), &canon());
    rec.check_lineage(1, &cp(2, &[1, 2, 3], &[(1, 1), (3, 1)], None), &canon());
    assert!(rec.fork_ok);
}

#[test]
fn equal_states_with_unequal_forget_lines_are_one_checkpoint() {
    let ops = [1u64, 2, 3, 4];
    let entries: Vec<Entry> = ops.iter().copied().map(entry).collect();
    let canon = CanonicalOrder::from_entries(&entries);
    let signature = |forget: u64| {
        let mut c = cp(3, &ops, &[(1, 1)], Some(Vec::new()));
        Arc::make_mut(&mut c.s).forget_committed_prefix(forget);
        ForkSignature::of(&canon, &c)
    };
    for forget in 1..=ops.len() as u64 {
        assert_eq!(
            signature(0),
            signature(forget),
            "forgetting {forget} of one committed prefix made a second checkpoint"
        );
    }
}

#[test]
fn a_divergence_above_the_forget_lines_is_still_two_checkpoints() {
    let entries: Vec<Entry> = [1u64, 2, 3, 4].iter().copied().map(entry).collect();
    let canon = CanonicalOrder::from_entries(&entries);
    let signature = |ops: &[u64], forget: u64| {
        let mut c = cp(3, ops, &[(1, 1)], None);
        Arc::make_mut(&mut c.s).forget_committed_prefix(forget);
        ForkSignature::of(&canon, &c)
    };
    for forget in 0..=2 {
        assert_ne!(
            signature(&[1, 2, 3, 4], forget),
            signature(&[1, 2, 9, 4], forget),
            "forget {forget}: a fork above the line hashed the same"
        );
    }
}

#[test]
#[should_panic(expected = "disagree on what was forgotten")]
fn comparing_the_states_themselves_would_assert_on_that_pair() {
    let ops = [1u64, 2, 3, 4];
    let a = cp(3, &ops, &[(1, 1)], None);
    let mut b = cp(3, &ops, &[(1, 1)], None);
    Arc::make_mut(&mut b.s).forget_committed_prefix(2);
    let _ = a == b;
}

fn reply(window: u64) -> RecoveryReply {
    RecoveryReply {
        l_j: None,
        c_j: Arc::new(Checkpoint {
            s: SharedState::default().into(),
            p: None,
            w: window,
            certs: None,
        }),
        r_j: RState::Reset,
    }
}

#[test]
#[should_panic(expected = "executed sequence regressed")]
fn a_shrinking_executed_length_panics() {
    let mut rec = RecRunState::new(10, 2, false, crate::smr::PrefixMismatch::Abort);
    rec.check_growth(1, 5, 3);
    rec.check_growth(1, 4, 4);
}

#[test]
fn legal_growth_and_a_standing_length_pass() {
    let mut rec = RecRunState::new(10, 2, false, crate::smr::PrefixMismatch::Abort);
    for (round, len) in [(1, 0), (2, 3), (3, 3), (4, 9)] {
        rec.check_growth(0, len, round);
    }
    assert_eq!(rec.prev_len, vec![9, 0]);
}

#[test]
fn argmax_picks_the_very_reply_the_node_adopts() {
    for windows in [
        vec![0],
        vec![0, 1, 2],
        vec![2, 1, 0],
        vec![1, 3, 3, 2],
        vec![3, 3, 3],
        vec![0, 5, 5, 5, 1],
    ] {
        let replies: Vec<RecoveryReply> = windows.iter().map(|&w| reply(w)).collect();
        let adopted = replies
            .iter()
            .map(|rep| &rep.c_j)
            .max_by_key(|c| c.w)
            .expect("non-empty");
        assert!(
            std::ptr::eq(&replies[last_max_window(&replies)].c_j, adopted),
            "windows {windows:?}: tie must resolve to the same element"
        );
    }
}
