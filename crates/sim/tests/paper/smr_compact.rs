use protocol::compact::{ClientCommand, Entry};
use sim::smr::{
    ClientModel, CommandStatus, Injection, Proto, SmrScenario, prefixes_consistent, run_smr,
};
use sim::{BlockSchedule, Config};

const T: u64 = 6;

fn scenario(n: usize, seed: u64, injections: Vec<Injection>) -> SmrScenario {
    SmrScenario {
        n,
        seed,
        cfg: Config::default(),
        sigma: 5.0,
        proto: Proto::Compact { t_commit_rounds: T },
        injections,
        max_rounds: 40,
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

#[test]
fn compact_quiescent_run_ages_seed_and_stays_alive() {
    let report = run_smr(&scenario(32, 7, vec![]));
    assert_eq!(report.metrics.len(), 40);
    for m in &report.metrics {
        assert_eq!(m.nonbot_logs, 32, "no log may fail at beta = 0");
        assert_eq!(m.max_log_len, 1, "seed/dummy no-ops only");
        assert_eq!(m.max_executed_len, 0, "no-ops never enter executed");
    }
    assert!(report.safety_ok);
}

#[test]
fn compact_lifecycle_commits_everywhere() {
    let report = run_smr(&scenario(32, 9, vec![inj(2, 1, 7)]));
    let cmd = &report.commands[0];
    assert_eq!(
        cmd.delivered_round,
        Some(2),
        "beta = 0: accepted on first draw"
    );
    let ack = cmd.committed_ack_round.expect("client learns commitment");
    assert!(
        ack >= 2 + T as usize,
        "ack only after the command aged T rounds (ack {ack})"
    );
    assert_eq!(cmd.status, CommandStatus::Complete);
    let last = report.metrics.last().unwrap();
    assert_eq!(
        (last.min_executed_len, last.max_executed_len),
        (1, 1),
        "every server executed exactly the injected command"
    );
    assert!(report.safety_ok);
}

#[test]
fn client_resends_until_committed_ack() {
    // The delivery ack (Amplify accept) and the commitment ack are different
    // rounds: the client keeps resending between them and stops after
    // AckCommitted.
    let report = run_smr(&scenario(32, 11, vec![inj(2, 1, 7)]));
    let cmd = &report.commands[0];
    let delivered = cmd.delivered_round.unwrap();
    let ack = cmd.committed_ack_round.unwrap();
    assert!(ack > delivered);
}

#[test]
fn bot_requester_recovers_shared_state_from_snapshot_peers() {
    // Node 0 is blocked while the others commit the command; when released it
    // is ⊥ with an empty state and must catch up via a peer's attached
    // snapshot state (b_i = 1 reply path).
    let mut s = scenario(32, 13, vec![inj(2, 1, 7)]);
    s.schedule = BlockSchedule::Windows(vec![sim::BlockWindow {
        start_round: 2,
        rounds: T as usize + 4,
        target: sim::BlockTarget::Nodes(vec![0]),
    }]);
    let report = run_smr(&s);
    let last = report.metrics.last().unwrap();
    assert_eq!(
        (last.min_executed_len, last.max_executed_len),
        (1, 1),
        "the recovered node's state caught up"
    );
    assert!(report.safety_ok);
}

#[test]
fn executed_prefix_checker_flags_divergence() {
    let a = Entry::Cmd(ClientCommand {
        client: 1,
        sn: 1,
        op: 7,
    });
    let b = Entry::Cmd(ClientCommand {
        client: 2,
        sn: 1,
        op: 8,
    });
    let consistent: Vec<Vec<Entry>> = vec![vec![a.clone()], vec![a.clone(), b.clone()], vec![]];
    assert!(prefixes_consistent(&consistent));
    let diverged: Vec<Vec<Entry>> = vec![vec![a.clone()], vec![b.clone(), a.clone()]];
    assert!(!prefixes_consistent(&diverged));
    let same_len_diverged: Vec<Vec<Entry>> = vec![vec![a.clone()], vec![b.clone()]];
    assert!(!prefixes_consistent(&same_len_diverged));
}

/// FNV-1a over the run's observable trajectory. Hand-rolled rather than
/// `DefaultHasher`, whose output the standard library does not promise to
/// keep stable across releases — a golden constant has to outlive toolchains.
fn digest(vals: impl IntoIterator<Item = u64>) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for v in vals {
        for b in v.to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h
}

#[test]
fn golden_compact_landmarks_are_stable() {
    // These constants pin the RNG stream and merge order; same-seed replay
    // tests move with a shared stream shift and cannot detect it.
    let mut s = scenario(
        64,
        424_242,
        vec![inj(2, 1, 7), inj(9, 2, 11), inj(17, 3, 13)],
    );
    s.max_rounds = 60;
    s.schedule = BlockSchedule::FreshPerRound { fraction: 0.12 };
    let report = run_smr(&s);
    let m = &report.metrics;
    assert_eq!(m.len(), 60);
    assert!(
        m.iter().all(|round| round.blocked == 7),
        "floor(0.12*64) must block exactly 7 servers per round"
    );

    // Commitment timing: the first round each command's client is told it
    // committed, plus the two broadcast landmarks around it.
    let marks = |i: usize| {
        let c = &report.commands[i];
        (
            c.delivered_round,
            c.all_logs_round,
            c.prefix_fixed_round,
            c.committed_ack_round,
        )
    };
    // `prefix_fixed_round` is None throughout by construction: compact strips
    // the committed prefix out of the log, so the extended rule's
    // shared-prefix landmark never fires here. `committed_ack_round` is this
    // rule's commitment moment.
    assert_eq!(marks(0), (Some(3), Some(6), None, Some(10)));
    assert_eq!(marks(1), (Some(9), Some(11), None, Some(16)));
    assert_eq!(marks(2), (Some(17), Some(19), None, Some(24)));

    // Log length at fixed rounds — the merge's own output size, which any
    // change to the containment test or the union walk would move.
    let shape = |r: usize| {
        let x = m[r - 1];
        (x.nonbot_logs, x.distinct_logs, x.max_log_len)
    };
    assert_eq!(shape(10), (55, 5, 2));
    assert_eq!(shape(30), (53, 1, 1));

    // The broadcast curve of the first command: holders round by round, the
    // most stream-sensitive observable the report carries.
    let spread: Vec<u32> = report.commands[0]
        .spread
        .points()
        .iter()
        .take(8)
        .map(|p| p.useful_holders)
        .collect();
    assert_eq!(spread, vec![0, 0, 19, 33, 48, 49, 49, 49]);

    // Digest of the committed trajectory: every round's executed-length pair
    // and log shape folded into one constant.
    let d = digest(m.iter().flat_map(|x| {
        [
            x.min_executed_len as u64,
            x.max_executed_len as u64,
            x.max_log_len as u64,
            x.nonbot_logs as u64,
        ]
    }));
    assert_eq!(d, 0x08b4_a58f_5e58_0688);

    let last = m.last().unwrap();
    assert_eq!((last.min_executed_len, last.max_executed_len), (3, 3));
    assert!(report.safety_ok);
}

#[test]
fn compact_safety_holds_under_blocking() {
    let mut s = scenario(64, 17, vec![inj(2, 1, 7), inj(5, 2, 9), inj(9, 3, 11)]);
    s.max_rounds = 100;
    s.schedule = BlockSchedule::FreshPerRound { fraction: 0.1 };
    let report = run_smr(&s);
    assert!(report.safety_ok);
    for cmd in &report.commands {
        assert_eq!(cmd.status, CommandStatus::Complete, "op {}", cmd.op);
    }
}
