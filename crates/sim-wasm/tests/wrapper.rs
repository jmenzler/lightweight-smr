use sim_wasm::run_scenario_json;

fn spec(n: usize, seed: u64, beta: f64, max_rounds: usize) -> String {
    format!(
        r#"{{"n":{n},"seed":{seed},"k":6,"ell":3,"init":{{"kind":"split","fraction":0.5}},
            "max_rounds":{max_rounds},"schedule":{{"kind":"fresh_per_round","fraction":{beta}}}}}"#
    )
}

#[test]
fn run_scenario_json_matches_native_run_traced() {
    // canonical-seed promise: WASM wrapper = same trace as CLI for same inputs
    let out: serde_json::Value =
        serde_json::from_str(&run_scenario_json(&spec(50, 3, 0.10, 500))).unwrap();
    let (_, trace) = sim::run_traced(&sim::Scenario {
        n: 50,
        seed: 3,
        cfg: sim::Config::default(),
        init: sim::Init::Split { fraction: 0.5 },
        max_rounds: 500,
        schedule: sim::BlockSchedule::FreshPerRound { fraction: 0.10 },
        partition: None,
    });
    let direct: serde_json::Value =
        serde_json::from_str(&serde_json::to_string(&trace).unwrap()).unwrap();
    assert_eq!(out["trace"], direct);
}

#[test]
fn run_scenario_json_reports_outcome_and_metrics() {
    let out: serde_json::Value =
        serde_json::from_str(&run_scenario_json(&spec(100, 7, 0.10, 500))).unwrap();
    assert!(out["report"]["outcome"].is_object(), "{out}");
    assert_eq!(
        out["report"]["metrics"].as_array().unwrap().len(),
        out["trace"]["rounds"].as_array().unwrap().len()
    );
}

#[test]
fn invalid_spec_returns_error_json() {
    let bad = r#"{"n":100,"seed":1,"k":6,"ell":4,"init":{"kind":"distinct"},"max_rounds":10}"#;
    let out: serde_json::Value = serde_json::from_str(&run_scenario_json(bad)).unwrap();
    let err = out["error"].as_str().expect("error field");
    assert!(err.contains("median-rule config requires"), "{err}");

    let malformed: serde_json::Value =
        serde_json::from_str(&run_scenario_json("not json")).unwrap();
    assert!(malformed["error"].is_string());
}

#[test]
fn targets_stripped_above_2000_nodes() {
    let big: serde_json::Value =
        serde_json::from_str(&run_scenario_json(&spec(2500, 1, 0.05, 50))).unwrap();
    assert_eq!(
        big["trace"]["rounds"][0]["targets"]
            .as_array()
            .unwrap()
            .len(),
        0,
        "targets dropped for large n to bound trace size"
    );
    let small: serde_json::Value =
        serde_json::from_str(&run_scenario_json(&spec(100, 1, 0.05, 50))).unwrap();
    assert_eq!(
        small["trace"]["rounds"][0]["targets"]
            .as_array()
            .unwrap()
            .len(),
        100
    );
}
