use sim_wasm::{LiveSim, run_scenario_json};

fn split_spec(n: usize, seed: u64) -> String {
    format!(
        r#"{{"n":{n},"seed":{seed},"k":6,"ell":3,"init":{{"kind":"split","fraction":0.5}},"max_rounds":1}}"#
    )
}

fn parse(json: &str) -> serde_json::Value {
    serde_json::from_str(json).unwrap()
}

/// Step until the session reports holder-unanimity with no ⊥ left; panics if
/// it never absorbs (seed picked to converge).
fn step_to_absorption(live: &mut LiveSim) -> serde_json::Value {
    for _ in 0..500 {
        let status = parse(&live.step(0.0));
        if !status["agreed_value"].is_null() && status["all_hold"] == true {
            return status;
        }
    }
    panic!("session never absorbed");
}

#[test]
fn live_provenance_constant_fractions() {
    let mut live = LiveSim::new(&split_spec(50, 3)).unwrap();
    for _ in 0..30 {
        live.step(0.10);
    }
    let exported = live.export_scenario();
    let batch = parse(&run_scenario_json(&exported));
    assert_eq!(batch["trace"], parse(&live.trace_json()));
}

#[test]
fn live_provenance_absorbing_stop() {
    let mut live = LiveSim::new(&split_spec(50, 3)).unwrap();
    for _ in 0..3 {
        live.step(0.2);
    }
    step_to_absorption(&mut live);

    let batch = parse(&run_scenario_json(&live.export_scenario()));
    let live_trace = parse(&live.trace_json());
    assert_eq!(
        batch["trace"]["rounds"].as_array().unwrap().len(),
        live_trace["rounds"].as_array().unwrap().len(),
        "batch replay must early-exit exactly where the live session absorbed"
    );
    assert_eq!(batch["trace"], live_trace);
}

#[test]
fn step_after_absorption_is_noop() {
    let mut live = LiveSim::new(&split_spec(50, 3)).unwrap();
    for _ in 0..3 {
        live.step(0.2);
    }
    step_to_absorption(&mut live);
    let exported = live.export_scenario();
    let rounds_before = parse(&live.trace_json())["rounds"]
        .as_array()
        .unwrap()
        .len();

    for _ in 0..3 {
        let status = parse(&live.step(0.0));
        assert_eq!(status["absorbed"], true);
        assert_eq!(status["round"].as_u64().unwrap() as usize, rounds_before);
    }
    assert_eq!(
        live.export_scenario(),
        exported,
        "no-ops must not extend history"
    );
    let batch = parse(&run_scenario_json(&live.export_scenario()));
    assert_eq!(batch["trace"], parse(&live.trace_json()));
}

#[test]
fn export_scenario_records_history() {
    let mut live = LiveSim::new(&split_spec(50, 3)).unwrap();
    live.step(0.2);
    live.step(0.0);
    live.step(0.4);
    let spec = parse(&live.export_scenario());
    assert_eq!(spec["schedule"]["kind"], "per_round_fractions");
    assert_eq!(
        spec["schedule"]["fractions"],
        serde_json::json!([0.2, 0.0, 0.4])
    );
    assert_eq!(spec["max_rounds"], 3);
    assert_eq!(spec["n"], 50);
    assert_eq!(spec["seed"], 3);
}

#[test]
fn step_reports_round_state() {
    let mut live = LiveSim::new(&split_spec(50, 3)).unwrap();
    let status = parse(&live.step(0.10));
    assert_eq!(status["round"], 1);
    assert_eq!(status["states"].as_array().unwrap().len(), 50);
    assert_eq!(status["blocked"].as_array().unwrap().len(), 5);
    assert_eq!(status["targets"].as_array().unwrap().len(), 50);
    for key in [
        "holders",
        "undecided",
        "blocked",
        "useful",
        "distinct_values",
    ] {
        assert!(status["metrics"][key].is_u64(), "missing metric {key}");
    }
    assert!(status["agreed_value"].is_null() || status["agreed_value"].is_u64());
    assert!(status["all_hold"].is_boolean());
    assert!(status["all_undecided"].is_boolean());
    assert_eq!(status["absorbed"], false);
}

#[test]
fn live_new_ignores_schedule() {
    let with_schedule = r#"{"n":50,"seed":9,"k":6,"ell":3,"init":{"kind":"split","fraction":0.5},"max_rounds":1,
            "schedule":{"kind":"permanent","fraction":0.3}}"#;
    let mut a = LiveSim::new(with_schedule).unwrap();
    let mut b = LiveSim::new(&split_spec(50, 9)).unwrap();
    assert_eq!(
        a.step(0.0),
        b.step(0.0),
        "a spec schedule must not consume RNG in a live session"
    );
}

#[test]
fn live_new_rejects_invalid_spec() {
    let bad = r#"{"n":50,"seed":1,"k":6,"ell":4,"init":{"kind":"distinct"},"max_rounds":1}"#;
    let err = LiveSim::new(bad)
        .err()
        .expect("invalid spec must be rejected");
    assert!(err.contains("median-rule config requires"), "{err}");
}

#[test]
fn live_trace_strips_targets_above_2000() {
    let mut live = LiveSim::new(&split_spec(2500, 1)).unwrap();
    let status = parse(&live.step(0.0));
    assert_eq!(
        status["targets"].as_array().unwrap().len(),
        0,
        "step payload targets dropped for large n"
    );
    let trace = parse(&live.trace_json());
    assert_eq!(trace["rounds"][0]["targets"].as_array().unwrap().len(), 0);
}

fn sticky_session(n: usize, seed: u64) -> LiveSim {
    LiveSim::new_with_mode(&split_spec(n, seed), true).unwrap()
}

#[test]
fn sticky_live_keeps_blocked_set_between_steps() {
    let mut live = sticky_session(100, 5);
    let r1 = parse(&live.step(0.20));
    let r2 = parse(&live.step(0.20));
    assert_eq!(
        r1["blocked"], r2["blocked"],
        "steady slider must not resample"
    );
    let b3: Vec<u64> = parse(&live.step(0.10))["blocked"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap())
        .collect();
    let b1: Vec<u64> = r1["blocked"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap())
        .collect();
    assert_eq!(b3.len(), 10);
    assert!(
        b3.iter().all(|id| b1.contains(id)),
        "shrink releases, never resamples"
    );
}

#[test]
fn sticky_live_provenance_replays_as_batch() {
    let mut live = sticky_session(100, 5);
    for f in [0.20, 0.20, 0.10, 0.15, 0.0, 0.0] {
        live.step(f);
    }
    let exported = live.export_scenario();
    let spec = parse(&exported);
    assert_eq!(spec["schedule"]["kind"], "per_round_sticky");
    let batch = parse(&run_scenario_json(&exported));
    assert_eq!(batch["trace"], parse(&live.trace_json()));
}

#[test]
fn sticky_absorbed_noop_matches_batch_early_exit() {
    let mut live = sticky_session(50, 3);
    live.step(0.2);
    step_to_absorption(&mut live);
    let exported = live.export_scenario();
    let status = parse(&live.step(0.0));
    assert_eq!(status["absorbed"], true);
    assert_eq!(live.export_scenario(), exported);
    let batch = parse(&run_scenario_json(&exported));
    assert_eq!(batch["trace"], parse(&live.trace_json()));
}

fn sticky_bg_spec(n: usize, seed: u64, bg: f64) -> String {
    format!(
        r#"{{"n":{n},"seed":{seed},"k":6,"ell":3,"init":{{"kind":"split","fraction":0.5}},"max_rounds":1,
            "schedule":{{"kind":"fresh_per_round","fraction":{bg}}}}}"#
    )
}

#[test]
fn sticky_mode_applies_spec_schedule_as_background() {
    let mut live = LiveSim::new_with_mode(&sticky_bg_spec(100, 5, 0.10), true).unwrap();
    let r1 = parse(&live.step(0.20));
    let blocked = r1["blocked"].as_array().unwrap().len();
    assert!(
        (20..=30).contains(&blocked),
        "20 targeted + up to 10 background, got {blocked}"
    );
    let exported = parse(&live.export_scenario());
    assert_eq!(exported["schedule"]["kind"], "per_round_sticky");
    assert_eq!(exported["schedule"]["background"], 0.10);
}

#[test]
fn sticky_background_provenance_replays_as_batch() {
    let mut live = LiveSim::new_with_mode(&sticky_bg_spec(100, 5, 0.10), true).unwrap();
    for f in [0.20, 0.20, 0.10, 0.0] {
        live.step(f);
    }
    let batch = parse(&run_scenario_json(&live.export_scenario()));
    assert_eq!(batch["trace"], parse(&live.trace_json()));
}

#[test]
fn background_keeps_stepping_past_unanimity() {
    let mut live = LiveSim::new_with_mode(&sticky_bg_spec(50, 3, 0.10), true).unwrap();
    let mut absorbed_seen = false;
    for _ in 0..120 {
        let status = parse(&live.step(0.0));
        assert_eq!(
            status["absorbed"], false,
            "background noise means never absorbed"
        );
        if !status["agreed_value"].is_null() && status["all_hold"] == true {
            absorbed_seen = true;
            break;
        }
    }
    if absorbed_seen {
        let before = parse(&live.trace_json())["rounds"]
            .as_array()
            .unwrap()
            .len();
        live.step(0.0);
        let after = parse(&live.trace_json())["rounds"]
            .as_array()
            .unwrap()
            .len();
        assert_eq!(after, before + 1, "steps must continue under background");
    }
    let batch = parse(&run_scenario_json(&live.export_scenario()));
    assert_eq!(batch["trace"], parse(&live.trace_json()));
}
