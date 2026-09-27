//! Per-round message counts over a median GridSpec. Usage: trace_counts <spec.json> <out.csv>

use sim::sweep::{AnySpec, concat_csv, load_specs};
use sim::trace_counts::run_grid_counts;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let spec_path = args
        .get(1)
        .expect("usage: trace_counts <spec.json> <out.csv>");
    let out_path = args
        .get(2)
        .expect("usage: trace_counts <spec.json> <out.csv>");
    let specs = load_specs(spec_path).unwrap_or_else(|e| panic!("{e}"));

    let mut counts_parts = Vec::with_capacity(specs.len());
    let mut outcomes_parts = Vec::with_capacity(specs.len());
    for (i, spec) in specs.iter().enumerate() {
        let grid = match spec {
            AnySpec::Grid(g) => g,
            AnySpec::Smr(_) => {
                panic!("trace_counts is median-grid only — spec entry {i} is an SMR grid")
            }
        };
        let (counts_part, outcomes_part) =
            run_grid_counts(grid).unwrap_or_else(|e| panic!("grid failed: {e}"));
        counts_parts.push(counts_part);
        outcomes_parts.push(outcomes_part);
    }

    std::fs::write(out_path, concat_csv(&counts_parts)).expect("write counts csv");
    let outcomes_path = format!("{out_path}.outcomes.csv");
    std::fs::write(&outcomes_path, concat_csv(&outcomes_parts)).expect("write outcomes csv");
    eprintln!("wrote {out_path}");
    eprintln!("wrote {outcomes_path}");
}
