use sim::smr::{
    AUTO_CLIENT_BASE, AUTO_OP_BASE, ClientModel, CommandStatus, Injection, Proto, SmrScenario,
    SmrState, SmrTerminal, TrafficPhase, run_smr,
};
use sim::{BlockSchedule, Config};

fn scenario(traffic: Option<Vec<TrafficPhase>>) -> SmrScenario {
    SmrScenario {
        n: 16,
        seed: 7,
        cfg: Config::default(),
        sigma: 1.0,
        proto: Proto::Extended,
        injections: vec![],
        max_rounds: 20,
        schedule: BlockSchedule::FreshPerRound { fraction: 0.0 },
        traffic,
        client_model: ClientModel::Unique,
        certs: false,
        manual_blocks: Vec::new(),
        partition: None,
        halt_on_violation: false,
        merge_policy: Default::default(),
        repeated_commit: Default::default(),
    }
}

fn phase(from_round: usize, arrivals_pmf: Vec<f64>) -> TrafficPhase {
    TrafficPhase {
        from_round,
        arrivals_pmf,
    }
}

#[test]
fn traffic_validation_rejects_bad_phases() {
    assert!(scenario(None).validate().is_ok());
    assert!(
        scenario(Some(vec![phase(1, vec![0.5, 0.5])]))
            .validate()
            .is_ok()
    );
    assert!(
        scenario(Some(vec![phase(1, vec![0.0, 1.0]), phase(9, vec![1.0])]))
            .validate()
            .is_ok(),
        "increasing phases with valid pmfs"
    );

    assert!(
        scenario(Some(vec![])).validate().is_err(),
        "empty phase list"
    );
    assert!(
        scenario(Some(vec![phase(0, vec![1.0])]))
            .validate()
            .is_err(),
        "phase rounds are 1-based"
    );
    assert!(
        scenario(Some(vec![phase(5, vec![1.0]), phase(5, vec![1.0])]))
            .validate()
            .is_err(),
        "from_round strictly increasing"
    );
    assert!(
        scenario(Some(vec![phase(9, vec![1.0]), phase(1, vec![1.0])]))
            .validate()
            .is_err(),
        "from_round out of order"
    );
    assert!(
        scenario(Some(vec![phase(1, vec![])])).validate().is_err(),
        "empty pmf"
    );
    assert!(
        scenario(Some(vec![phase(1, vec![1.0, f64::NAN])]))
            .validate()
            .is_err(),
        "NaN weight"
    );
    assert!(
        scenario(Some(vec![phase(1, vec![1.0, -0.1])]))
            .validate()
            .is_err(),
        "negative weight"
    );
    assert!(
        scenario(Some(vec![phase(1, vec![0.0, 0.0])]))
            .validate()
            .is_err(),
        "all-zero pmf"
    );
}

#[test]
fn point_mass_rate_one_spawns_one_arrival_per_round() {
    let report = run_smr(&scenario(Some(vec![phase(1, vec![0.0, 1.0])])));
    let autos: Vec<_> = report.commands.iter().filter(|c| c.auto).collect();
    assert_eq!(autos.len(), 20, "one arrival per round over the horizon");
    for (j, c) in autos.iter().enumerate() {
        assert_eq!(c.client, AUTO_CLIENT_BASE + j as u32);
        assert_eq!(c.op, AUTO_OP_BASE + j as u64);
        assert_eq!(c.injection_round, j + 1, "arrivals land in draw order");
    }
    assert!(
        report
            .commands
            .iter()
            .filter(|c| c.status == CommandStatus::Complete)
            .count()
            > 10,
        "healthy population completes most arrivals within the horizon"
    );
}

#[test]
fn pre_activation_rounds_are_stream_identical_to_no_traffic() {
    let with = run_smr(&scenario(Some(vec![phase(10, vec![0.0, 1.0])])));
    let without = run_smr(&scenario(None));
    assert_eq!(
        with.metrics[..9],
        without.metrics[..9],
        "no draw before the first phase activates"
    );
    assert!(
        with.commands.iter().all(|c| c.injection_round >= 10),
        "no arrivals before activation"
    );
}

#[test]
fn death_stops_spawning() {
    let mut s = scenario(Some(vec![phase(1, vec![0.0, 1.0])]));
    s.schedule = BlockSchedule::Permanent { fraction: 1.0 };
    let report = run_smr(&s);
    let SmrTerminal::Dead { round } = report.terminal else {
        panic!("fully blocked population must die");
    };
    assert!(
        report.commands.iter().all(|c| c.injection_round <= round),
        "no arrivals spawn after the population died"
    );
}

#[test]
fn zero_count_draw_still_shifts_the_stream() {
    let manual = |traffic| {
        let mut s = scenario(traffic);
        s.injections = vec![Injection {
            round: 2,
            client: 1,
            op: 7,
            target: None,
        }];
        run_smr(&s)
    };
    let zero_traffic = manual(Some(vec![phase(1, vec![1.0])]));
    let no_traffic = manual(None);
    assert!(
        zero_traffic.commands.iter().all(|c| !c.auto),
        "point-mass-0 pmf spawns nothing"
    );
    assert_ne!(
        zero_traffic.commands, no_traffic.commands,
        "the count-draw consumes from the stream even at count 0 — the manual \
         command's delivery draws land elsewhere"
    );
}

#[test]
fn phase_switch_changes_rate_at_the_boundary() {
    let report = run_smr(&scenario(Some(vec![
        phase(1, vec![0.0, 0.0, 1.0]),
        phase(9, vec![1.0]),
    ])));
    let arrivals_before: Vec<_> = report
        .commands
        .iter()
        .filter(|c| c.injection_round < 9)
        .collect();
    let arrivals_after: Vec<_> = report
        .commands
        .iter()
        .filter(|c| c.injection_round >= 9)
        .collect();
    assert_eq!(arrivals_before.len(), 16, "rate 2 over rounds 1-8");
    assert!(arrivals_after.is_empty(), "point-mass 0 from round 9 on");
}

#[test]
fn same_seed_replays_identically_with_traffic() {
    let s = scenario(Some(vec![
        phase(1, vec![0.2, 0.5, 0.3]),
        phase(11, vec![1.0]),
    ]));
    assert_eq!(run_smr(&s), run_smr(&s));
}

#[test]
fn metrics_report_realized_arrivals() {
    let report = run_smr(&scenario(Some(vec![phase(3, vec![0.0, 1.0])])));
    let realized: Vec<u32> = report.metrics.iter().map(|m| m.arrivals).collect();
    assert_eq!(&realized[..2], &[0, 0], "pre-activation rounds show zero");
    assert!(
        realized[2..].iter().all(|&a| a == 1),
        "point-mass rate 1 from round 3 on"
    );
}

#[test]
fn live_session_with_traffic_and_mid_run_inject_replays_as_batch() {
    let traffic = vec![phase(1, vec![0.0, 1.0])];
    let mut live = SmrState::new(
        16,
        Config::default(),
        Proto::Extended,
        1.0,
        7,
        &[],
        &traffic,
        ClientModel::Unique,
        None,
    );
    for _ in 0..5 {
        live.step_fraction(0.0);
    }
    let landing = live.inject(1, 99, None).unwrap();
    assert_eq!(landing, 6);
    for _ in 0..10 {
        live.step_fraction(0.0);
    }

    let mut s = scenario(Some(traffic));
    s.injections = vec![Injection {
        round: 6,
        client: 1,
        op: 99,
        target: None,
    }];
    s.max_rounds = 15;
    assert_eq!(
        live.report(),
        run_smr(&s),
        "registration order differs (live pushes the manual mid-run) but the \
         canonical report order must make both sides identical"
    );
}

fn live_state(traffic: &[TrafficPhase]) -> SmrState {
    SmrState::new(
        16,
        Config::default(),
        Proto::Extended,
        1.0,
        7,
        &[],
        traffic,
        ClientModel::Unique,
        None,
    )
}

#[test]
#[should_panic(expected = "draw_arrivals")]
fn mask_draw_before_arrivals_panics_under_traffic() {
    let mut state = live_state(&[phase(1, vec![0.0, 1.0])]);
    let mut sticky = vec![false; 16];
    state.adjust_sticky(&mut sticky, 2);
}

#[test]
fn mid_run_pmf_change_applies_next_round_and_replays_from_phases() {
    let initial = vec![phase(1, vec![0.0, 1.0])];
    let mut live = live_state(&initial);
    for _ in 0..5 {
        live.step_fraction(0.0);
    }
    live.set_traffic_pmf(vec![1.0]).unwrap();
    for _ in 0..5 {
        live.step_fraction(0.0);
    }
    assert_eq!(
        live.traffic_phases(),
        &[phase(1, vec![0.0, 1.0]), phase(6, vec![1.0])],
        "the edit lands as a phase from the next round"
    );
    assert!(
        live.report()
            .commands
            .iter()
            .filter(|c| c.auto)
            .all(|c| c.injection_round <= 5),
        "no arrivals after the off-phase"
    );

    let mut s = scenario(Some(live.traffic_phases().to_vec()));
    s.max_rounds = 10;
    assert_eq!(
        live.report(),
        run_smr(&s),
        "exported phases replay the mutated session byte-identically"
    );
}

#[test]
fn same_round_pmf_re_edit_replaces_pending_phase() {
    let mut live = live_state(&[phase(1, vec![0.0, 1.0])]);
    live.step_fraction(0.0);
    live.set_traffic_pmf(vec![1.0]).unwrap();
    live.set_traffic_pmf(vec![0.0, 0.0, 1.0]).unwrap();
    assert_eq!(
        live.traffic_phases().len(),
        2,
        "same-round re-edit replaces the pending phase"
    );
    live.step_fraction(0.0);
    assert_eq!(
        live.report().metrics[1].arrivals,
        2,
        "the last edit before the step wins"
    );
}

#[test]
fn live_edit_overrides_planned_future_phases() {
    let mut live = live_state(&[phase(1, vec![0.0, 1.0]), phase(8, vec![1.0])]);
    for _ in 0..3 {
        live.step_fraction(0.0);
    }
    live.set_traffic_pmf(vec![0.0, 0.0, 1.0]).unwrap();
    assert_eq!(
        live.traffic_phases(),
        &[phase(1, vec![0.0, 1.0]), phase(4, vec![0.0, 0.0, 1.0])],
        "an edit replaces every phase from its landing round on — the list \
         stays monotone and replayable"
    );
}

#[test]
fn bad_pmf_rejected_on_live_edit() {
    let mut live = live_state(&[phase(1, vec![1.0])]);
    assert!(live.set_traffic_pmf(vec![]).is_err());
    assert!(live.set_traffic_pmf(vec![-1.0]).is_err());
    assert!(live.set_traffic_pmf(vec![0.0]).is_err());
}

#[test]
fn manual_injections_exclude_autos() {
    let mut live = live_state(&[phase(1, vec![0.0, 1.0])]);
    for _ in 0..3 {
        live.step_fraction(0.0);
    }
    live.inject(1, 99, None).unwrap();
    live.step_fraction(0.0);
    assert_eq!(
        live.manual_injections(),
        vec![Injection {
            round: 4,
            client: 1,
            op: 99,
            target: None,
        }],
        "exported injections carry only manual commands"
    );
    assert!(
        live.injections().len() > 1,
        "the full list still includes traffic arrivals"
    );
}

#[test]
fn spread_recording_stops_one_round_past_completion() {
    // Sampling runs through one round past the completion landmark (extended:
    // prefix fixed, T_E; compact: committed ack) so the position band's
    // collapse is recorded; everything later is derivable and would cost
    // O(rate * rounds^2) memory under sustained load.
    let check = |proto: Proto, landmark: fn(&sim::smr::CommandReport) -> Option<usize>| {
        let mut s = scenario(Some(vec![phase(1, vec![0.0, 1.0])]));
        s.proto = proto;
        let report = run_smr(&s);
        let completed = report
            .commands
            .iter()
            .filter(|c| landmark(c).is_some_and(|r| r + 1 < s.max_rounds))
            .count();
        assert!(
            completed > 5,
            "test needs commands completed well before horizon"
        );
        for c in &report.commands {
            let Some(done) = landmark(c) else {
                continue;
            };
            let last = c.spread.points().last().expect("spread recorded").round;
            if done < s.max_rounds {
                assert_eq!(
                    last,
                    done + 1,
                    "op {}: spread must end exactly one round past completion",
                    c.op
                );
            }
        }
    };
    check(Proto::Extended, |c| c.prefix_fixed_round);
    check(Proto::Compact { t_commit_rounds: 8 }, |c| {
        c.committed_ack_round
    });
}

#[test]
fn manual_ids_in_auto_namespace_are_rejected() {
    let manual = |client: u32, op: u64| {
        let mut s = scenario(None);
        s.injections = vec![Injection {
            round: 1,
            client,
            op,
            target: None,
        }];
        s
    };
    assert!(
        manual(AUTO_CLIENT_BASE - 1, AUTO_OP_BASE - 1)
            .validate()
            .is_ok()
    );
    assert!(
        manual(AUTO_CLIENT_BASE, 7).validate().is_err(),
        "client namespace reserved for traffic arrivals"
    );
    assert!(
        manual(1, AUTO_OP_BASE).validate().is_err(),
        "op namespace reserved for traffic arrivals"
    );
}
