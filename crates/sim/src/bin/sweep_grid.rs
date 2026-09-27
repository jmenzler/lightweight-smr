//! Spec-driven grid sweep runner. Usage: sweep_grid <spec.json> [out.csv]

use sim::sweep::{AnySpec, concat_csv, load_specs, run_grid_streaming, run_smr_grid_streaming};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let spec_path = args
        .get(1)
        .expect("usage: sweep_grid <spec.json> [out.csv]");
    let specs = load_specs(spec_path).unwrap_or_else(|e| panic!("{e}"));
    let same_kind = specs.windows(2).all(|w| {
        matches!(
            (&w[0], &w[1]),
            (AnySpec::Grid(_), AnySpec::Grid(_)) | (AnySpec::Smr(_), AnySpec::Smr(_))
        )
    });
    assert!(same_kind, "array entries must all be the same grid kind");

    let exp = match &specs[0] {
        AnySpec::Grid(g) => g.exp.clone(),
        AnySpec::Smr(g) => g.exp.clone(),
    };
    let out = args
        .get(2)
        .cloned()
        .unwrap_or_else(|| format!("{exp}-grid.csv"));
    let partial = std::path::PathBuf::from(format!("{out}.partial"));
    let _ = std::fs::remove_file(&partial);
    if std::env::var("SIM_PROGRESS").as_deref() != Ok("off") {
        sim::progress::activate(std::path::PathBuf::from(format!("{out}.progress.json")));
    }

    let parts: Vec<String> = specs
        .iter()
        .map(|spec| match spec {
            AnySpec::Grid(g) => run_grid_streaming(g, None, Some(&partial))
                .unwrap_or_else(|e| panic!("grid failed: {e}")),
            AnySpec::Smr(g) => run_smr_grid_streaming(g, None, Some(&partial))
                .unwrap_or_else(|e| panic!("grid failed: {e}")),
        })
        .collect();
    let csv = concat_csv(&parts);

    for line in sim::sweep::grid_summary(&csv) {
        eprintln!("{line}");
    }

    std::fs::write(&out, csv).expect("write csv");
    let _ = std::fs::remove_file(&partial);
    let (noop, unions) = protocol::merge::noop_union_counts();
    if unions > 0 {
        eprintln!(
            "fast-path hits {noop} of {unions} merges tested ({:.2}%)",
            100.0 * noop as f64 / unions as f64
        );
    }
    eprintln!("wrote {out}");
}
