//! Append-only JSONL run ledger: commit + params + outcome per run, so every number is regenerable.

use crate::smr::{RecoveryFailure, SmrReport, SmrScenario, SmrTerminal, amp_count, run_smr};
use crate::{Outcome, Scenario};
use serde::Serialize;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const VALUE_METRICS_CSV_HEADER: &str = "round,holders,undecided,blocked,useful,distinct_values";
pub const SMR_METRICS_CSV_HEADER: &str = "round,nonbot_logs,blocked,useful,distinct_logs,max_log_len,min_executed_len,max_executed_len,arrivals,lcp_len";

pub const DEFAULT_LOG: &str = ".runs/sim-runs.jsonl";

#[derive(Serialize)]
struct RunRecord<'a> {
    timestamp: String,
    commit: String,
    dirty: bool,
    scenario: &'a Scenario,
    outcome: &'a Outcome,
    trace_path: Option<String>,
    metrics_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    extra: Option<serde_json::Value>,
}

pub const DEFAULT_TRACE_DIR: &str = ".runs/traces";
pub const DEFAULT_TRACE_CAP_CELLS: u64 = 20_000_000;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub(crate) fn utc_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock before epoch")
        .as_secs();
    let (days, rem) = (secs / 86_400, secs % 86_400);
    // civil-from-days (Hinnant): valid for the unix era
    let z = days as i64 + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

pub fn log_run_to(
    path: &Path,
    scenario: &Scenario,
    outcome: &Outcome,
    extra: Option<serde_json::Value>,
) -> std::io::Result<()> {
    log_run_record(path, scenario, outcome, None, None, extra)
}

pub(crate) fn provenance() -> (String, String, bool) {
    (
        utc_now(),
        git(&["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".into()),
        git(&["status", "--porcelain"]).is_none_or(|s| !s.is_empty()),
    )
}

fn log_run_record(
    path: &Path,
    scenario: &Scenario,
    outcome: &Outcome,
    trace_path: Option<String>,
    metrics_path: Option<String>,
    extra: Option<serde_json::Value>,
) -> std::io::Result<()> {
    let (timestamp, commit, dirty) = provenance();
    let record = RunRecord {
        timestamp,
        commit,
        dirty,
        scenario,
        outcome,
        trace_path,
        metrics_path,
        extra,
    };
    append_jsonl(path, &record)
}

fn append_jsonl<S: Serialize>(path: &Path, record: &S) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(
        f,
        "{}",
        serde_json::to_string(record).expect("record serializes")
    )?;
    Ok(())
}

pub fn default_log_path() -> PathBuf {
    git(&["rev-parse", "--show-toplevel"])
        .map(|top| Path::new(&top).join(DEFAULT_LOG))
        .unwrap_or_else(|| PathBuf::from(DEFAULT_LOG))
}

/// Ledger path: an explicit path always logs; the repo-root default honors SIM_RUNLOG=off.
pub fn ledger_target(explicit: Option<&Path>) -> Option<PathBuf> {
    match explicit {
        Some(p) => Some(p.to_path_buf()),
        None if std::env::var("SIM_RUNLOG").as_deref() == Ok("off") => None,
        None => Some(default_log_path()),
    }
}

/// `run()` + ledger append, returning the metrics CSV path; a ledger write failure panics, since an unlogged run is unauditable.
pub fn run_logged(
    scenario: &Scenario,
    extra: Option<serde_json::Value>,
) -> (Outcome, Option<String>) {
    let Some(ledger) = ledger_target(None) else {
        return (crate::run(scenario), None);
    };
    let cap = if std::env::var("SIM_TRACE").as_deref() == Ok("off") {
        0
    } else {
        std::env::var("SIM_TRACE_MAX_CELLS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_TRACE_CAP_CELLS)
    };
    let trace_dir = ledger.parent().unwrap_or(Path::new(".")).join("traces");
    let (outcome, metrics_path) = run_recorded(scenario, &ledger, &trace_dir, cap, extra);
    (outcome, Some(metrics_path))
}

pub fn run_recorded(
    scenario: &Scenario,
    ledger: &Path,
    trace_dir: &Path,
    cap_cells: u64,
    extra: Option<serde_json::Value>,
) -> (Outcome, String) {
    let stamp = utc_now().replace(':', "");
    let cells = scenario.n as u64 * scenario.max_rounds as u64;
    let record_trace = cells <= cap_cells && cap_cells > 0;
    let (outcome, metrics, trace) = crate::run_inner(scenario, record_trace);

    let trace_path = trace.map(|t| {
        let file = trace_dir.join(format!(
            "{stamp}-n{}-seed{}.json",
            scenario.n, scenario.seed
        ));
        std::fs::create_dir_all(trace_dir).expect("trace dir");
        std::fs::write(&file, serde_json::to_string(&t).expect("trace serializes"))
            .expect("trace write failed (set SIM_TRACE=off to opt out)");
        file.to_string_lossy().into_owned()
    });

    let mut csv = format!("{VALUE_METRICS_CSV_HEADER}\n");
    for (i, m) in metrics.iter().enumerate() {
        csv.push_str(&format!(
            "{},{},{},{},{},{}\n",
            i + 1,
            m.holders,
            m.undecided,
            m.blocked,
            m.useful,
            m.distinct_values
        ));
    }
    let metrics_dir = ledger.parent().unwrap_or(Path::new(".")).join("metrics");
    let metrics_file =
        metrics_dir.join(format!("{stamp}-n{}-seed{}.csv", scenario.n, scenario.seed));
    std::fs::create_dir_all(&metrics_dir).expect("metrics dir");
    std::fs::write(&metrics_file, csv).expect("metrics write failed");
    let metrics_path = metrics_file.to_string_lossy().into_owned();

    log_run_record(
        ledger,
        scenario,
        &outcome,
        trace_path,
        Some(metrics_path.clone()),
        extra,
    )
    .expect("run ledger write failed (set SIM_RUNLOG=off to opt out)");
    (outcome, metrics_path)
}

#[derive(Serialize)]
struct SmrOutcomeSummary {
    terminal: SmrTerminal,
    safety_ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    exec_dup_round: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    repeat_skips: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    first_repeat_skip_round: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    boundary_skip_events: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    first_boundary_skip_round: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_boundary_skip_round: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_rejoin_rounds: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    unrejoined_nodes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    failure: Option<RecoveryFailure>,
    commands: usize,
    complete: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    recovery_fork_ok: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pool_peak_in_flight: Option<u32>,
}

#[derive(Serialize)]
struct SmrRunRecord<'a, O> {
    timestamp: String,
    commit: String,
    dirty: bool,
    scenario: &'a SmrScenario,
    amp_count: usize,
    outcome: O,
    trace_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metrics_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    commands_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    recovery_path: Option<String>,
    // Read off the report so the ledger cannot claim a spill that was not written.
    #[serde(skip_serializing_if = "Option::is_none")]
    spill_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    extra: Option<serde_json::Value>,
}

fn smr_outcome_summary(report: &SmrReport) -> SmrOutcomeSummary {
    SmrOutcomeSummary {
        terminal: report.terminal,
        safety_ok: report.safety_ok,
        exec_dup_round: report.exec_dup_round,
        repeat_skips: (report.repeat_skips > 0).then_some(report.repeat_skips),
        first_repeat_skip_round: report.first_repeat_skip_round,
        boundary_skip_events: (report.boundary_skip_events > 0)
            .then_some(report.boundary_skip_events),
        first_boundary_skip_round: report.first_boundary_skip_round,
        last_boundary_skip_round: report.last_boundary_skip_round,
        max_rejoin_rounds: report.max_rejoin_rounds,
        unrejoined_nodes: (report.unrejoined_nodes > 0).then_some(report.unrejoined_nodes),
        failure: report.failure.clone(),
        commands: report.commands.len(),
        complete: report
            .commands
            .iter()
            .filter(|c| c.status == crate::smr::CommandStatus::Complete)
            .count(),
        recovery_fork_ok: report.recovery.as_ref().map(|r| r.fork_ok),
        pool_peak_in_flight: report.pool_peak_in_flight,
    }
}

fn smr_run_record<'a>(
    scenario: &'a SmrScenario,
    report: &SmrReport,
    metrics_path: Option<String>,
    commands_path: Option<String>,
    recovery_path: Option<String>,
    extra: Option<serde_json::Value>,
) -> SmrRunRecord<'a, SmrOutcomeSummary> {
    let (timestamp, commit, dirty) = provenance();
    SmrRunRecord {
        timestamp,
        commit,
        dirty,
        scenario,
        amp_count: amp_count(scenario.sigma, scenario.n),
        outcome: smr_outcome_summary(report),
        trace_path: None,
        metrics_path,
        commands_path,
        recovery_path,
        spill_path: report.spill_path.clone(),
        extra,
    }
}

pub fn log_smr_run_to(
    path: &Path,
    scenario: &SmrScenario,
    report: &SmrReport,
    extra: Option<serde_json::Value>,
) -> std::io::Result<()> {
    append_jsonl(
        path,
        &smr_run_record(scenario, report, None, None, None, extra),
    )
}

#[derive(Serialize)]
struct SmrAbortSummary {
    terminal: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    round: Option<usize>,
}

pub fn log_smr_abort_to(
    path: &Path,
    scenario: &SmrScenario,
    round: Option<usize>,
    extra: Option<serde_json::Value>,
) -> std::io::Result<()> {
    let (timestamp, commit, dirty) = provenance();
    append_jsonl(
        path,
        &SmrRunRecord {
            timestamp,
            commit,
            dirty,
            scenario,
            amp_count: amp_count(scenario.sigma, scenario.n),
            outcome: SmrAbortSummary {
                terminal: "abort",
                round,
            },
            trace_path: None,
            metrics_path: None,
            commands_path: None,
            recovery_path: None,
            // No report to read a spill path off; the abort's file is the one without a trailer.
            spill_path: None,
            extra,
        },
    )
}

pub fn run_smr_recorded(
    scenario: &SmrScenario,
    ledger: &Path,
    extra: Option<serde_json::Value>,
) -> (SmrReport, String, String) {
    let report = run_smr(scenario);
    let (metrics_path, commands_path) = record_smr_report(scenario, &report, ledger, extra);
    (report, metrics_path, commands_path)
}

pub fn record_smr_report(
    scenario: &SmrScenario,
    report: &SmrReport,
    ledger: &Path,
    extra: Option<serde_json::Value>,
) -> (String, String) {
    let stamp = utc_now().replace(':', "");
    let base = ledger.parent().unwrap_or(Path::new("."));
    let name = format!("{stamp}-smr-n{}-seed{}", scenario.n, scenario.seed);

    let mut csv = format!("{SMR_METRICS_CSV_HEADER}\n");
    for (i, m) in report.metrics.iter().enumerate() {
        csv.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{}\n",
            i + 1,
            m.nonbot_logs,
            m.blocked,
            m.useful,
            m.distinct_logs,
            m.max_log_len,
            m.min_executed_len,
            m.max_executed_len,
            m.arrivals,
            m.lcp_len
        ));
    }
    let metrics_file = base.join("metrics").join(format!("{name}.csv"));
    std::fs::create_dir_all(metrics_file.parent().expect("has parent")).expect("metrics dir");
    std::fs::write(&metrics_file, csv).expect("metrics write failed");

    let commands_file = base.join("commands").join(format!("{name}.json"));
    std::fs::create_dir_all(commands_file.parent().expect("has parent")).expect("commands dir");
    std::fs::write(
        &commands_file,
        serde_json::to_string(&report.commands).expect("commands serialize"),
    )
    .expect("commands write failed");

    let recovery_path = report.recovery.as_ref().map(|rec| {
        let mut csv =
            String::from("round,noreset,reset,bot_r,window,rollbacks,max_checkpoint_window\n");
        for (i, r) in rec.rounds.iter().enumerate() {
            csv.push_str(&format!(
                "{},{},{},{},{},{},{}\n",
                i + 1,
                r.noreset,
                r.reset,
                r.bot_r,
                r.window,
                r.rollbacks,
                r.max_checkpoint_window
            ));
        }
        let file = base.join("metrics").join(format!("{name}-recovery.csv"));
        std::fs::write(&file, csv).expect("recovery metrics write failed");
        file.to_string_lossy().into_owned()
    });

    let metrics_path = metrics_file.to_string_lossy().into_owned();
    let commands_path = commands_file.to_string_lossy().into_owned();
    let record = smr_run_record(
        scenario,
        report,
        Some(metrics_path.clone()),
        Some(commands_path.clone()),
        recovery_path,
        extra,
    );
    append_jsonl(ledger, &record).expect("run ledger write failed (set SIM_RUNLOG=off to opt out)");
    (metrics_path, commands_path)
}

pub fn run_smr_logged(scenario: &SmrScenario, extra: Option<serde_json::Value>) -> SmrReport {
    let Some(ledger) = ledger_target(None) else {
        return run_smr(scenario);
    };
    run_smr_recorded(scenario, &ledger, extra).0
}
