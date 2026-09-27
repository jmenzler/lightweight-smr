use sim::smr::{ClientModel, Injection, Proto, SmrScenario, run_smr_recorded};
use sim::{BlockSchedule, Config};

fn scenario() -> SmrScenario {
    SmrScenario {
        n: 32,
        seed: 5,
        cfg: Config::default(),
        sigma: 5.0,
        proto: Proto::Extended,
        injections: vec![Injection {
            round: 2,
            client: 1,
            op: 7,
            target: None,
        }],
        max_rounds: 20,
        schedule: BlockSchedule::FreshPerRound { fraction: 0.0 },
        traffic: None,
        client_model: ClientModel::Unique,
        certs: false,
        manual_blocks: Vec::new(),
        partition: None,
        halt_on_violation: false,
        merge_policy: Default::default(),
        repeated_commit: Default::default(),
    }
}

#[test]
fn metrics_csv_rows_align_with_header_and_report() {
    let dir = std::env::temp_dir().join(format!("smr-runlog-metrics-{}", std::process::id()));
    let ledger = dir.join("ledger.jsonl");
    let scenario = scenario();
    let report = sim::smr::run_smr(&scenario);
    let (metrics_path, _) = sim::runlog::record_smr_report(&scenario, &report, &ledger, None);

    let csv = std::fs::read_to_string(&metrics_path).unwrap();
    let mut lines = csv.lines();
    let header_fields = lines.next().unwrap().split(',').count();
    let rows: Vec<&str> = lines.collect();
    assert_eq!(rows.len(), report.metrics.len());
    for row in &rows {
        assert_eq!(row.split(',').count(), header_fields);
    }
    // spot-check round 3 (row index 2): arrivals and lcp_len are the last two columns
    let cols: Vec<&str> = rows[2].split(',').collect();
    assert_eq!(cols[0], "3");
    assert_eq!(cols[8], report.metrics[2].arrivals.to_string());
    assert_eq!(cols[9], report.metrics[2].lcp_len.to_string());

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn ledger_record_for_precomputed_report_has_no_side_files() {
    let dir = std::env::temp_dir().join(format!("smr-runlog-direct-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let ledger = dir.join("ledger.jsonl");
    let scenario = scenario();
    let report = sim::smr::run_smr(&scenario);
    sim::runlog::log_smr_run_to(&ledger, &scenario, &report, None).unwrap();

    let line = std::fs::read_to_string(&ledger).unwrap();
    let record: serde_json::Value = serde_json::from_str(line.lines().next().unwrap()).unwrap();
    assert_eq!(record["amp_count"], 25, "ceil(5·log2(32))");
    assert_eq!(record["outcome"]["terminal"]["Ran"]["rounds"], 20);
    assert!(record["outcome"]["safety_ok"].as_bool().unwrap());
    assert_eq!(record["outcome"]["commands"], 1);
    assert_eq!(record["trace_path"], serde_json::Value::Null);
    assert!(
        record.get("metrics_path").is_none(),
        "no metrics side file for a precomputed report"
    );
    assert!(record.get("commands_path").is_none());

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn smr_record_carries_provenance_and_returns_paths() {
    let dir = std::env::temp_dir().join(format!("smr-runlog-{}", std::process::id()));
    let ledger = dir.join("ledger.jsonl");
    let (report, metrics_path, commands_path) = run_smr_recorded(&scenario(), &ledger, None);
    assert_eq!(report.metrics.len(), 20);

    let line = std::fs::read_to_string(&ledger).unwrap();
    let record: serde_json::Value =
        serde_json::from_str(line.lines().next_back().unwrap()).unwrap();
    assert!(record["commit"].is_string());
    assert!(record["dirty"].is_boolean());
    assert_eq!(record["scenario"]["proto"], "Extended");
    assert_eq!(record["amp_count"], 25, "ceil(5·log2(32)) recorded");
    assert_eq!(record["trace_path"], serde_json::Value::Null);
    assert_eq!(record["metrics_path"], metrics_path);
    assert_eq!(record["commands_path"], commands_path);
    assert!(record["outcome"]["safety_ok"].as_bool().unwrap());

    assert_eq!(
        record["scenario"].get("traffic"),
        None,
        "traffic-free scenarios serialize without the field"
    );

    let metrics = std::fs::read_to_string(&metrics_path).unwrap();
    let mut lines = metrics.lines();
    assert_eq!(
        lines.next().unwrap(),
        "round,nonbot_logs,blocked,useful,distinct_logs,max_log_len,min_executed_len,max_executed_len,arrivals,lcp_len"
    );
    assert_eq!(lines.count(), 20, "one row per simulated round");

    let commands: Vec<serde_json::Value> =
        serde_json::from_str(&std::fs::read_to_string(&commands_path).unwrap()).unwrap();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0]["op"], 7);
    assert!(
        commands[0]["spread"].is_array(),
        "spread series persisted (recorded through coverage + 1)"
    );

    std::fs::remove_dir_all(&dir).ok();
}
