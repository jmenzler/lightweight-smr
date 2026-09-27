//! Collector goldens: byte-exact sim-schema CSV, the persistent-unanimity
//! convergence rule (incl. its sharpest edge: unanimity broken then
//! restored), message aggregation under the F6 accounting rule, gates.

use network_node::collect::{aggregate, aggregate_smr};
use network_node::record::{PostState, RoundRecord, TrackedDigest};
use network_node::spec::NodeRunSpec;

fn spec() -> NodeRunSpec {
    NodeRunSpec::from_json(
        r#"{"scenario":{"median":{"n":2,"seed":1,"k":6,"ell":3,
            "init":{"kind":"distinct"},"max_rounds":4}},"sync":"timer"}"#,
    )
    .expect("spec")
}

fn rec(r: u64, blocked: bool, snap_held: bool, post: Option<u64>) -> RoundRecord {
    RoundRecord {
        r,
        blocked,
        snap_held,
        post: PostState::Value(post),
        replies: 0,
        req_sent: if blocked { 0 } else { 6 },
        rep_sent: 0,
        rep_recv: 2,
        app_sent: 0,
        app_recv: 0,
        cli_recv: 0,
        acks: 0,
        bytes_out: 100,
        bytes_in: 40,
        late_rep: 0,
        late_app: 0,
        dropped_past: 0,
        encode_err: 0,
        exec_prefix: Vec::new(),
        t_last_reply_us: 0,
        t_step_done_us: 0,
        t_step_compute_us: 0,
        tracked: Vec::new(),
    }
}

#[test]
fn summary_aggregates_round_timing_tails() {
    // 2 nodes × 2 rounds; nearest-rank p99 over 4 samples is the max sample.
    let mut recs = vec![
        vec![rec(1, false, true, Some(7)), rec(2, false, true, Some(7))],
        vec![rec(1, false, true, Some(7)), rec(2, false, true, Some(7))],
    ];
    let last_reply = [10u64, 20, 30, 40];
    let step_done = [1u64, 2, 3, 100];
    let step_compute = [5u64, 6, 7, 50];
    let mut i = 0;
    for node in recs.iter_mut() {
        for r in node.iter_mut() {
            r.t_last_reply_us = last_reply[i];
            r.t_step_done_us = step_done[i];
            r.t_step_compute_us = step_compute[i];
            i += 1;
        }
    }
    let out = aggregate(&recs, &spec()).expect("aggregate");
    assert_eq!(out.summary.timing.t_last_reply_us_max, 40);
    assert_eq!(out.summary.timing.t_last_reply_us_p99, 40);
    assert_eq!(out.summary.timing.t_step_done_us_max, 100);
    assert_eq!(out.summary.timing.t_step_done_us_p99, 100);
    assert_eq!(out.summary.timing.t_step_compute_us_max, 50);
    assert_eq!(out.summary.timing.t_step_compute_us_p99, 50);
}

#[test]
fn metrics_csv_is_byte_exact_sim_schema() {
    // Node 0: holds 7 all four rounds, blocked in round 2.
    // Node 1: ⊥ in r1-r2, adopts 7 in r3.
    let records = vec![
        vec![
            rec(1, false, true, Some(7)),
            rec(2, true, true, None),
            rec(3, false, false, Some(7)),
            rec(4, false, true, Some(7)),
        ],
        vec![
            rec(1, false, true, None),
            rec(2, false, false, None),
            rec(3, false, false, Some(7)),
            rec(4, false, true, Some(7)),
        ],
    ];
    let out = aggregate(&records, &spec()).expect("aggregate");
    let want = "round,holders,undecided,blocked,useful,distinct_values\n\
                1,1,1,0,2,1\n\
                2,0,2,1,0,0\n\
                3,2,0,0,0,1\n\
                4,2,0,0,2,1\n";
    assert_eq!(out.metrics_csv, want);
    // unanimity holds from r1 (holders {7}), breaks at r2 (no holders),
    // returns at r3 and persists → convergence round 3.
    assert_eq!(out.summary.convergence_round, Some(3));
}

#[test]
fn never_unanimous_reports_none() {
    let records = vec![
        vec![rec(1, false, true, Some(1)), rec(2, false, true, Some(1))],
        vec![rec(1, false, true, Some(2)), rec(2, false, true, Some(2))],
    ];
    let out = aggregate(&records, &spec()).expect("aggregate");
    assert_eq!(out.summary.convergence_round, None);
}

#[test]
fn extinct_population_reports_none() {
    let records = vec![
        vec![rec(1, false, true, Some(1)), rec(2, false, false, None)],
        vec![rec(1, false, true, Some(1)), rec(2, false, false, None)],
    ];
    let out = aggregate(&records, &spec()).expect("aggregate");
    assert_eq!(
        out.summary.convergence_round, None,
        "all-bot horizon is extinction, not agreement"
    );
}

#[test]
fn messages_csv_sums_the_cluster_per_round() {
    let records = vec![
        vec![rec(1, false, true, Some(7))],
        vec![rec(1, true, true, None)],
    ];
    let out = aggregate(&records, &spec()).expect("aggregate");
    let want = "round,requests,replies,appends,client_msgs,acks,bytes_out,bytes_in\n\
                1,6,4,0,0,0,200,80\n";
    assert_eq!(
        out.messages_csv, want,
        "blocked node contributed 0 requests"
    );
}

#[test]
fn gates_surface_validity_violations_and_ragged_rounds() {
    let mut bad = rec(2, false, true, Some(1));
    bad.late_rep = 2;
    bad.dropped_past = 1;
    let records = vec![
        vec![rec(1, false, true, Some(1)), bad],
        vec![rec(1, false, true, Some(1))], // ragged: one round missing
    ];
    let out = aggregate(&records, &spec()).expect("aggregate");
    assert_eq!(out.gates.late_rep, 2);
    assert_eq!(out.gates.dropped_past, 1);
    assert!(!out.gates.rounds_consistent, "ragged per-node round counts");
    assert!(!out.gates.clean());
}

#[test]
fn empty_input_is_an_error_not_a_panic() {
    assert!(aggregate(&[], &spec()).is_err());
}

// --- SMR aggregation --------------------------------------------------------

fn smr_spec() -> NodeRunSpec {
    NodeRunSpec::from_json(
        r#"{"scenario":{"smr":{"n":2,"seed":1,"k":6,"ell":3,"sigma":1.0,
            "proto":{"kind":"compact","t_commit_rounds":3},"injections":[],
            "max_rounds":2}},"sync":"timer"}"#,
    )
    .expect("spec")
}

fn smr_rec(r: u64, log_len: Option<usize>, exec_len: usize, log_hash: Option<u64>) -> RoundRecord {
    let mut rec = rec(r, false, log_len.is_some(), None);
    rec.post = PostState::Log {
        log_len,
        exec_len,
        log_hash,
    };
    rec
}

#[test]
fn smr_metrics_csv_matches_the_ten_column_schema() {
    let records = vec![
        vec![
            smr_rec(1, Some(3), 1, Some(0xAA)),
            smr_rec(2, Some(4), 2, Some(0xBB)),
        ],
        vec![
            smr_rec(1, Some(3), 1, Some(0xAA)),
            smr_rec(2, None, 1, None), // went ⊥
        ],
    ];
    let out = aggregate_smr(&records, &smr_spec(), Some(&[2, 0])).expect("aggregate");
    let want = "round,nonbot_logs,blocked,useful,distinct_logs,max_log_len,min_executed_len,max_executed_len,arrivals,lcp_len\n\
                1,2,0,2,1,3,1,1,2,1\n\
                2,1,0,1,1,4,1,2,0,1\n";
    assert_eq!(out.metrics_csv, want);
}

#[test]
fn extended_leaves_lcp_empty_never_a_computed_zero() {
    let spec = NodeRunSpec::from_json(
        r#"{"scenario":{"smr":{"n":2,"seed":1,"k":6,"ell":3,"sigma":1.0,
            "proto":{"kind":"extended"},"injections":[],"max_rounds":1}},
            "sync":"timer"}"#,
    )
    .expect("spec");
    let records = vec![vec![smr_rec(1, Some(3), 0, Some(1))]];
    let out = aggregate_smr(&records, &spec, None).expect("aggregate");
    assert!(
        out.metrics_csv.ends_with(
            ",0,
"
        ),
        "extended lcp cell must be empty: {}",
        out.metrics_csv
    );
}

#[test]
fn smr_arrivals_default_to_zero_without_client_data() {
    let records = vec![vec![smr_rec(1, Some(1), 0, Some(1))]];
    let out = aggregate_smr(&records, &smr_spec(), None).expect("aggregate");
    assert!(
        out.metrics_csv.ends_with("1,1,0,1,1,1,0,0,0,0\n"),
        "{}",
        out.metrics_csv
    );
}

#[test]
fn encode_errors_fail_the_gates() {
    let mut bad = rec(1, false, true, Some(1));
    bad.encode_err = 1;
    let records = vec![vec![bad], vec![rec(1, false, true, Some(1))]];
    let out = aggregate(&records, &spec()).expect("aggregate");
    assert_eq!(out.gates.encode_err, 1);
    assert!(
        !out.gates.clean(),
        "encode faults are config errors, never silence"
    );
}

// --- strong safety + SMR landmarks ------------------------------------------

fn smr_rec_full(
    r: u64,
    log_len: Option<usize>,
    exec_prefix: Vec<(usize, u64)>,
    tracked: Vec<TrackedDigest>,
) -> RoundRecord {
    let mut rec = rec(r, false, log_len.is_some(), None);
    let exec_len = exec_prefix.last().map_or(0, |(l, _)| *l);
    rec.post = PostState::Log {
        log_len,
        exec_len,
        log_hash: Some(1),
    };
    rec.exec_prefix = exec_prefix;
    rec.tracked = tracked;
    rec
}

fn digest(op: u64, present: bool, pos: Option<usize>, prefix_hash: Option<u64>) -> TrackedDigest {
    TrackedDigest {
        op,
        present,
        pos,
        prefix_hash,
    }
}

#[test]
fn divergent_executed_prefixes_fail_the_safety_gate() {
    // Both nodes execute two entries; they agree at length 1 and differ at 2.
    let a = vec![smr_rec_full(1, Some(1), vec![(1, 0xA1), (2, 0xB1)], vec![])];
    let b = vec![smr_rec_full(1, Some(1), vec![(1, 0xA1), (2, 0xB2)], vec![])];
    let out = aggregate_smr(&[a, b], &smr_spec(), None).expect("aggregate");
    assert!(
        !out.gates.safety_ok,
        "split brain must latch the safety gate"
    );
    assert!(!out.gates.clean());
}

#[test]
fn agreeing_prefixes_of_different_lengths_stay_safe() {
    // Node b is one entry behind — a prefix, not a divergence.
    let a = vec![smr_rec_full(1, Some(1), vec![(1, 0xA1), (2, 0xB1)], vec![])];
    let b = vec![smr_rec_full(1, Some(1), vec![(1, 0xA1)], vec![])];
    let out = aggregate_smr(&[a, b], &smr_spec(), None).expect("aggregate");
    assert!(out.gates.safety_ok);
    assert!(out.gates.clean());
}

#[test]
fn landmarks_derive_from_tracked_digests() {
    // op 700: in every non-⊥ log from round 2 (T_B); positions/prefixes agree
    // only from round 3 (T_E).
    let a = vec![
        smr_rec_full(1, Some(1), vec![], vec![digest(700, false, None, None)]),
        smr_rec_full(
            2,
            Some(2),
            vec![],
            vec![digest(700, true, Some(1), Some(0xF1))],
        ),
        smr_rec_full(
            3,
            Some(2),
            vec![],
            vec![digest(700, true, Some(1), Some(0xF9))],
        ),
        smr_rec_full(
            4,
            Some(2),
            vec![],
            vec![digest(700, true, Some(1), Some(0xF9))],
        ),
    ];
    let b = vec![
        smr_rec_full(1, Some(1), vec![], vec![digest(700, false, None, None)]),
        smr_rec_full(
            2,
            Some(2),
            vec![],
            vec![digest(700, true, Some(2), Some(0xF2))],
        ),
        smr_rec_full(
            3,
            Some(2),
            vec![],
            vec![digest(700, true, Some(1), Some(0xF9))],
        ),
        smr_rec_full(
            4,
            Some(2),
            vec![],
            vec![digest(700, true, Some(1), Some(0xF9))],
        ),
    ];
    let out = aggregate_smr(&[a, b], &smr_spec(), None).expect("aggregate");
    let lm = out
        .landmarks
        .iter()
        .find(|l| l.op == 700)
        .expect("tracked op landmarks");
    assert_eq!(
        lm.all_logs_round,
        Some(2),
        "T_B: present in every non-⊥ log"
    );
    assert_eq!(
        lm.prefix_fixed_round,
        Some(3),
        "T_E: position + prefix agreed and never changed after"
    );
}

#[test]
fn landmarks_stay_none_when_agreement_never_settles() {
    let a = vec![
        smr_rec_full(
            1,
            Some(1),
            vec![],
            vec![digest(700, true, Some(1), Some(0xF1))],
        ),
        smr_rec_full(
            2,
            Some(1),
            vec![],
            vec![digest(700, true, Some(1), Some(0xF1))],
        ),
    ];
    let b = vec![
        smr_rec_full(
            1,
            Some(1),
            vec![],
            vec![digest(700, true, Some(2), Some(0xF2))],
        ),
        smr_rec_full(
            2,
            Some(1),
            vec![],
            vec![digest(700, true, Some(2), Some(0xF2))],
        ),
    ];
    let out = aggregate_smr(&[a, b], &smr_spec(), None).expect("aggregate");
    let lm = out
        .landmarks
        .iter()
        .find(|l| l.op == 700)
        .expect("landmarks");
    assert_eq!(lm.all_logs_round, Some(1));
    assert_eq!(lm.prefix_fixed_round, None, "never agreed on a position");
}

#[test]
fn transient_agreement_resets_the_te_candidate() {
    // Rounds 1-2 agree only because the divergent node sits ⊥; it rejoins in
    // round 3 with a different prefix, so the round-1 candidate must not
    // survive — T_E is the round-4 re-agreement.
    let a = vec![
        smr_rec_full(
            1,
            Some(1),
            vec![],
            vec![digest(700, true, Some(1), Some(0xF1))],
        ),
        smr_rec_full(
            2,
            Some(1),
            vec![],
            vec![digest(700, true, Some(1), Some(0xF1))],
        ),
        smr_rec_full(
            3,
            Some(1),
            vec![],
            vec![digest(700, true, Some(1), Some(0xF1))],
        ),
        smr_rec_full(
            4,
            Some(1),
            vec![],
            vec![digest(700, true, Some(1), Some(0xF1))],
        ),
    ];
    let b = vec![
        smr_rec_full(1, None, vec![], vec![]),
        smr_rec_full(2, None, vec![], vec![]),
        smr_rec_full(
            3,
            Some(1),
            vec![],
            vec![digest(700, true, Some(2), Some(0xF2))],
        ),
        smr_rec_full(
            4,
            Some(1),
            vec![],
            vec![digest(700, true, Some(1), Some(0xF1))],
        ),
    ];
    let out = aggregate_smr(&[a, b], &smr_spec(), None).expect("aggregate");
    let lm = out
        .landmarks
        .iter()
        .find(|l| l.op == 700)
        .expect("landmarks");
    assert_eq!(
        lm.prefix_fixed_round,
        Some(4),
        "T_E restarts after the transient rounds-1..2 agreement breaks"
    );
}
