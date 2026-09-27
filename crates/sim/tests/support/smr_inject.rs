//! Explicit-target injections, mid-run `inject`, and the delivery-attempt
//! audit trail.

use sim::smr::{AttemptOutcome, ClientModel, Injection, Proto, SmrScenario, SmrState, run_smr};
use sim::{BlockSchedule, BlockTarget, BlockWindow, Config};

fn base(n: usize, seed: u64, injections: Vec<Injection>) -> SmrScenario {
    SmrScenario {
        n,
        seed,
        cfg: Config::default(),
        sigma: 2.0,
        proto: Proto::Extended,
        injections,
        max_rounds: 30,
        schedule: BlockSchedule::FreshPerRound { fraction: 0.0 },
        traffic: None,
        client_model: ClientModel::Unique,
        certs: false,
        manual_blocks: Vec::new(),
        partition: None,
        halt_on_violation: false,
        merge_policy: Default::default(),
        repeated_commit: Default::default(),
    }
}

fn inj(round: usize, client: u32, op: u64) -> Injection {
    Injection {
        round,
        client,
        op,
        target: None,
    }
}

fn inj_at(round: usize, client: u32, op: u64, target: u32) -> Injection {
    Injection {
        round,
        client,
        op,
        target: Some(target),
    }
}

#[test]
fn undelivered_explicit_injection_is_stream_invisible() {
    // Client 2 pins its target to node 0, which is blocked for the whole
    // run: the injection never draws and never delivers, so every metric
    // and the OTHER command's whole report are byte-identical to a run
    // without it.
    let block_forever = BlockSchedule::Windows(vec![BlockWindow {
        start_round: 1,
        rounds: 30,
        target: BlockTarget::Nodes(vec![0]),
    }]);
    let mut with = base(32, 9, vec![inj(2, 1, 7), inj_at(2, 2, 9, 0)]);
    with.schedule = block_forever.clone();
    let mut without = base(32, 9, vec![inj(2, 1, 7)]);
    without.schedule = block_forever;
    let (rw, rwo) = (run_smr(&with), run_smr(&without));
    assert_eq!(rw.metrics, rwo.metrics);
    assert_eq!(rw.commands[0], rwo.commands[0]);
    assert_eq!(rw.commands[1].delivered_round, None);
    assert!(
        rw.commands[1]
            .attempts
            .iter()
            .all(|a| a.target == 0 && a.outcome == AttemptOutcome::TargetBlocked)
    );
}

#[test]
fn explicit_target_delivers_when_released_from_blocking() {
    // Target node 5 is blocked rounds 2-3, which also makes it ⊥. Algorithm 3
    // admits the pinned command as soon as the target is unblocked in round 4.
    let mut s = base(32, 11, vec![inj_at(2, 1, 7, 5)]);
    s.schedule = BlockSchedule::Windows(vec![BlockWindow {
        start_round: 2,
        rounds: 2,
        target: BlockTarget::Nodes(vec![5]),
    }]);
    let r = run_smr(&s);
    let cmd = &r.commands[0];
    assert_eq!(cmd.delivered_round, Some(4));
    let shape: Vec<(usize, u32, AttemptOutcome)> = cmd
        .attempts
        .iter()
        .map(|a| (a.round, a.target, a.outcome))
        .collect();
    assert_eq!(
        shape,
        vec![
            (2, 5, AttemptOutcome::TargetBlocked),
            (3, 5, AttemptOutcome::TargetBlocked),
            (4, 5, AttemptOutcome::Delivered),
        ]
    );
}

#[test]
fn released_bottom_target_delivers_immediately() {
    // Node 0 is blocked only in round 1 and enters round 2 at ⊥. Algorithm 3
    // still admits the command because the target is now unblocked.
    let mut s = base(16, 3, vec![inj_at(2, 1, 7, 0)]);
    s.schedule = BlockSchedule::Windows(vec![BlockWindow {
        start_round: 1,
        rounds: 1,
        target: BlockTarget::Nodes(vec![0]),
    }]);
    let r = run_smr(&s);
    let first = &r.commands[0].attempts[0];
    assert_eq!((first.round, first.outcome), (2, AttemptOutcome::Delivered));
    assert_eq!(r.commands[0].delivered_round, Some(2));
}

#[test]
fn random_attempts_record_every_engagement() {
    let mut s = base(32, 17, vec![inj(2, 1, 7)]);
    s.schedule = BlockSchedule::FreshPerRound { fraction: 0.2 };
    let r = run_smr(&s);
    let cmd = &r.commands[0];
    let delivered = cmd.delivered_round.unwrap();
    assert_eq!(
        cmd.attempts.len(),
        delivered - 2 + 1,
        "one attempt per round from injection to delivery"
    );
    assert_eq!(
        cmd.attempts.last().unwrap().outcome,
        AttemptOutcome::Delivered
    );
}

#[test]
fn compact_attempt_trail_runs_amplified_ignored_acked() {
    let mut s = base(32, 5, vec![inj_at(2, 1, 7, 3)]);
    s.proto = Proto::Compact { t_commit_rounds: 6 };
    s.max_rounds = 40;
    let r = run_smr(&s);
    let outcomes: Vec<AttemptOutcome> = r.commands[0].attempts.iter().map(|a| a.outcome).collect();
    assert_eq!(*outcomes.first().unwrap(), AttemptOutcome::Amplified);
    assert_eq!(*outcomes.last().unwrap(), AttemptOutcome::AckCommitted);
    assert!(
        outcomes[1..outcomes.len() - 1]
            .iter()
            .all(|o| *o == AttemptOutcome::Ignored),
        "between accept and ack the pinned resends are ignored: {outcomes:?}"
    );
    assert_eq!(
        r.commands[0].committed_ack_round,
        r.commands[0].attempts.last().map(|a| a.round)
    );
}

#[test]
fn scenario_validation_rejects_out_of_range_target() {
    let s = base(16, 1, vec![inj_at(2, 1, 7, 16)]);
    assert!(s.validate().is_err());
}

#[test]
fn mid_run_inject_lands_next_round_and_leaves_history_untouched() {
    // Step 5 rounds, inject a LOW client id (0), keep stepping: rounds 1-5
    // metrics equal the no-inject baseline, the new command starts round 6.
    let run = |inject: bool| {
        let mut state = SmrState::new(
            32,
            Config::default(),
            Proto::Extended,
            2.0,
            21,
            &[inj(2, 1, 7)],
            &[],
            ClientModel::Unique,
            None,
        );
        for _ in 0..5 {
            state.step_fraction(0.0);
        }
        if inject {
            let round = state.inject(0, 99, None).unwrap();
            assert_eq!(round, 6, "injections always land next round");
        }
        for _ in 0..10 {
            state.step_fraction(0.0);
        }
        state.report()
    };
    let (with, without) = (run(true), run(false));
    assert_eq!(with.metrics[..5], without.metrics[..5]);
    let injected = with.commands.iter().find(|c| c.op == 99).unwrap();
    assert_eq!(injected.injection_round, 6);
    assert_eq!(injected.spread.points().first().map(|p| p.round), Some(6));
}

#[test]
fn inject_validation_mirrors_scenario_rules() {
    let mut state = SmrState::new(
        16,
        Config::default(),
        Proto::Extended,
        2.0,
        1,
        &[inj(1, 1, 7)],
        &[],
        ClientModel::Unique,
        None,
    );
    assert!(state.inject(2, 7, None).is_err(), "duplicate op");
    assert!(state.inject(2, 0, None).is_err(), "op 0 reserved");
    assert!(state.inject(2, 8, Some(16)).is_err(), "target out of range");
    assert!(state.inject(2, 8, Some(3)).is_ok());
    assert!(
        state.inject(1, 9, None).is_ok(),
        "a client's second command is sn 2, not an error"
    );
}
