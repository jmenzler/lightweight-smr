//! In-memory harness driver: perfect-network rehearsal writing the same outputs as bin/collect.rs.

use network_node::cli::arg;
use network_node::collect::{aggregate, summary_json};
use network_node::harness;
use network_node::spec::{NodeRunSpec, ScenarioKind};

fn die(msg: &str) -> ! {
    network_node::cli::die("harness_run", msg)
}

fn main() {
    let spec_path = arg("--spec").unwrap_or_else(|| die("--spec required"));
    let out = arg("--out").unwrap_or_else(|| die("--out required"));
    let spec_json =
        std::fs::read_to_string(&spec_path).unwrap_or_else(|e| die(&format!("{spec_path}: {e}")));
    let spec = NodeRunSpec::from_json(&spec_json).unwrap_or_else(|e| die(&format!("spec: {e}")));
    if matches!(spec.scenario, ScenarioKind::Smr(_)) {
        die(
            "harness_run supports median/gossip/priority scenarios only — \
             SMR needs the client stage, which the in-memory harness doesn't drive",
        );
    }

    let records = harness::run(&spec);
    let collected = aggregate(&records, &spec).unwrap_or_else(|e| die(&e));

    let out_dir = std::path::Path::new(&out);
    std::fs::create_dir_all(out_dir).unwrap_or_else(|e| die(&format!("mkdir: {e}")));
    std::fs::write(out_dir.join("metrics.csv"), &collected.metrics_csv)
        .unwrap_or_else(|e| die(&e.to_string()));
    std::fs::write(out_dir.join("messages.csv"), &collected.messages_csv)
        .unwrap_or_else(|e| die(&e.to_string()));
    std::fs::write(out_dir.join("summary.json"), summary_json(&collected))
        .unwrap_or_else(|e| die(&e.to_string()));
    println!("wrote {}", out_dir.display());
}
