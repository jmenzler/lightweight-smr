use sim::BlockSchedule;
use sim::smr::{Proto, SmrScenario, TrafficPhase};
use sim::spec::SmrScenarioSpec;

fn parse(json: &str) -> Result<SmrScenario, String> {
    let spec: SmrScenarioSpec = serde_json::from_str(json).map_err(|e| e.to_string())?;
    SmrScenario::try_from(spec)
}

#[test]
fn traffic_kinds_normalize_to_phases() {
    let base = |traffic: &str| {
        format!(
            r#"{{"n":16,"seed":1,"k":6,"ell":3,"sigma":1.0,
               "proto":{{"kind":"extended"}},
               "injections":[],"max_rounds":40,"traffic":{traffic}}}"#
        )
    };
    let constant = parse(&base(r#"{"kind":"pmf","arrivals_pmf":[0.0,1.0]}"#)).unwrap();
    assert_eq!(
        constant.traffic,
        Some(vec![TrafficPhase {
            from_round: 1,
            arrivals_pmf: vec![0.0, 1.0],
        }]),
        "pmf kind is sugar for one phase at round 1"
    );

    let phased = parse(&base(
        r#"{"kind":"phases","phases":[
            {"from_round":1,"arrivals_pmf":[0.0,1.0]},
            {"from_round":9,"arrivals_pmf":[1.0]}]}"#,
    ))
    .unwrap();
    assert_eq!(
        phased.traffic.as_ref().map(Vec::len),
        Some(2),
        "phase list carried through"
    );

    assert!(
        parse(&base(r#"{"kind":"pmf","arrivals_pmf":[-1.0]}"#)).is_err(),
        "bad pmf routes through domain validation"
    );
    assert!(
        parse(&base(
            r#"{"kind":"phases","phases":[
                {"from_round":9,"arrivals_pmf":[1.0]},
                {"from_round":1,"arrivals_pmf":[1.0]}]}"#
        ))
        .is_err(),
        "phase order routes through domain validation"
    );
    assert!(
        parse(&base(r#"{"kind":"pmf","bogus":[1.0]}"#)).is_err(),
        "unknown traffic fields denied"
    );

    let untouched = parse(
        r#"{"n":16,"seed":1,"k":6,"ell":3,"sigma":1.0,"proto":{"kind":"extended"},
           "injections":[],"max_rounds":40}"#,
    )
    .unwrap();
    assert_eq!(untouched.traffic, None, "no traffic field = no traffic");
}

#[test]
fn phase_boundary_switches_realized_arrivals() {
    // Point-mass pmfs make realized arrivals deterministic: 1/round until
    // round 5, 3/round from round 6 on.
    let scenario = parse(
        r#"{"n":16,"seed":1,"k":6,"ell":3,"sigma":1.0,
            "proto":{"kind":"extended"},
            "injections":[],"max_rounds":12,
            "traffic":{"kind":"phases","phases":[
                {"from_round":1,"arrivals_pmf":[0.0,1.0]},
                {"from_round":6,"arrivals_pmf":[0.0,0.0,0.0,1.0]}]}}"#,
    )
    .unwrap();
    let phases = scenario.traffic.as_ref().expect("traffic set");
    assert_eq!(
        phases.iter().map(|p| p.from_round).collect::<Vec<_>>(),
        vec![1, 6],
        "phase order carried through conversion"
    );

    let report = sim::smr::run_smr(&scenario);
    assert_eq!(report.metrics.len(), 12);
    for (i, m) in report.metrics.iter().enumerate() {
        let round = i + 1;
        let expected = if round < 6 { 1 } else { 3 };
        assert_eq!(m.arrivals, expected, "round {round}");
    }
}

#[test]
fn extended_spec_round_trips_into_a_validated_scenario() {
    let s = parse(
        r#"{"n":32,"seed":7,"k":6,"ell":3,"sigma":1.5,
            "proto":{"kind":"extended"},
            "injections":[{"round":2,"client":1,"op":9}],
            "max_rounds":50,
            "schedule":{"kind":"fresh_per_round","fraction":0.1}}"#,
    )
    .unwrap();
    assert_eq!(s.proto, Proto::Extended);
    assert_eq!(s.injections[0].target, None);
    assert!(matches!(s.schedule, BlockSchedule::FreshPerRound { fraction } if fraction == 0.1));
}

#[test]
fn compact_spec_carries_t_commit_and_pinned_target() {
    let s = parse(
        r#"{"n":16,"seed":1,"k":6,"ell":3,"sigma":1.0,
            "proto":{"kind":"compact","t_commit_rounds":8},
            "injections":[{"round":2,"client":1,"op":9,"target":5}],
            "max_rounds":40}"#,
    )
    .unwrap();
    assert_eq!(s.proto, Proto::Compact { t_commit_rounds: 8 });
    assert_eq!(s.injections[0].target, Some(5));
    assert!(matches!(s.schedule, BlockSchedule::FreshPerRound { fraction } if fraction == 0.0));
}

#[test]
fn invariants_route_through_domain_validation() {
    let base = |patch: &str| {
        format!(
            r#"{{"n":16,"seed":1,"k":6,"ell":3,"sigma":1.0,
               "proto":{{"kind":"extended"}},
               "injections":[{patch}],
               "max_rounds":40}}"#
        )
    };
    assert!(parse(&base(r#"{"round":2,"client":1,"op":9,"target":16}"#)).is_err());
    assert!(parse(&base(r#"{"round":0,"client":1,"op":9}"#)).is_err());
    assert!(
        parse(&base(
            r#"{"round":2,"client":1,"op":9},{"round":3,"client":1,"op":10}"#
        ))
        .is_ok(),
        "a client may inject repeatedly: its b-th command carries sn = b"
    );
    assert!(
        parse(
            r#"{"n":16,"seed":1,"k":6,"ell":4,"sigma":1.0,"proto":{"kind":"extended"},
               "injections":[],"max_rounds":40}"#
        )
        .is_err(),
        "even ell rejected by Config::new"
    );
    assert!(
        parse(
            r#"{"n":16,"seed":1,"k":6,"ell":3,"sigma":1.0,"proto":{"kind":"extended"},
               "injections":[],"max_rounds":40,"bogus":1}"#
        )
        .is_err(),
        "unknown fields denied"
    );
}
