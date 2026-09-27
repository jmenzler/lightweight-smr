use std::path::PathBuf;
use std::process::Command;

const HEALTHY_SPEC: &str = r#"{
    "n": 8, "seed": 1, "k": 6, "ell": 3, "sigma": 1.0,
    "proto": {"kind": "extended"},
    "injections": [],
    "max_rounds": 3
}"#;

const FAILING_SPEC: &str = r#"{
    "n": 598, "seed": 1, "k": 6, "ell": 3, "sigma": 1.0,
    "proto": {"kind": "recovery", "t_window_rounds": 20, "resend_until_acked": true},
    "injections": [
        {"round": 2, "client": 1, "op": 1},
        {"round": 2, "client": 2, "op": 2}
    ],
    "max_rounds": 200,
    "schedule": {"kind": "fresh_per_round", "fraction": 0.1},
    "traffic": {"kind": "pmf", "arrivals_pmf": [0.0, 0.0, 1.0]},
    "certs": true
}"#;

fn temp_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("smr-metrics-{name}-{}", std::process::id()))
}

fn write_spec(name: &str, json: &str) -> PathBuf {
    let path = temp_path(name);
    std::fs::write(&path, json).expect("write fixture");
    path
}

fn assert_failure_diagnostic(output: &std::process::Output) {
    assert!(!output.status.success(), "failed run must exit nonzero");
    assert!(output.stdout.is_empty(), "failed run emitted ordinary CSV");
    let stderr = String::from_utf8_lossy(&output.stderr);
    for expected in [
        "SMR failed during BoundaryPreflight",
        "attempted round",
        "completed round",
        "observed round",
    ] {
        assert!(stderr.contains(expected), "missing {expected}: {stderr}");
    }
}

#[test]
fn healthy_run_keeps_the_existing_csv_shape() {
    let spec = write_spec("healthy.json", HEALTHY_SPEC);
    let output = Command::new(env!("CARGO_BIN_EXE_smr_metrics"))
        .arg(&spec)
        .output()
        .expect("run smr_metrics");
    std::fs::remove_file(spec).ok();
    assert!(output.status.success(), "healthy run failed");
    let stdout = String::from_utf8(output.stdout).expect("utf8 csv");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines[0],
        "round,nonbot_logs,blocked,useful,distinct_logs,max_log_len,min_executed_len,max_executed_len,arrivals,lcp_len"
    );
    assert_eq!(lines.len(), 4, "header plus three healthy rows");
}

#[test]
fn failed_run_emits_no_stdout_csv() {
    let spec = write_spec("stdout.json", FAILING_SPEC);
    let output = Command::new(env!("CARGO_BIN_EXE_smr_metrics"))
        .arg(&spec)
        .output()
        .expect("run smr_metrics");
    std::fs::remove_file(spec).ok();
    assert_failure_diagnostic(&output);
}

#[test]
fn failed_run_does_not_overwrite_existing_output() {
    let spec = write_spec("file.json", FAILING_SPEC);
    let output_path = temp_path("existing.csv");
    let sentinel = "existing scientific output\n";
    std::fs::write(&output_path, sentinel).expect("write sentinel");

    let output = Command::new(env!("CARGO_BIN_EXE_smr_metrics"))
        .args([spec.as_os_str(), output_path.as_os_str()])
        .output()
        .expect("run smr_metrics");

    assert_failure_diagnostic(&output);
    assert_eq!(
        std::fs::read_to_string(&output_path).expect("read existing output"),
        sentinel
    );
    std::fs::remove_file(spec).ok();
    std::fs::remove_file(output_path).ok();
}
