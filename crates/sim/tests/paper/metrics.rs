use protocol::Config;
use sim::{BlockSchedule, Init, Outcome, Scenario, run_report, run_traced};

fn scenario(
    n: usize,
    seed: u64,
    init: Init,
    schedule: BlockSchedule,
    max_rounds: usize,
) -> Scenario {
    Scenario {
        n,
        seed,
        cfg: Config::default(),
        init,
        max_rounds,
        schedule,
        partition: None,
    }
}

#[test]
fn one_metrics_row_per_simulated_round() {
    let s = scenario(
        100,
        7,
        Init::Split { fraction: 0.5 },
        BlockSchedule::FreshPerRound { fraction: 0.1 },
        200,
    );
    let report = run_report(&s);
    let (_, trace) = run_traced(&s);
    assert_eq!(report.metrics.len(), trace.rounds.len());
}

#[test]
fn same_seed_same_metrics() {
    let s = scenario(
        150,
        3,
        Init::UniformRandom { k: 4 },
        BlockSchedule::Permanent { fraction: 0.15 },
        100,
    );
    assert_eq!(run_report(&s).metrics, run_report(&s).metrics);
}

#[test]
fn counters_match_the_trace_exactly() {
    // The trace is the ground truth: recompute every counter from it.
    // useful_r = snapshot entering round r (initial for r=1) ∧ not blocked in r;
    // holders/undecided/distinct describe the states AFTER round r's step.
    let s = scenario(
        200,
        11,
        Init::WithUndecided {
            useful_fraction: 0.6,
            inner: Box::new(Init::UniformRandom { k: 3 }),
        },
        BlockSchedule::FreshPerRound { fraction: 0.2 },
        150,
    );
    let report = run_report(&s);
    let (_, trace) = run_traced(&s);

    let mut snapshot = trace.initial.clone();
    for (row, round) in report.metrics.iter().zip(&trace.rounds) {
        let blocked: std::collections::HashSet<u32> = round.blocked.iter().copied().collect();
        let useful = snapshot
            .iter()
            .enumerate()
            .filter(|(i, s)| s.is_some() && !blocked.contains(&(*i as u32)))
            .count() as u32;
        let holders = round.states.iter().flatten().count() as u32;
        let distinct = round
            .states
            .iter()
            .flatten()
            .collect::<std::collections::HashSet<_>>()
            .len() as u32;

        assert_eq!(row.useful, useful);
        assert_eq!(row.blocked, round.blocked.len() as u32);
        assert_eq!(row.holders, holders);
        assert_eq!(row.undecided, s.n as u32 - holders);
        assert_eq!(row.distinct_values, distinct);
        snapshot = round.states.clone();
    }
}

#[test]
fn lemma_2_2_useful_fraction_converges_to_three_quarters_and_holds() {
    // E2 regime (b): from ≥ 1/2 useful under β = 1/10 fresh-per-round, the
    // useful fraction reaches ≥ 3/4 and never drops below it again.
    // "Useful" is the paper's sense (non-⊥ ∧ non-blocked), so the ≥ 1/2
    // precondition needs initial HOLDERS ≥ 1/(2(1−β)) = 5/9 — a holder
    // fraction of exactly 1/2 sits on the unstable fixed point and dies.
    let s = scenario(
        4000,
        42,
        Init::WithUndecided {
            useful_fraction: 0.56,
            inner: Box::new(Init::Split { fraction: 0.5 }),
        },
        BlockSchedule::FreshPerRound { fraction: 0.10 },
        300,
    );
    let report = run_report(&s);
    assert!(
        matches!(report.outcome, Outcome::Agreement { .. }),
        "got {:?}",
        report.outcome
    );
    let threshold = (0.75 * 4000.0) as u32;
    let crossing = report
        .metrics
        .iter()
        .position(|row| row.useful >= threshold)
        .expect("useful fraction never reached 3/4");
    for (i, row) in report.metrics.iter().enumerate().skip(crossing) {
        assert!(
            row.useful >= threshold,
            "useful dropped below 3/4 at round {} after crossing at {}",
            i + 1,
            crossing + 1
        );
    }
}
