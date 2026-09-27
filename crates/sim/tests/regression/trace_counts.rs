use sim::RoundTrace;
use sim::spec::{InitSpec, ScenarioSpec, ScheduleSpec};
use sim::sweep::{GridSpec, LadderRung, run_grid_streaming};
use sim::trace_counts::{round_counts, run_grid_counts};

/// Hand-built round: node 1 is blocked, node 2 is ⊥ entering the round.
/// n=4, k=2. requests = k * unblocked = 2*3 = 6.
/// replies: node0->{1(blocked,skip),2(⊥,skip)} = 0
///          node1 is blocked, contributes nothing
///          node2->{0,0} both hit a value holder = 2
///          node3->{2(⊥,skip),1(blocked,skip)} = 0
/// total replies = 2.
#[test]
fn f6_derivation_counts_requests_and_gated_replies() {
    let round = RoundTrace {
        states: vec![Some(5), Some(0), Some(6), Some(7)],
        targets: vec![vec![1, 2], vec![0, 3], vec![0, 0], vec![2, 1]],
        blocked: vec![1],
    };
    let entering: Vec<Option<u64>> = vec![Some(5), Some(6), None, Some(7)];

    let (requests, replies) = round_counts(2, &entering, &round, None);

    assert_eq!(requests, 6);
    assert_eq!(replies, 2);
}

/// F6 accounting mirrors `SimState::step_with`'s gate, so a cross-component
/// request costs a message and buys no reply — the counts would overstate
/// delivered replies otherwise.
#[test]
fn f6_derivation_gates_replies_across_the_partition() {
    // n=4, k=2, nobody blocked, everybody holds; {0,1} | {2,3}.
    let round = RoundTrace {
        states: vec![Some(1), Some(1), Some(1), Some(1)],
        targets: vec![vec![0, 2], vec![3, 1], vec![0, 3], vec![2, 1]],
        blocked: vec![],
    };
    let entering: Vec<Option<u64>> = vec![Some(1), Some(2), Some(3), Some(4)];

    let (requests, replies) = round_counts(2, &entering, &round, Some(&[0, 0, 1, 1]));

    assert_eq!(
        requests, 8,
        "every unblocked node still sends its k requests"
    );
    // same-component hits: 0->0, 1->1, 2->3, 3->2
    assert_eq!(replies, 4);
}

#[test]
fn f6_derivation_with_no_blocking_counts_full_fan_out() {
    // n=3, k=3, nobody blocked, everybody holds a value: every request lands.
    let round = RoundTrace {
        states: vec![Some(1), Some(1), Some(1)],
        targets: vec![vec![0, 1, 2], vec![0, 1, 2], vec![0, 1, 2]],
        blocked: vec![],
    };
    let entering: Vec<Option<u64>> = vec![Some(1), Some(2), Some(3)];

    let (requests, replies) = round_counts(3, &entering, &round, None);

    assert_eq!(requests, 9);
    assert_eq!(replies, 9);
}

/// The counts rows key on (k, ell, n, beta, split, seed): without the split
/// column two partition arms of the same seed would collide into one run.
#[test]
fn the_counts_csv_carries_the_split_key_column() {
    let spec = GridSpec {
        splits: Some(vec![0.6, 0.8]),
        betas: None,
        ..smoke_spec()
    };
    let (counts_csv, _) = run_grid_counts(&spec).expect("trace grid runs");
    let cols: Vec<&str> = sim::trace_counts::COUNTS_CSV_HEADER.split(',').collect();
    let at = cols
        .iter()
        .position(|&c| c == "split")
        .expect("split column");
    assert_eq!((cols[at - 1], cols[at + 1]), ("beta", "seed"));

    let splits: std::collections::BTreeSet<&str> = counts_csv
        .lines()
        .skip(1)
        .map(|row| row.split(',').nth(at).expect("split cell"))
        .collect();
    assert_eq!(
        splits,
        ["0.600", "0.800"].into_iter().collect(),
        "each arm must carry its own split in the row key"
    );
}

fn base(init: InitSpec, schedule: Option<ScheduleSpec>) -> ScenarioSpec {
    ScenarioSpec {
        n: 60,
        seed: 0,
        k: 6,
        ell: 3,
        init,
        max_rounds: 400,
        schedule,
        partition: None,
    }
}

fn smoke_spec() -> GridSpec {
    GridSpec {
        exp: "trace-counts-smoke".into(),
        base: base(
            InitSpec::Split { fraction: 0.5 },
            Some(ScheduleSpec::FreshPerRound { fraction: 0.0 }),
        ),
        pairs: None,
        betas: Some(vec![0.100, 0.150]),
        useful_fractions: None,
        horizons: None,
        splits: None,
        ladder: vec![LadderRung {
            n: 60,
            seed_base: 9000,
            seed_count: 2,
        }],
    }
}

/// Stream-identity: trace_counts must run the byte-identical scenario stream
/// sweep_grid does, so its derived outcomes CSV matches sweep_grid's own CSV
/// exactly (same expansion order, same RNG per scenario, same formatting).
#[test]
fn outcomes_csv_matches_sweep_grid_csv_exactly() {
    let dir = std::env::temp_dir().join(format!("trace-counts-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("tempdir");
    let ledger = dir.join("ledger.jsonl");

    let spec = smoke_spec();
    let grid_csv = run_grid_streaming(&spec, Some(&ledger), None).expect("grid runs");
    let (counts_csv, outcomes_csv) = run_grid_counts(&spec).expect("trace grid runs");

    assert_eq!(
        outcomes_csv, grid_csv,
        "trace_counts outcomes must byte-match sweep_grid's own CSV"
    );

    // sanity: counts CSV carries many more rows than runs (one per round, not per run)
    let run_rows = grid_csv.lines().count() - 1;
    let counts_rows = counts_csv.lines().count() - 1;
    assert!(
        counts_rows > run_rows,
        "one count row per round, not per run"
    );
    assert_eq!(
        counts_csv.lines().next().unwrap(),
        sim::trace_counts::COUNTS_CSV_HEADER
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// No round exceeds the theoretical max (k * n requests, k * n replies) and
/// every row belongs to a round the run actually executed. Under nonzero-beta
/// blocking, `run_inner` never early-exits on agreement (persistence must be
/// confirmed to max_rounds) — only a died (AllUndecided) run early-exits at
/// its reported round; agreement/censored runs execute the full max_rounds.
#[test]
fn counts_rows_are_bounded_by_actual_rounds_executed() {
    use std::collections::HashMap;

    let spec = smoke_spec();
    let max_rounds = spec.base.max_rounds;
    let (counts_csv, outcomes_csv) = run_grid_counts(&spec).expect("trace grid runs");

    // Column index by NAME: a new key column must never silently shift a test.
    let at = |header: &str, name: &str| {
        header
            .split(',')
            .position(|c| c == name)
            .unwrap_or_else(|| panic!("no {name} column"))
    };
    let out = |cols: &[&str], name: &str| cols[at(sim::sweep::CSV_HEADER, name)].to_string();
    let cnt = |cols: &[&str], name: &str| {
        cols[at(sim::trace_counts::COUNTS_CSV_HEADER, name)].to_string()
    };

    let mut expected_rows: HashMap<(String, String, String, String, String), usize> =
        HashMap::new();
    for row in outcomes_csv.lines().skip(1) {
        let cols: Vec<&str> = row.split(',').collect();
        let key = (
            out(&cols, "k"),
            out(&cols, "ell"),
            out(&cols, "n"),
            out(&cols, "beta"),
            out(&cols, "seed"),
        );
        let rounds: usize = out(&cols, "rounds").parse().unwrap();
        let expected = if out(&cols, "outcome") == "died" {
            rounds
        } else {
            max_rounds
        };
        expected_rows.insert(key, expected);
    }

    let mut rows_per_run: HashMap<(String, String, String, String, String), usize> = HashMap::new();
    for row in counts_csv.lines().skip(1) {
        let cols: Vec<&str> = row.split(',').collect();
        let key = (
            cnt(&cols, "k"),
            cnt(&cols, "ell"),
            cnt(&cols, "n"),
            cnt(&cols, "beta"),
            cnt(&cols, "seed"),
        );
        *rows_per_run.entry(key).or_insert(0) += 1;
        let requests: usize = cnt(&cols, "requests").parse().unwrap();
        let replies: usize = cnt(&cols, "replies").parse().unwrap();
        assert!(requests <= 6 * 60, "requests bounded by k*n");
        assert!(replies <= requests, "replies gated, cannot exceed requests");
    }

    assert_eq!(rows_per_run.len(), expected_rows.len(), "4 distinct runs");
    for (key, expected) in &expected_rows {
        assert_eq!(
            rows_per_run.get(key).copied().unwrap_or(0),
            *expected,
            "run {key:?} must have one count-row per executed round"
        );
    }
}
