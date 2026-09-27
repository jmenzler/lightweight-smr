use sim::{BlockSchedule, Config, Init, Outcome, Scenario, run_traced};

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

fn trace_json(schedule: BlockSchedule, max_rounds: usize) -> (Outcome, String) {
    let (outcome, trace) = run_traced(&scenario(schedule, max_rounds));
    (outcome, serde_json::to_string(&trace).unwrap())
}

#[test]
fn per_round_fractions_replays_fresh_per_round_exactly() {
    let (fresh_out, fresh_trace) = trace_json(BlockSchedule::FreshPerRound { fraction: 0.10 }, 500);
    let (per_out, per_trace) = trace_json(BlockSchedule::PerRoundFractions(vec![0.10; 500]), 500);
    assert_eq!(per_out, fresh_out);
    assert_eq!(per_trace, fresh_trace);
}

#[test]
fn empty_fractions_replay_fresh_zero() {
    let (fresh_out, fresh_trace) = trace_json(BlockSchedule::FreshPerRound { fraction: 0.0 }, 200);
    let (per_out, per_trace) = trace_json(BlockSchedule::PerRoundFractions(vec![]), 200);
    assert_eq!(per_out, fresh_out);
    assert_eq!(per_trace, fresh_trace);
}

#[test]
fn explicit_zeros_equal_implicit_padding() {
    let short = vec![0.2, 0.2, 0.2];
    let mut padded = short.clone();
    padded.resize(500, 0.0);
    let (short_out, short_trace) = trace_json(BlockSchedule::PerRoundFractions(short), 500);
    let (padded_out, padded_trace) = trace_json(BlockSchedule::PerRoundFractions(padded), 500);
    assert_eq!(short_out, padded_out);
    assert_eq!(short_trace, padded_trace);
}

#[test]
fn trailing_zeros_make_unanimity_absorbing() {
    let (outcome, trace) = run_traced(&scenario(
        BlockSchedule::PerRoundFractions(vec![0.2, 0.2, 0.2]),
        500,
    ));
    assert!(matches!(outcome, Outcome::Agreement { .. }), "{outcome:?}");
    assert!(trace.rounds.len() < 500, "expected early absorbing exit");
}
