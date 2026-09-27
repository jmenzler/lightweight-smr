//! Certificates (§5) need clients with sn sequences: a client's b-th command
//! carries sn = b and can only commit after its predecessor (Alg 5 triage).
//! The engine derives sn from per-client injection order; the resend loop
//! makes later commands wait naturally (Ignore until sn(c) catches up).

use sim::smr::{ClientModel, CommandStatus, Injection, Proto, SmrScenario, SmrState, run_smr};
use sim::{BlockSchedule, Config};

const PROTO: Proto = Proto::Compact {
    t_commit_rounds: 25,
};

#[test]
fn a_client_commits_a_second_command_after_its_first() {
    let scenario = SmrScenario {
        n: 32,
        seed: 42,
        cfg: Config::default(),
        sigma: 1.0,
        proto: PROTO,
        injections: vec![
            Injection {
                round: 3,
                client: 1,
                op: 100,
                target: None,
            },
            Injection {
                round: 3,
                client: 1,
                op: 101,
                target: None,
            },
            Injection {
                round: 5,
                client: 2,
                op: 200,
                target: None,
            },
        ],
        max_rounds: 300,
        schedule: BlockSchedule::FreshPerRound { fraction: 0.0 },
        traffic: None,
        client_model: ClientModel::Unique,
        certs: false,
        manual_blocks: Vec::new(),
        partition: None,
        halt_on_violation: false,
        merge_policy: Default::default(),
        repeated_commit: Default::default(),
    };
    let report = run_smr(&scenario);
    assert!(report.safety_ok);
    let by_op = |op: u64| {
        report
            .commands
            .iter()
            .find(|c| c.op == op)
            .unwrap_or_else(|| panic!("op {op} missing"))
    };
    for op in [100, 101, 200] {
        assert_eq!(by_op(op).status, CommandStatus::Complete, "op {op}");
    }
    // sn order: the second command (op 101) can only be acked after the first.
    assert!(
        by_op(101).committed_ack_round.unwrap() > by_op(100).committed_ack_round.unwrap(),
        "sn 2 acked at {:?}, sn 1 at {:?}",
        by_op(101).committed_ack_round,
        by_op(100).committed_ack_round
    );
}

#[test]
fn live_injection_accepts_a_clients_second_command() {
    let mut state = SmrState::new(
        32,
        Config::default(),
        PROTO,
        1.0,
        43,
        &[],
        &[],
        ClientModel::Unique,
        None,
    );
    let no_block = vec![false; 32];
    state.inject(1, 100, None).expect("first command");
    for _ in 0..60 {
        state.step_masked(&no_block);
    }
    state
        .inject(1, 101, None)
        .expect("second command of the same client");
    for _ in 0..120 {
        state.step_masked(&no_block);
    }
    let report = state.report();
    assert!(report.safety_ok);
    for op in [100, 101] {
        let c = report.commands.iter().find(|c| c.op == op).unwrap();
        assert_eq!(c.status, CommandStatus::Complete, "op {op}");
    }
}
