use sim::spec::ScenarioSpec;
use sim::{BlockSchedule, BlockTarget, BlockWindow, Init, Scenario};

fn parse(json: &str) -> Result<Scenario, String> {
    let spec: ScenarioSpec = serde_json::from_str(json).map_err(|e| e.to_string())?;
    Scenario::try_from(spec)
}

const REGIME_A: &str = r#"{"n":1000,"seed":81000,"k":6,"ell":3,
    "init":{"kind":"with_undecided","useful_fraction":0.30,"inner":{"kind":"split","fraction":0.5}},
    "max_rounds":200,"schedule":{"kind":"fresh_per_round","fraction":0.0}}"#;
const REGIME_B: &str = r#"{"n":1000,"seed":82000,"k":6,"ell":3,
    "init":{"kind":"with_undecided","useful_fraction":0.5555555555555556,"inner":{"kind":"split","fraction":0.5}},
    "max_rounds":500,"schedule":{"kind":"fresh_per_round","fraction":0.10}}"#;
const REGIME_C: &str = r#"{"n":1000,"seed":83000,"k":6,"ell":3,
    "init":{"kind":"split","fraction":0.5},
    "max_rounds":500,"schedule":{"kind":"permanent","fraction":0.30}}"#;

#[test]
fn parses_e2_regime_a_verbatim() {
    let sc = parse(REGIME_A).unwrap();
    assert_eq!((sc.n, sc.seed, sc.max_rounds), (1000, 81000, 200));
    assert_eq!((sc.cfg.k, sc.cfg.ell), (6, 3));
    match &sc.init {
        Init::WithUndecided {
            useful_fraction,
            inner,
        } => {
            assert_eq!(*useful_fraction, 0.30);
            assert!(matches!(**inner, Init::Split { fraction } if fraction == 0.5));
        }
        other => panic!("wrong init: {other:?}"),
    }
    assert_eq!(sc.schedule, BlockSchedule::FreshPerRound { fraction: 0.0 });
}

#[test]
fn parses_e2_regime_b_verbatim() {
    let sc = parse(REGIME_B).unwrap();
    assert_eq!(sc.seed, 82000);
    match &sc.init {
        Init::WithUndecided {
            useful_fraction, ..
        } => assert_eq!(*useful_fraction, 5.0 / 9.0),
        other => panic!("wrong init: {other:?}"),
    }
    assert_eq!(sc.schedule, BlockSchedule::FreshPerRound { fraction: 0.10 });
}

#[test]
fn parses_e2_regime_c_verbatim() {
    let sc = parse(REGIME_C).unwrap();
    assert!(matches!(sc.init, Init::Split { fraction } if fraction == 0.5));
    assert_eq!(sc.schedule, BlockSchedule::Permanent { fraction: 0.30 });
}

#[test]
fn rejects_invalid_config() {
    for (k, ell) in [(6, 4), (6, 1)] {
        let json = format!(
            r#"{{"n":100,"seed":1,"k":{k},"ell":{ell},"init":{{"kind":"distinct"}},"max_rounds":10}}"#
        );
        let err = parse(&json).unwrap_err();
        assert!(err.contains("median-rule config requires"), "{err}");
    }
}

#[test]
fn rejects_out_of_range_fractions() {
    let cases = [
        r#"{"n":100,"seed":1,"k":6,"ell":3,"init":{"kind":"split","fraction":1.5},"max_rounds":10}"#,
        r#"{"n":100,"seed":1,"k":6,"ell":3,"init":{"kind":"with_undecided","useful_fraction":-0.1,"inner":{"kind":"distinct"}},"max_rounds":10}"#,
        r#"{"n":100,"seed":1,"k":6,"ell":3,"init":{"kind":"distinct"},"max_rounds":10,"schedule":{"kind":"fresh_per_round","fraction":1.2}}"#,
        r#"{"n":100,"seed":1,"k":6,"ell":3,"init":{"kind":"distinct"},"max_rounds":10,"schedule":{"kind":"per_round_fractions","fractions":[0.1,2.0]}}"#,
        r#"{"n":100,"seed":1,"k":6,"ell":3,"init":{"kind":"distinct"},"max_rounds":10,"schedule":{"kind":"windows","windows":[{"start_round":1,"rounds":5,"target":{"kind":"sample_fraction","fraction":-0.2}}]}}"#,
    ];
    for json in cases {
        assert!(parse(json).is_err(), "accepted: {json}");
    }
}

#[test]
fn rejects_nested_with_undecided() {
    let json = r#"{"n":100,"seed":1,"k":6,"ell":3,
        "init":{"kind":"with_undecided","useful_fraction":0.5,
                "inner":{"kind":"with_undecided","useful_fraction":0.5,"inner":{"kind":"distinct"}}},
        "max_rounds":10}"#;
    let err = parse(json).unwrap_err();
    assert!(err.contains("nested"), "{err}");
}

#[test]
fn defaults_schedule_to_fresh_zero() {
    let json = r#"{"n":100,"seed":1,"k":6,"ell":3,"init":{"kind":"distinct"},"max_rounds":10}"#;
    let sc = parse(json).unwrap();
    assert_eq!(sc.schedule, BlockSchedule::FreshPerRound { fraction: 0.0 });
}

#[test]
fn parses_windows_schedule() {
    let json = r#"{"n":100,"seed":1,"k":6,"ell":3,"init":{"kind":"distinct"},"max_rounds":10,
        "schedule":{"kind":"windows","windows":[
            {"start_round":3,"rounds":5,"target":{"kind":"nodes","ids":[0,1,2]}},
            {"start_round":4,"rounds":2,"target":{"kind":"sample_fraction","fraction":0.2}}]}}"#;
    let sc = parse(json).unwrap();
    assert_eq!(
        sc.schedule,
        BlockSchedule::Windows(vec![
            BlockWindow {
                start_round: 3,
                rounds: 5,
                target: BlockTarget::Nodes(vec![0, 1, 2]),
            },
            BlockWindow {
                start_round: 4,
                rounds: 2,
                target: BlockTarget::SampleFraction(0.2),
            },
        ])
    );
}

#[test]
fn rejects_zero_round_window() {
    for bad in [
        r#"{"start_round":1,"rounds":0,"target":{"kind":"nodes","ids":[0]}}"#,
        r#"{"start_round":0,"rounds":3,"target":{"kind":"nodes","ids":[0]}}"#,
        r#"{"start_round":1,"rounds":3,"target":{"kind":"nodes","ids":[100]}}"#,
    ] {
        let json = format!(
            r#"{{"n":100,"seed":1,"k":6,"ell":3,"init":{{"kind":"distinct"}},"max_rounds":10,
                "schedule":{{"kind":"windows","windows":[{bad}]}}}}"#
        );
        assert!(parse(&json).is_err(), "accepted: {bad}");
    }
}

#[test]
fn rejects_unknown_fields() {
    let json = r#"{"n":100,"seed":1,"k":6,"ell":3,"init":{"kind":"distinct"},"max_rounds":10,"schedle":{"kind":"permanent","fraction":0.3}}"#;
    assert!(parse(json).is_err(), "typo'd key must error, not default");
}

#[test]
fn spec_serialize_roundtrip() {
    let spec: ScenarioSpec = serde_json::from_str(REGIME_B).unwrap();
    let json = serde_json::to_string(&spec).unwrap();
    let again: ScenarioSpec = serde_json::from_str(&json).unwrap();
    let sc = Scenario::try_from(again).unwrap();
    assert_eq!(sc.schedule, BlockSchedule::FreshPerRound { fraction: 0.10 });
}

#[test]
fn parses_per_round_sticky_schedule() {
    let json = r#"{"n":100,"seed":1,"k":6,"ell":3,"init":{"kind":"distinct"},"max_rounds":10,
        "schedule":{"kind":"per_round_sticky","fractions":[0.2,0.1,0.15]}}"#;
    let sc = parse(json).unwrap();
    assert_eq!(
        sc.schedule,
        BlockSchedule::PerRoundSticky {
            background: 0.0,
            targets: vec![0.2, 0.1, 0.15]
        },
        "background must default to 0 so pre-background exports stay valid"
    );
    let bad = r#"{"n":100,"seed":1,"k":6,"ell":3,"init":{"kind":"distinct"},"max_rounds":10,
        "schedule":{"kind":"per_round_sticky","fractions":[0.2,1.4]}}"#;
    assert!(parse(bad).is_err());
}

#[test]
fn parses_sticky_background_and_validates_range() {
    let json = r#"{"n":100,"seed":1,"k":6,"ell":3,"init":{"kind":"distinct"},"max_rounds":10,
        "schedule":{"kind":"per_round_sticky","fractions":[0.2],"background":0.1}}"#;
    let sc = parse(json).unwrap();
    assert_eq!(
        sc.schedule,
        BlockSchedule::PerRoundSticky {
            background: 0.1,
            targets: vec![0.2]
        }
    );
    let bad = r#"{"n":100,"seed":1,"k":6,"ell":3,"init":{"kind":"distinct"},"max_rounds":10,
        "schedule":{"kind":"per_round_sticky","fractions":[0.2],"background":1.5}}"#;
    assert!(parse(bad).is_err());
}

#[test]
fn parses_even_split_and_weighted_inits() {
    let even = r#"{"n":100,"seed":1,"k":6,"ell":3,"init":{"kind":"even_split","values":4},"max_rounds":10}"#;
    assert!(matches!(
        parse(even).unwrap().init,
        Init::EvenSplit { values: 4 }
    ));
    let weighted = r#"{"n":100,"seed":1,"k":6,"ell":3,"init":{"kind":"weighted","weights":[1.0,0.0,3.0]},"max_rounds":10}"#;
    match parse(weighted).unwrap().init {
        Init::Weighted { weights, range } => {
            assert_eq!(weights, vec![1.0, 0.0, 3.0]);
            assert_eq!(range, 3);
        }
        other => panic!("wrong init: {other:?}"),
    }
}

#[test]
fn rejects_degenerate_weighted_and_even_split() {
    for bad in [
        r#"{"kind":"weighted","weights":[]}"#,
        r#"{"kind":"weighted","weights":[0.0,0.0]}"#,
        r#"{"kind":"weighted","weights":[1.0,-0.5]}"#,
        r#"{"kind":"even_split","values":0}"#,
    ] {
        let json = format!(r#"{{"n":100,"seed":1,"k":6,"ell":3,"init":{bad},"max_rounds":10}}"#);
        assert!(parse(&json).is_err(), "accepted: {bad}");
    }
}

#[test]
fn weighted_range_defaults_dense_and_validates() {
    let dense = r#"{"n":100,"seed":1,"k":6,"ell":3,"init":{"kind":"weighted","weights":[1.0,3.0]},"max_rounds":10}"#;
    match parse(dense).unwrap().init {
        Init::Weighted { range, .. } => assert_eq!(range, 2, "no range field = dense"),
        other => panic!("wrong init: {other:?}"),
    }
    let sparse = r#"{"n":100,"seed":1,"k":6,"ell":3,"init":{"kind":"weighted","weights":[1.0,3.0],"range":4294967296},"max_rounds":10}"#;
    match parse(sparse).unwrap().init {
        Init::Weighted { range, .. } => assert_eq!(range, 1 << 32),
        other => panic!("wrong init: {other:?}"),
    }
    let bad = r#"{"n":100,"seed":1,"k":6,"ell":3,"init":{"kind":"weighted","weights":[1.0,3.0,1.0],"range":2},"max_rounds":10}"#;
    assert!(
        parse(bad).is_err(),
        "range below bucket count must be rejected"
    );
}
