//! Sim-side per-command landmark export. Usage: smr_commands <smr-scenario-spec.json> <out.json>

use sim::smr_commands::commands_json;
use sim::spec::SmrScenarioSpec;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let spec_path = args
        .get(1)
        .expect("usage: smr_commands <smr-scenario-spec.json> <out.json>");
    let out_path = args
        .get(2)
        .expect("usage: smr_commands <smr-scenario-spec.json> <out.json>");
    let spec_text = std::fs::read_to_string(spec_path).expect("read spec");
    let spec: SmrScenarioSpec = serde_json::from_str(&spec_text).expect("parse spec");
    let json = commands_json(spec).unwrap_or_else(|e| panic!("{e}"));
    std::fs::write(out_path, json).expect("write out.json");
    eprintln!("wrote {out_path}");
}
