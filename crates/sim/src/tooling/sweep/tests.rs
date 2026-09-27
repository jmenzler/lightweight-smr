use super::*;

const STRIP_PANIC: &str =
    "P must be a prefix of L: strip would corrupt the log (pre len 7, log len 17 at round 40)";

#[test]
fn a_split_brain_row_is_identical_with_and_without_truncation() {
    let scenario = crate::smr::SmrScenario {
        n: 64,
        seed: 950_000,
        cfg: crate::Config::default(),
        sigma: 1.0,
        proto: Proto::Compact {
            t_commit_rounds: 21,
        },
        injections: vec![],
        max_rounds: 400,
        schedule: crate::BlockSchedule::FreshPerRound { fraction: 0.0 },
        traffic: Some(vec![crate::smr::TrafficPhase {
            from_round: 1,
            arrivals_pmf: vec![0.0, 0.0, 0.0, 0.0, 1.0],
        }]),
        client_model: crate::smr::ClientModel::Unique,
        certs: false,
        manual_blocks: Vec::new(),
        partition: None,
        halt_on_violation: false,
        merge_policy: Default::default(),
        repeated_commit: Default::default(),
    };
    let key = SmrRowKey {
        k: 6,
        ell: 3,
        n: 64,
        beta: Some(0.0),
        sigma: Some(1.0),
        rate: Some(4),
        t_commit: Some(21),
        split: None,
        horizon: Some(400),
        seed: 950_000,
    };
    let grid = crate::smr::run_smr_grid(&scenario);
    let reference = crate::smr::run_smr(&scenario);
    assert!(!grid.safety_ok, "the cell must still split its brain");
    assert!(!reference.safety_ok);
    assert_eq!(
        smr_row("split", &key, scenario.proto, &grid),
        smr_row("split", &key, scenario.proto, &reference)
    );
}

#[test]
fn clean_values_pass_through() {
    assert_eq!(capture_strip_abort(|| 7).expect("no panic"), 7);
}

#[test]
fn strip_abort_is_captured_with_its_round() {
    let abort = capture_strip_abort::<()>(|| panic!("{STRIP_PANIC}")).expect_err("captured");
    assert_eq!(abort.round, Some(40));
    assert_eq!(abort.kind, AbortKind::Prefix);
}

#[test]
fn strip_abort_without_a_round_still_captures() {
    let abort =
        capture_strip_abort::<()>(|| panic!("P must be a prefix of L")).expect_err("captured");
    assert_eq!(abort.round, None);
    assert_eq!(abort.kind, AbortKind::Prefix);
}

#[test]
fn mono_abort_is_captured_with_its_round() {
    let abort = capture_strip_abort::<()>(|| {
        panic!("driver bug or split brain: node 3 executed sequence regressed in round 17")
    })
    .expect_err("captured");
    assert_eq!(abort.round, Some(17));
    assert_eq!(abort.kind, AbortKind::Mono);
}

#[test]
#[should_panic(expected = "an unrecognised engine failure")]
fn other_panics_are_not_swallowed() {
    let _ = capture_strip_abort::<()>(|| panic!("an unrecognised engine failure"));
}
