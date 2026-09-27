use sim::{
    BlockSchedule, BlockTarget, BlockWindow, Config, Init, Scenario, SimState, run_report,
    run_traced,
};

fn scenario(schedule: BlockSchedule, max_rounds: usize) -> Scenario {
    Scenario {
        n: 50,
        seed: 3,
        cfg: Config::default(),
        init: Init::Split { fraction: 0.5 },
        max_rounds,
        schedule,
        partition: None,
    }
}

#[test]
fn sampled_stepping_replays_fresh_per_round_run() {
    let sc = scenario(BlockSchedule::FreshPerRound { fraction: 0.10 }, 500);
    let (_, trace) = run_traced(&sc);
    let report = run_report(&sc);

    let mut state = SimState::new(sc.n, sc.cfg, &sc.init, sc.seed, true, None);
    for _ in 0..trace.rounds.len() {
        state.step_sampled(5);
    }

    let stepped = serde_json::to_string(state.trace().expect("recording")).unwrap();
    let reference = serde_json::to_string(&trace).unwrap();
    assert_eq!(stepped, reference);
    assert_eq!(state.metrics(), report.metrics.as_slice());
}

#[test]
fn masked_stepping_replays_nodes_window_run() {
    let sc = scenario(
        BlockSchedule::Windows(vec![BlockWindow {
            start_round: 1,
            rounds: 200,
            target: BlockTarget::Nodes(vec![0, 1, 2, 3, 4]),
        }]),
        500,
    );
    let (_, trace) = run_traced(&sc);
    let report = run_report(&sc);

    let mut window_mask = vec![false; sc.n];
    window_mask[..5].fill(true);
    let empty_mask = vec![false; sc.n];

    let mut state = SimState::new(sc.n, sc.cfg, &sc.init, sc.seed, true, None);
    for round in 1..=trace.rounds.len() {
        let mask = if round <= 200 {
            &window_mask
        } else {
            &empty_mask
        };
        state.step_masked(mask);
    }

    let stepped = serde_json::to_string(state.trace().expect("recording")).unwrap();
    let reference = serde_json::to_string(&trace).unwrap();
    assert_eq!(stepped, reference);
    assert_eq!(state.metrics(), report.metrics.as_slice());
}

#[test]
fn trace_free_stepping_matches_metrics() {
    let sc = scenario(BlockSchedule::FreshPerRound { fraction: 0.10 }, 500);
    let report = run_report(&sc);

    let mut state = SimState::new(sc.n, sc.cfg, &sc.init, sc.seed, false, None);
    for _ in 0..report.metrics.len() {
        state.step_sampled(5);
    }

    assert!(state.trace().is_none());
    assert_eq!(state.metrics(), report.metrics.as_slice());
}

#[test]
fn run_traced_report_matches_separate_runs() {
    let sc = scenario(BlockSchedule::FreshPerRound { fraction: 0.10 }, 500);
    let (report, trace) = sim::run_traced_report(&sc);
    let (outcome, reference_trace) = run_traced(&sc);
    let reference_report = run_report(&sc);
    assert_eq!(report.outcome, outcome);
    assert_eq!(report.metrics, reference_report.metrics);
    assert_eq!(
        serde_json::to_string(&trace).unwrap(),
        serde_json::to_string(&reference_trace).unwrap()
    );
}
