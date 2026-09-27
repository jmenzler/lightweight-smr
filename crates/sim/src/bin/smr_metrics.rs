//! Per-round SMR metrics export (not ledgered). Usage: smr_metrics <scenario-spec.json> [out.csv]

use std::io::Write;

fn main() {
    let mut args = std::env::args().skip(1);
    let spec_path = args
        .next()
        .expect("usage: smr_metrics <spec.json> [out.csv]");
    let out_path = args.next();
    let spec_json = std::fs::read_to_string(&spec_path).expect("read spec");
    let spec: sim::spec::SmrScenarioSpec = serde_json::from_str(&spec_json).expect("parse spec");
    let scenario = sim::smr::SmrScenario::try_from(spec).expect("valid scenario");
    let report = sim::smr::run_smr(&scenario);
    if let Some(failure) = &report.failure {
        eprintln!(
            "SMR failed during {:?}: attempted round {}, completed round {}, observed round {}, node {}",
            failure.phase,
            failure.attempted_round,
            failure.completed_round,
            failure.observed_round,
            failure.node,
        );
        std::process::exit(2);
    }

    let rec = report.recovery.as_ref();
    let mut out = String::new();
    let base = sim::runlog::SMR_METRICS_CSV_HEADER;
    if rec.is_some() {
        out.push_str(base);
        out.push_str(",noreset,reset,bot_r,window,rollbacks,max_checkpoint_window\n");
    } else {
        out.push_str(base);
        out.push('\n');
    }
    for (i, m) in report.metrics.iter().enumerate() {
        out.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{}",
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
        if let Some(r) = rec {
            let rr = &r.rounds[i];
            out.push_str(&format!(
                ",{},{},{},{},{},{}",
                rr.noreset, rr.reset, rr.bot_r, rr.window, rr.rollbacks, rr.max_checkpoint_window
            ));
        }
        out.push('\n');
    }
    match out_path {
        Some(p) => std::fs::write(&p, out).expect("write csv"),
        None => std::io::stdout().write_all(out.as_bytes()).expect("stdout"),
    }
}
