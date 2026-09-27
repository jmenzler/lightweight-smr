use sim::smr::{RepeatedCommit, SmrScenario, SmrState, run_smr_grid_observed};
use sim::spec::SmrScenarioSpec;

/// n=256 near-miss at the rate-4 boundary: one command executed twice two rounds before the split.
fn near_miss(max_rounds: usize) -> SmrScenario {
    let spec: SmrScenarioSpec = serde_json::from_value(serde_json::json!({
        "n": 256, "seed": 971_007, "k": 6, "ell": 3, "sigma": 1.0,
        "proto": {"kind": "compact", "t_commit_rounds": 28},
        "injections": [], "max_rounds": max_rounds,
        "schedule": {"kind": "fresh_per_round", "fraction": 0.1},
        "traffic": {"kind": "pmf", "arrivals_pmf": [0.0, 0.0, 0.0, 0.0, 1.0]},
    }))
    .expect("spec parses");
    SmrScenario::try_from(spec).expect("valid scenario")
}

#[test]
fn a_double_execution_trips_before_the_prefix_latch() {
    let mut safety_round = None;
    let mut dup_round = None;
    let report = run_smr_grid_observed(&near_miss(480), &mut |st: &SmrState| {
        let round = st.round();
        if safety_round.is_none() && !st.safety_ok() {
            safety_round = Some(round);
        }
        if dup_round.is_none() && !st.exec_unique_ok() {
            dup_round = Some(round);
        }
    });
    assert_eq!(dup_round, Some(477));
    assert_eq!(safety_round, Some(479));
    assert_eq!(report.exec_dup_round, Some(477));
    assert!(!report.safety_ok);
}

#[test]
fn before_the_repeat_the_run_serializes_as_today() {
    let report = sim::smr::run_smr_grid(&near_miss(476));
    assert!(report.safety_ok);
    assert_eq!(report.exec_dup_round, None);
    let json = serde_json::to_string(&report).unwrap();
    assert!(
        !json.contains("exec_dup_round"),
        "a clean run's report must keep its bytes"
    );
}

#[test]
fn the_execute_once_guard_keeps_the_repeat_off_every_server() {
    let mut guarded = near_miss(480);
    guarded.repeated_commit = RepeatedCommit::Skip;
    let report = sim::smr::run_smr_grid(&guarded);
    assert_eq!(report.exec_dup_round, None);
    assert!(
        report.repeat_skips > 0,
        "the suppressed repeat is still reported"
    );
    let first = report.first_repeat_skip_round.expect("a first skip round");
    let outcome = ledger_outcome(&guarded, "guarded");
    assert_eq!(outcome["repeat_skips"], report.repeat_skips);
    assert_eq!(outcome["first_repeat_skip_round"], first);
}

fn ledger_outcome(scenario: &SmrScenario, tag: &str) -> serde_json::Value {
    let dir = std::env::temp_dir().join(format!("exec-dup-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let ledger = dir.join("ledger.jsonl");
    let report = sim::smr::run_smr_grid(scenario);
    sim::runlog::log_smr_run_to(&ledger, scenario, &report, None).unwrap();
    let row: serde_json::Value =
        serde_json::from_str(std::fs::read_to_string(&ledger).unwrap().trim()).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    row["outcome"].clone()
}

#[test]
fn the_ledger_carries_the_round_only_when_it_tripped() {
    let tripped = ledger_outcome(&near_miss(480), "tripped");
    assert_eq!(tripped["exec_dup_round"], 477);
    let clean = ledger_outcome(&near_miss(476), "clean");
    assert!(
        clean.get("exec_dup_round").is_none(),
        "a clean row must keep its bytes: {clean}"
    );
    assert!(clean.get("repeat_skips").is_none(), "{clean}");
    assert!(clean.get("first_repeat_skip_round").is_none(), "{clean}");
}
