use std::collections::HashSet;

use sim::{BlockSchedule, Config, Init, Outcome, Scenario, SimState, run_traced};

fn scenario(fractions: Vec<f64>, max_rounds: usize) -> Scenario {
    Scenario {
        n: 100,
        seed: 5,
        cfg: Config::default(),
        init: Init::Split { fraction: 0.5 },
        max_rounds,
        schedule: BlockSchedule::PerRoundSticky {
            background: 0.0,
            targets: fractions,
        },
        partition: None,
    }
}

fn blocked_sets(fractions: Vec<f64>, max_rounds: usize) -> Vec<HashSet<u32>> {
    let (_, trace) = run_traced(&scenario(fractions, max_rounds));
    trace
        .rounds
        .iter()
        .map(|r| r.blocked.iter().copied().collect())
        .collect()
}

#[test]
fn shrink_releases_subset_grow_adds_to_survivors() {
    let sets = blocked_sets(vec![0.20, 0.10, 0.15], 3);
    assert_eq!(sets[0].len(), 20);
    assert_eq!(sets[1].len(), 10);
    assert_eq!(sets[2].len(), 15);
    assert!(
        sets[1].is_subset(&sets[0]),
        "shrink must release from the blocked set, not resample"
    );
    assert!(
        sets[2].is_superset(&sets[1]),
        "grow must keep every currently-blocked node"
    );
}

#[test]
fn steady_fraction_keeps_the_same_nodes_blocked() {
    let sets = blocked_sets(vec![0.20; 5], 5);
    for later in &sets[1..] {
        assert_eq!(later, &sets[0], "steady slider must not resample");
    }
}

#[test]
fn released_tail_makes_unanimity_absorbing() {
    let (outcome, trace) = run_traced(&scenario(vec![0.30, 0.30, 0.30], 500));
    assert!(matches!(outcome, Outcome::Agreement { .. }), "{outcome:?}");
    assert!(trace.rounds.len() < 500, "expected early absorbing exit");
}

#[test]
fn sticky_stepping_replays_batch_run() {
    let fractions = vec![0.20, 0.10, 0.15, 0.0, 0.25];
    let sc = scenario(fractions.clone(), 5);
    let (_, trace) = run_traced(&sc);

    let mut state = SimState::new(sc.n, sc.cfg, &sc.init, sc.seed, true, None);
    let mut mask = vec![false; sc.n];
    for fraction in &fractions {
        let count = state.blocked_count(*fraction);
        state.adjust_sticky(&mut mask, count);
        state.step_masked(&mask);
    }

    assert_eq!(
        serde_json::to_string(state.trace().expect("recording")).unwrap(),
        serde_json::to_string(&trace).unwrap()
    );
}

#[test]
fn background_union_keeps_sticky_core() {
    let (_, trace) = run_traced(&Scenario {
        n: 100,
        seed: 5,
        cfg: Config::default(),
        init: Init::Split { fraction: 0.5 },
        max_rounds: 2,
        schedule: BlockSchedule::PerRoundSticky {
            background: 0.10,
            targets: vec![0.20, 0.20],
        },
        partition: None,
    });
    let b1: HashSet<u32> = trace.rounds[0].blocked.iter().copied().collect();
    let b2: HashSet<u32> = trace.rounds[1].blocked.iter().copied().collect();
    for b in [&b1, &b2] {
        assert!(
            b.len() >= 20,
            "union must cover the sticky targets: {}",
            b.len()
        );
        assert!(
            b.len() <= 30,
            "union of 20 sticky + 10 background max: {}",
            b.len()
        );
    }
    assert!(
        b1.intersection(&b2).count() >= 20,
        "the sticky core must persist under background churn"
    );
}

#[test]
fn background_blocking_prevents_absorbing_exit() {
    let (_, trace) = run_traced(&Scenario {
        n: 50,
        seed: 3,
        cfg: Config::default(),
        init: Init::Split { fraction: 0.5 },
        max_rounds: 60,
        schedule: BlockSchedule::PerRoundSticky {
            background: 0.10,
            targets: vec![0.2],
        },
        partition: None,
    });
    assert_eq!(
        trace.rounds.len(),
        60,
        "with background noise the run must never early-exit"
    );
}
