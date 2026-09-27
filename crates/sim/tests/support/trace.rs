use sim::{BlockSchedule, Init, Outcome, Scenario, run, run_traced};

fn scenario() -> Scenario {
    Scenario {
        n: 40,
        seed: 5,
        cfg: protocol::Config::default(),
        init: Init::Split { fraction: 0.5 },
        max_rounds: 500,
        schedule: BlockSchedule::FreshPerRound { fraction: 0.1 },
        partition: None,
    }
}

#[test]
fn blocked_run_simulates_to_max_rounds_and_agreement_persists() {
    let (outcome, trace) = run_traced(&scenario());
    let rounds = match outcome {
        Outcome::Agreement { rounds, .. } => rounds,
        other => panic!("expected agreement, got {other:?}"),
    };
    // under blocking, agreement must be proven persistent to max_rounds
    assert_eq!(trace.rounds.len(), 500);

    let distinct = |states: &[Option<u64>]| {
        states
            .iter()
            .flatten()
            .collect::<std::collections::HashSet<_>>()
            .len()
    };
    // `rounds` is the FIRST unanimous round: mixed before, unanimous ever after
    assert!(rounds >= 2, "seed 5 starts split, cannot agree in round 1");
    assert!(distinct(&trace.rounds[rounds - 2].states) >= 2);
    for round in &trace.rounds[rounds - 1..] {
        assert_eq!(distinct(&round.states), 1);
    }
}

#[test]
fn trace_round_zero_holds_initial_inputs() {
    let (_, trace) = run_traced(&scenario());
    // Split 0.5 over 40 nodes: first 20 hold 0, rest hold 1
    assert_eq!(trace.initial.len(), 40);
    assert!(trace.initial[..20].iter().all(|s| *s == Some(0)));
    assert!(trace.initial[20..].iter().all(|s| *s == Some(1)));
}

#[test]
fn trace_records_k_sampled_targets_per_node() {
    let (_, trace) = run_traced(&scenario());
    for round in &trace.rounds {
        assert_eq!(round.targets.len(), 40);
        for t in &round.targets {
            assert_eq!(t.len(), 6, "each node samples k=6 targets");
        }
    }
}

#[test]
fn trace_records_blocked_set_of_expected_size() {
    let (_, trace) = run_traced(&scenario());
    for round in &trace.rounds {
        assert_eq!(round.blocked.len(), 4, "10% of 40 nodes blocked");
        assert!(round.blocked.iter().all(|&b| (b as usize) < 40));
    }
}

#[test]
fn traced_run_matches_untraced_run() {
    // tracing must not perturb the RNG stream
    let plain = run(&scenario());
    let (traced, _) = run_traced(&scenario());
    assert_eq!(plain, traced);
}

#[test]
fn trace_serializes_to_json() {
    let (_, trace) = run_traced(&scenario());
    let json = serde_json::to_string(&trace).expect("serialize");
    assert!(json.contains("\"rounds\""));
    assert!(json.len() > 100);
}
