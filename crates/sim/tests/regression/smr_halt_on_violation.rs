use sim::smr::{Proto, SmrScenario, SmrTerminal, TrafficPhase, run_smr_lean};
use sim::{BlockSchedule, Config};

fn split_brain_scenario(halt_on_violation: bool) -> SmrScenario {
    SmrScenario {
        n: 64,
        seed: 950_000,
        cfg: Config::default(),
        sigma: 1.0,
        proto: Proto::Compact {
            t_commit_rounds: 21,
        },
        injections: vec![],
        max_rounds: 400,
        schedule: BlockSchedule::FreshPerRound { fraction: 0.0 },
        traffic: Some(vec![TrafficPhase {
            from_round: 1,
            arrivals_pmf: vec![0.0, 0.0, 0.0, 0.0, 1.0],
        }]),
        client_model: sim::smr::ClientModel::Unique,
        certs: false,
        manual_blocks: Vec::new(),
        partition: None,
        halt_on_violation,
        merge_policy: Default::default(),
        repeated_commit: Default::default(),
    }
}

#[test]
fn a_halting_run_ends_at_the_violation_with_the_same_latch() {
    let full = run_smr_lean(&split_brain_scenario(false));
    assert!(!full.safety_ok, "the cell must split its brain");
    let SmrTerminal::Ran {
        rounds: full_rounds,
    } = full.terminal
    else {
        panic!("full run must reach its horizon");
    };
    assert_eq!(full_rounds, 400);

    let halted = run_smr_lean(&split_brain_scenario(true));
    assert!(!halted.safety_ok, "halting must not change the latch");
    let SmrTerminal::Ran {
        rounds: halted_rounds,
    } = halted.terminal
    else {
        panic!("a halted run still terminates as Ran");
    };
    assert!(
        halted_rounds < full_rounds,
        "halting run stopped at {halted_rounds}, expected before {full_rounds}"
    );
}

#[test]
fn a_clean_run_is_byte_identical_with_the_flag_on() {
    let mut clean = split_brain_scenario(false);
    clean.proto = Proto::Compact {
        t_commit_rounds: 60,
    };
    let mut clean_halting = clean.clone();
    clean_halting.halt_on_violation = true;

    let a = run_smr_lean(&clean);
    let b = run_smr_lean(&clean_halting);
    assert!(a.safety_ok, "the tall-threshold cell must stay safe");
    assert_eq!(
        serde_json::to_string(&a).unwrap(),
        serde_json::to_string(&b).unwrap(),
        "the flag may not move a single byte of a clean run's report"
    );
}

#[test]
fn the_flag_is_absent_from_the_wire_while_off() {
    let s = split_brain_scenario(false);
    let json = serde_json::to_string(&s).unwrap();
    assert!(
        !json.contains("halt_on_violation"),
        "an off flag must not change existing spec bytes or ledger rows"
    );
    let with = serde_json::to_string(&split_brain_scenario(true)).unwrap();
    assert!(with.contains("halt_on_violation"));
}
