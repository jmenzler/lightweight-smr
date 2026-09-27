//! The position census is computed for consumers that read it and scoped out
//! for the grid path, which reads none of it. Everything else the report
//! carries must be bit-identical across that choice — landmarks, the per-round
//! counters, the safety latch, and every column the grid row projects.
//!
//! The differential is `run_smr_lean` against `run_smr_grid`, not `run_smr`
//! against `run_smr_grid`: both lean runners shed settled curves, so pairing
//! them isolates `observe_spread` as the single difference. Comparing against
//! the full runner would conflate the scoping with the shed.
//!
//! All three push sites are covered, extended included — it is the one most
//! easily forgotten, because the two engines carrying the forget-line hazard
//! are the ones that get talked about.

use sim::smr::{
    ClientModel, Proto, SmrReport, SmrScenario, SpreadCurve, TrafficPhase, point_mass_pmf, run_smr,
    run_smr_grid, run_smr_lean,
};
use sim::{BlockSchedule, Config};

/// Loaded compact under blocking, the shape `smr_lean_report` pins the shed on:
/// commands settle every round, so both the census and the shed are live.
fn compact_cell(max_rounds: usize, rate: usize) -> SmrScenario {
    SmrScenario {
        n: 32,
        seed: 975_300,
        cfg: Config::default(),
        sigma: 1.0,
        proto: Proto::Compact {
            t_commit_rounds: 12,
        },
        injections: vec![],
        max_rounds,
        schedule: BlockSchedule::FreshPerRound { fraction: 0.1 },
        traffic: Some(vec![TrafficPhase {
            from_round: 1,
            arrivals_pmf: point_mass_pmf(rate),
        }]),
        client_model: ClientModel::Unique,
        certs: false,
        manual_blocks: Vec::new(),
        partition: None,
        halt_on_violation: false,
        merge_policy: Default::default(),
        repeated_commit: Default::default(),
    }
}

/// The recovery counterpart: this is the engine whose census owns the O(n)
/// position fold.
fn recovery_cell(seed: u64, rounds: usize, rate: usize) -> SmrScenario {
    SmrScenario {
        n: 32,
        seed,
        cfg: Config::default(),
        sigma: 1.0,
        proto: Proto::Recovery {
            t_window_rounds: 20,
            resend_until_acked: false,
            prefix_mismatch: sim::smr::PrefixMismatch::Abort,
        },
        injections: vec![],
        max_rounds: rounds,
        schedule: BlockSchedule::FreshPerRound { fraction: 0.0 },
        traffic: Some(vec![TrafficPhase {
            from_round: 1,
            arrivals_pmf: point_mass_pmf(rate),
        }]),
        client_model: ClientModel::Unique,
        certs: false,
        manual_blocks: Vec::new(),
        partition: None,
        halt_on_violation: false,
        merge_policy: Default::default(),
        repeated_commit: Default::default(),
    }
}

/// Everything that is NOT the census. `metrics` carries `distinct_logs` and
/// `useful` — both are reported values the scoping may not move — and the
/// landmark tuple is the whole projection `smr_row` takes.
fn assert_everything_but_the_census(on: &SmrReport, off: &SmrReport, label: &str) {
    assert_eq!(on.terminal, off.terminal, "{label}: terminal");
    assert_eq!(on.safety_ok, off.safety_ok, "{label}: safety latch");
    assert_eq!(
        on.metrics, off.metrics,
        "{label}: a per-round counter moved — distinct_logs and useful live here"
    );
    assert_eq!(
        on.pool_peak_in_flight, off.pool_peak_in_flight,
        "{label}: pool occupancy moved — the scoping reached the client stage"
    );
    match (&on.recovery, &off.recovery) {
        (Some(a), Some(b)) => {
            assert_eq!(a.fork_ok, b.fork_ok, "{label}: the lineage oracle moved");
            assert_eq!(a.rounds, b.rounds, "{label}: a recovery counter row moved");
        }
        (None, None) => {}
        _ => panic!("{label}: one side produced a recovery report and the other did not"),
    }
    assert_eq!(on.commands.len(), off.commands.len(), "{label}: commands");
    for (a, b) in on.commands.iter().zip(&off.commands) {
        assert_eq!(
            (
                a.client,
                a.op,
                a.status,
                a.injection_round,
                a.delivered_round,
                a.all_logs_round,
                a.prefix_fixed_round,
                a.committed_ack_round,
                a.executed_round,
            ),
            (
                b.client,
                b.op,
                b.status,
                b.injection_round,
                b.delivered_round,
                b.all_logs_round,
                b.prefix_fixed_round,
                b.committed_ack_round,
                b.executed_round,
            ),
            "{label}: a landmark moved on op {}",
            a.op
        );
        assert_eq!(
            a.attempts, b.attempts,
            "{label}: attempts moved on op {}",
            a.op
        );
        assert_eq!(a.amp_receivers, b.amp_receivers, "{label}: receivers moved");
    }
}

/// A declined measurement and a real zero must never render the same: the OFF
/// side reports `NotObserved`, never an empty `Observed`.
fn assert_scoping_is_legible(on: &SmrReport, off: &SmrReport, label: &str) {
    for c in &off.commands {
        assert!(
            matches!(c.spread, SpreadCurve::NotObserved),
            "{label}: op {} reported an Observed curve on the scoped-out path — \
             a declined measurement is rendering as data",
            c.op
        );
    }
    assert!(
        on.commands
            .iter()
            .any(|c| matches!(c.spread, SpreadCurve::Observed(_))),
        "{label}: the census-ON side observed nothing, so the pairing pins nothing"
    );
}

#[test]
fn scoping_the_census_moves_nothing_the_grid_reads_on_compact() {
    let s = compact_cell(200, 4);
    let on = run_smr_lean(&s);
    let off = run_smr_grid(&s);
    assert_everything_but_the_census(&on, &off, "compact");
    assert_scoping_is_legible(&on, &off, "compact");
}

#[test]
fn scoping_the_census_is_inert_after_the_compact_safety_latch_falls() {
    let s = SmrScenario {
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
            arrivals_pmf: point_mass_pmf(4),
        }]),
        client_model: ClientModel::Unique,
        certs: false,
        manual_blocks: Vec::new(),
        partition: None,
        halt_on_violation: false,
        merge_policy: Default::default(),
        repeated_commit: Default::default(),
    };
    let on = run_smr_lean(&s);
    let off = run_smr_grid(&s);
    assert!(!on.safety_ok, "the exact split-brain seed must lose safety");
    assert_everything_but_the_census(&on, &off, "compact after safety loss");
    assert_scoping_is_legible(&on, &off, "compact after safety loss");
}

#[test]
fn scoping_the_census_moves_nothing_the_grid_reads_on_recovery() {
    for (seed, rounds, rate) in [(4_340_000, 200, 4), (4_340_020, 400, 4)] {
        let s = recovery_cell(seed, rounds, rate);
        let on = run_smr_lean(&s);
        let off = run_smr_grid(&s);
        let label = format!("recovery seed {seed}");
        assert!(
            on.commands.iter().any(|c| c.executed_round.is_some()),
            "{label}: nothing committed — the cell would pin its own precondition"
        );
        assert_everything_but_the_census(&on, &off, &label);
        assert_scoping_is_legible(&on, &off, &label);
    }
}

/// The extended engine's push site sits inside `CommandTracker::observe`
/// alongside the landmark block. It is the site most easily forgotten,
/// because the two engines carrying the forget-line hazard are the ones that
/// get the attention.
#[test]
fn scoping_the_census_moves_nothing_the_grid_reads_on_extended() {
    let mut s = compact_cell(200, 4);
    s.proto = Proto::Extended;
    let on = run_smr_lean(&s);
    let off = run_smr_grid(&s);
    assert_everything_but_the_census(&on, &off, "extended");
    assert_scoping_is_legible(&on, &off, "extended");
}

/// The fail-loud contract is a tested contract, not a comment. `points()` on a
/// curve that was never computed must panic naming the fix, rather than
/// answering plausibly with an empty slice.
#[test]
#[should_panic(expected = "scoped out")]
fn reading_points_on_an_unobserved_curve_panics() {
    let off = run_smr_grid(&compact_cell(120, 4));
    let _ = off.commands[0].spread.points();
}

/// `observed()` is the branch-shaped accessor: the callers that genuinely
/// choose get an Option instead of a panic.
#[test]
fn observed_returns_none_on_the_scoped_out_path_and_some_on_the_full_one() {
    let s = compact_cell(120, 4);
    assert!(run_smr_grid(&s).commands[0].spread.observed().is_none());
    assert!(run_smr(&s).commands[0].spread.observed().is_some());
}
