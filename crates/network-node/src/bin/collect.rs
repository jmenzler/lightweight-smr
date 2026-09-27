//! Shadow outdir to sim-schema CSVs + summary; exits 1 when any validity gate fails.

use network_node::cli::arg;
use network_node::collect::{aggregate, aggregate_smr, summary_json};
use network_node::record::RoundRecord;
use network_node::spec::{NodeRunSpec, ScenarioKind};
use sim::smr_commands::{LandmarkRow, to_json};

fn die(msg: &str) -> ! {
    network_node::cli::die("collect", msg)
}

fn host_stdout(hosts: &std::path::Path, host: &str) -> String {
    let dir = hosts.join(host);
    let mut candidates: Vec<_> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| die(&format!("{}: {e}", dir.display())))
        .map(|e| e.unwrap_or_else(|err| die(&format!("{}: dir entry: {err}", dir.display()))))
        .map(|e| e.path())
        .filter(|p| p.to_string_lossy().ends_with(".stdout"))
        .collect();
    match candidates.len() {
        1 => std::fs::read_to_string(candidates.remove(0))
            .unwrap_or_else(|e| die(&format!("{host}: {e}"))),
        c => die(&format!("{host}: expected exactly one .stdout, found {c}")),
    }
}

fn main() {
    let outdir = arg("--outdir").unwrap_or_else(|| die("--outdir required"));
    let spec_path = arg("--spec").unwrap_or_else(|| die("--spec required"));
    let results = arg("--out").unwrap_or_else(|| die("--out required"));
    let spec_json =
        std::fs::read_to_string(&spec_path).unwrap_or_else(|e| die(&format!("{spec_path}: {e}")));
    let spec = NodeRunSpec::from_json(&spec_json).unwrap_or_else(|e| die(&format!("spec: {e}")));
    let hosts = std::path::Path::new(&outdir).join("hosts");

    let mut records: Vec<Vec<RoundRecord>> = Vec::with_capacity(spec.n());
    for i in 0..spec.n() {
        let text = host_stdout(&hosts, &format!("peer{i}"));
        let recs: Vec<RoundRecord> = text
            .lines()
            .map(|l| {
                RoundRecord::from_jsonl(l)
                    .unwrap_or_else(|e| die(&format!("peer{i}: bad record line: {e}")))
            })
            .collect();
        records.push(recs);
    }

    let smr = matches!(spec.scenario, ScenarioKind::Smr(_));
    let (out, commands_json) = if smr {
        let client = host_stdout(&hosts, "client");
        let mut arrivals: Vec<u32> = Vec::new();
        let mut starved = 0u64;
        let mut commands = String::from("{\"commands\":[]}");
        for line in client.lines() {
            let tail = |key: &str| -> Option<u32> {
                line.contains(key)
                    .then(|| line.rsplit(':').next())
                    .flatten()
                    .and_then(|s| s.trim_end_matches('}').parse::<u32>().ok())
            };
            if let Some(a) = tail("\"arrivals\"") {
                arrivals.push(a);
            } else if let Some(s) = tail("\"starved\"") {
                starved += u64::from(s);
            } else if line.starts_with("{\"commands\":") {
                commands = line.to_string();
            }
        }
        if starved > 0 {
            eprintln!(
                "collect: {starved} client attempts never left the driver — \
                 the client stage was starved, run is NOT comparable"
            );
            std::process::exit(1);
        }
        (
            aggregate_smr(&records, &spec, Some(&arrivals)).unwrap_or_else(|e| die(&e)),
            Some(commands),
        )
    } else {
        (aggregate(&records, &spec).unwrap_or_else(|e| die(&e)), None)
    };

    let results = std::path::Path::new(&results);
    std::fs::create_dir_all(results).unwrap_or_else(|e| die(&format!("mkdir: {e}")));
    std::fs::write(results.join("metrics.csv"), &out.metrics_csv)
        .unwrap_or_else(|e| die(&e.to_string()));
    std::fs::write(results.join("messages.csv"), &out.messages_csv)
        .unwrap_or_else(|e| die(&e.to_string()));
    std::fs::write(results.join("summary.json"), summary_json(&out))
        .unwrap_or_else(|e| die(&e.to_string()));
    if let Some(cmds) = commands_json {
        let lm: Vec<LandmarkRow> = out
            .landmarks
            .iter()
            .map(|l| LandmarkRow {
                op: l.op,
                all_logs_round: l.all_logs_round,
                prefix_fixed_round: l.prefix_fixed_round,
            })
            .collect();
        let merged = format!("{{\"client\":{cmds},\"landmarks\":{}}}", to_json(&lm));
        std::fs::write(results.join("commands.json"), merged)
            .unwrap_or_else(|e| die(&e.to_string()));
    }

    if !out.gates.clean() {
        eprintln!(
            "collect: VALIDITY GATES FAILED — late_rep {} late_app {} dropped_past {} encode_err {} rounds_consistent {} safety_ok {} — run is NOT E7-comparable",
            out.gates.late_rep,
            out.gates.late_app,
            out.gates.dropped_past,
            out.gates.encode_err,
            out.gates.rounds_consistent,
            out.gates.safety_ok
        );
        std::process::exit(1);
    }
    println!("gates clean; results in {}", results.display());
}
