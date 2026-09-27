//! The streamed grid holds a chunk of completed reports before flushing, so
//! the chunk size is a memory dial (`SIM_STREAM_CHUNK`). Turning it must move
//! nothing observable: rows come out in the expansion order because
//! `par_iter().collect()` is order-preserving and the row/ledger writes run
//! serially over that collection. These pin both halves of that.
//!
//! Single-threaded on purpose: the env var is process-global, so the cases
//! must not run concurrently.

use sim::sweep::{AnySpec, parse_specs, run_smr_grid_streaming};

const SPEC: &str = r#"[{
  "exp": "chunk-pin",
  "base": {
    "n": 24, "seed": 0, "k": 6, "ell": 3, "sigma": 1.0,
    "proto": { "kind": "compact", "t_commit_rounds": 10 },
    "injections": [], "max_rounds": 60,
    "schedule": { "kind": "fresh_per_round", "fraction": 0.0 }
  },
  "betas": [0.1], "rates": [2], "t_commits": [10],
  "ladder": [{ "n": 24, "seed_base": 975300, "seed_count": 21 }]
}]"#;

fn grid() -> sim::sweep::SmrGridSpec {
    let specs = parse_specs(SPEC).expect("spec parses");
    match specs.into_iter().next().expect("one grid") {
        AnySpec::Smr(g) => g,
        AnySpec::Grid(_) => panic!("smr grid"),
    }
}

/// 21 points over chunk sizes that straddle it: 1 (a flush per run), 8 (the
/// default, three flushes), 64 (one flush, the old constant).
fn csv_and_ledger(chunk: &str, tag: &str) -> (String, Vec<String>) {
    let dir = std::env::temp_dir().join(format!("smr-chunk-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("tempdir");
    let ledger = dir.join("ledger.jsonl");
    let _ = std::fs::remove_file(&ledger);
    // SAFETY-adjacent: the suite runs these serially within one test.
    unsafe { std::env::set_var("SIM_STREAM_CHUNK", chunk) };
    let csv = run_smr_grid_streaming(&grid(), Some(&ledger), None).expect("grid runs");
    unsafe { std::env::remove_var("SIM_STREAM_CHUNK") };
    let rows = std::fs::read_to_string(&ledger).expect("ledger written");
    let seeds: Vec<String> = rows
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).expect("ledger row is json");
            v["scenario"]["seed"].to_string()
        })
        .collect();
    (csv, seeds)
}

#[test]
fn csv_and_ledger_order_are_invariant_under_the_flush_chunk() {
    let (csv1, led1) = csv_and_ledger("1", "c1");
    let (csv8, led8) = csv_and_ledger("8", "c8");
    let (csv64, led64) = csv_and_ledger("64", "c64");

    assert_eq!(csv1, csv8, "chunk 1 vs 8 changed the CSV");
    assert_eq!(csv8, csv64, "chunk 8 vs 64 changed the CSV");
    assert_eq!(csv8.lines().count(), 22, "header + one row per seed");

    assert_eq!(led1, led8, "chunk 1 vs 8 reordered the ledger");
    assert_eq!(led8, led64, "chunk 8 vs 64 reordered the ledger");
    assert_eq!(led8.len(), 21, "one ledger row per run");

    // Expansion order, not completion order — the point of the serial flush.
    let expected: Vec<String> = (975_300..975_321).map(|s| s.to_string()).collect();
    assert_eq!(
        led8, expected,
        "ledger rows must follow the expansion order"
    );
}
