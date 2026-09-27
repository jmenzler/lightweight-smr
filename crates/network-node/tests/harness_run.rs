//! `bin/harness_run.rs` goldens: its written files must equal direct
//! `aggregate()` output for the same spec — the perfect-network CLI driver
//! and the library aggregation path it wraps must never drift.

use network_node::collect::{aggregate, summary_json};
use network_node::spec::NodeRunSpec;
use std::process::Command;

fn median_spec_json() -> &'static str {
    r#"{"scenario":{"median":{"n":4,"seed":7,"k":6,"ell":3,
        "init":{"kind":"distinct"},"max_rounds":20}},"sync":"timer"}"#
}

#[test]
fn harness_run_output_matches_direct_aggregate() {
    let dir = std::env::temp_dir().join(format!("harness-run-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let spec_path = dir.join("spec.json");
    std::fs::write(&spec_path, median_spec_json()).unwrap();
    let out_dir = dir.join("out");

    let status = Command::new(env!("CARGO_BIN_EXE_harness_run"))
        .args([
            "--spec",
            spec_path.to_str().unwrap(),
            "--out",
            out_dir.to_str().unwrap(),
        ])
        .status()
        .expect("spawn harness_run");
    assert!(status.success(), "harness_run exited nonzero");

    let spec = NodeRunSpec::from_json(median_spec_json()).expect("spec");
    let records = network_node::harness::run(&spec);
    let want = aggregate(&records, &spec).expect("aggregate");

    let got_metrics = std::fs::read_to_string(out_dir.join("metrics.csv")).unwrap();
    let got_messages = std::fs::read_to_string(out_dir.join("messages.csv")).unwrap();
    let got_summary = std::fs::read_to_string(out_dir.join("summary.json")).unwrap();

    assert_eq!(
        got_metrics, want.metrics_csv,
        "metrics.csv must match aggregate()"
    );
    assert_eq!(
        got_messages, want.messages_csv,
        "messages.csv must match aggregate()"
    );
    assert_eq!(
        got_summary,
        summary_json(&want),
        "summary.json must match aggregate()"
    );
}

#[test]
fn harness_run_dies_on_smr_specs() {
    let dir = std::env::temp_dir().join(format!("harness-run-smr-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let spec_json = r#"{"scenario":{"smr":{"n":4,"seed":1,"k":6,"ell":3,"sigma":1.0,
        "proto":{"kind":"extended"},"injections":[],"max_rounds":10,
        "schedule":{"kind":"fresh_per_round","fraction":0.0}}},"sync":"timer"}"#;
    let spec_path = dir.join("spec.json");
    std::fs::write(&spec_path, spec_json).unwrap();
    let out_dir = dir.join("out");

    let output = Command::new(env!("CARGO_BIN_EXE_harness_run"))
        .args([
            "--spec",
            spec_path.to_str().unwrap(),
            "--out",
            out_dir.to_str().unwrap(),
        ])
        .output()
        .expect("spawn harness_run");
    assert!(!output.status.success(), "SMR spec must die, not run");
}
