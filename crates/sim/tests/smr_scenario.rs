use sim::smr::{
    ClientModel, Injection, PrefixMismatch, Proto, RepeatedCommit, SmrScenario, amp_count,
};
use sim::{BlockSchedule, Config};

fn scenario(injections: Vec<Injection>) -> SmrScenario {
    SmrScenario {
        n: 16,
        seed: 7,
        cfg: Config::default(),
        sigma: 5.0,
        proto: Proto::Extended,
        injections,
        max_rounds: 50,
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
fn amp_count_is_ceil_sigma_log2_n() {
    assert_eq!(amp_count(5.0, 1000), 50); // 5 · 9.9658… → 50
    assert_eq!(amp_count(1.0, 1024), 10); // exact power of two
    assert_eq!(amp_count(2.0, 2), 2);
    assert_eq!(amp_count(0.1, 2), 1, "at least one append request");
}

#[test]
fn scenario_validation_rejects_bad_injections() {
    assert!(scenario(vec![inj(1, 1, 1)]).validate().is_ok());
    assert!(scenario(vec![]).validate().is_ok(), "no injections is fine");

    let mut s = scenario(vec![inj(1, 1, 1)]);
    s.sigma = 0.0;
    assert!(s.validate().is_err(), "sigma must be positive");

    assert!(
        scenario(vec![inj(0, 1, 1)]).validate().is_err(),
        "rounds are 1-based"
    );
    assert!(
        scenario(vec![inj(1, 1, 0)]).validate().is_err(),
        "op 0 is the reserved seed command"
    );
    assert!(
        scenario(vec![inj(1, 1, 7), inj(2, 2, 7)])
            .validate()
            .is_err(),
        "ops must be distinct"
    );
    assert!(
        scenario(vec![inj(1, 1, 7), inj(2, 1, 8)])
            .validate()
            .is_ok(),
        "a client may inject repeatedly: its b-th command carries sn = b"
    );
}

#[test]
fn repeated_commit_skip_needs_a_compact_or_certless_recovery_node() {
    let mut s = scenario(vec![]);
    s.repeated_commit = RepeatedCommit::Skip;
    assert!(s.validate().is_err(), "extended nodes keep no sn(c)");

    s.proto = Proto::Compact { t_commit_rounds: 8 };
    assert!(s.validate().is_ok());

    s.proto = Proto::Recovery {
        t_window_rounds: 8,
        resend_until_acked: false,
        prefix_mismatch: PrefixMismatch::Abort,
    };
    assert!(s.validate().is_ok());
    s.certs = true;
    assert!(s.validate().is_err(), "§5 would attest a skipped command");
}
