//! The memory accounting is a research instrument pointed at the runs the
//! grid actually makes, so two things have to hold: observing a run must not
//! change it, and the walk must report the structure the attribution rests on.

use sim::smr::{
    ClientModel, Proto, SmrScenario, SmrState, TrafficPhase, point_mass_pmf, run_smr_lean,
    run_smr_lean_observed,
};
use sim::{BlockSchedule, Config};

/// Loaded recovery with checkpoints turning over several times, so the
/// per-client certificate windows are populated rather than empty. Explicitly
/// cert-ON: the attribution this file records is a statement about the §5
/// layer, and a gated-off run has no windows to attribute anything to.
fn cell() -> SmrScenario {
    SmrScenario {
        n: 32,
        seed: 975_300,
        cfg: Config::default(),
        sigma: 1.0,
        proto: Proto::Recovery {
            t_window_rounds: 20,
            resend_until_acked: false,
            prefix_mismatch: sim::smr::PrefixMismatch::Abort,
        },
        injections: vec![],
        max_rounds: 120,
        schedule: BlockSchedule::FreshPerRound { fraction: 0.1 },
        traffic: Some(vec![TrafficPhase {
            from_round: 1,
            arrivals_pmf: point_mass_pmf(1),
        }]),
        client_model: ClientModel::Unique,
        certs: true,
        manual_blocks: Vec::new(),
        partition: None,
        halt_on_violation: false,
        merge_policy: Default::default(),
        repeated_commit: Default::default(),
    }
}

/// The instrument's licence: it reads, so the run it watches is the run the
/// grid made. A sampler that drew from the stream — or that mutated anything
/// on the way past — would land here as a differing report.
#[test]
fn walking_the_state_every_round_leaves_the_lean_run_byte_identical() {
    let s = cell();
    let mut plain = run_smr_lean(&s);

    let mut walks = 0usize;
    let mut observed = run_smr_lean_observed(&s, &mut |state: &SmrState| {
        assert!(
            state.memory_accounting().is_some(),
            "recovery nodes must be walkable"
        );
        walks += 1;
    });

    assert_eq!(walks, s.max_rounds, "the observer sees every round");
    // Provenance, not observation: each run names its own spill file.
    plain.spill_path = None;
    observed.spill_path = None;
    assert_eq!(plain, observed, "observing the run changed the run");
}

/// What the attribution rests on, stated as a contract rather than as a
/// reading of one run.
///
/// Deliberately a PER-CLASS statement, not a global-dominance one. Global
/// dominance was true only while every node minted its own checkpoint: once
/// content-equal checkpoints share one allocation, the cert term is divided by
/// the sharing factor while the per-node categories are not, so a global
/// comparison would measure the sharing rather than the §5 storage law. What
/// survives that change — and is what the law actually says — is the shape of
/// ONE checkpoint's window map: bounded by the last-two-per-client rule (which
/// is why cutting the client count alone need not cut the footprint), and
/// costing a Merkle chain plus a map entry per stored command, which is what
/// makes it the largest thing a checkpoint carries.
#[test]
fn one_checkpoints_cert_windows_hold_at_most_two_commands_per_client() {
    let s = cell();
    let mut last = None;
    run_smr_lean_observed(&s, &mut |state: &SmrState| {
        last = state.memory_accounting();
    });
    let acc = last.expect("a recovery run reports an accounting");

    assert!(
        acc.cert_stored_commands > 0,
        "the run must commit enough to populate a window: {acc:?}"
    );
    assert!(
        acc.cert_stored_commands <= 2 * acc.distinct_clients,
        "{} stored commands over {} clients breaks the last-two rule",
        acc.cert_stored_commands,
        acc.distinct_clients
    );

    let bytes = |key: &str| acc.rows.get(key).copied().unwrap_or(0);
    let windows = bytes("checkpoint.certs.windows");
    let carried = bytes("checkpoint.state.executed") + bytes("checkpoint.pre");
    assert!(
        windows > carried,
        "the §5 windows hold {windows} B against {carried} B of carried state and P — \
         no longer the dominant thing a checkpoint carries: {acc:?}"
    );

    // Per stored command, over the checkpoints that were really allocated: at
    // least one hash of Merkle chain plus its share of the map entry.
    let per_stored = windows / (acc.cert_stored_commands * acc.distinct_checkpoints) as u64;
    assert!(
        per_stored >= 32,
        "{per_stored} B per stored command is below one hash: {acc:?}"
    );
}
