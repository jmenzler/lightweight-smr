use sim::{BlockSchedule, Config, Init, Scenario, run_traced};

fn scenario(n: usize, seed: u64, init: Init) -> Scenario {
    Scenario {
        n,
        seed,
        cfg: Config::default(),
        init,
        max_rounds: 1,
        schedule: BlockSchedule::FreshPerRound { fraction: 0.0 },
        partition: None,
    }
}

fn initial(n: usize, seed: u64, init: Init) -> Vec<Option<u64>> {
    run_traced(&scenario(n, seed, init)).1.initial
}

#[test]
fn even_split_gives_equal_contiguous_blocks() {
    let init = initial(100, 1, Init::EvenSplit { values: 4 });
    for (i, v) in init.iter().enumerate() {
        assert_eq!(*v, Some((i / 25) as u64), "node {i}");
    }
}

#[test]
fn even_split_two_replays_split_half_exactly() {
    let (_, a) = run_traced(&{
        let mut sc = scenario(50, 3, Init::EvenSplit { values: 2 });
        sc.max_rounds = 200;
        sc
    });
    let (_, b) = run_traced(&{
        let mut sc = scenario(50, 3, Init::Split { fraction: 0.5 });
        sc.max_rounds = 200;
        sc
    });
    assert_eq!(
        serde_json::to_string(&a).unwrap(),
        serde_json::to_string(&b).unwrap(),
        "neither init draws RNG, so traces must be byte-identical"
    );
}

#[test]
fn weighted_is_deterministic_and_respects_support() {
    let weights = vec![1.0, 0.0, 3.0];
    let a = initial(
        2000,
        7,
        Init::Weighted {
            weights: weights.clone(),
            range: 3,
        },
    );
    let b = initial(2000, 7, Init::Weighted { weights, range: 3 });
    assert_eq!(a, b, "same seed, same assignment");

    let mut counts = [0usize; 3];
    for v in a.iter().flatten() {
        counts[*v as usize] += 1;
    }
    assert_eq!(counts[1], 0, "zero-weight value must never be sampled");
    let ratio = counts[2] as f64 / counts[0] as f64;
    assert!(
        (2.0..4.0).contains(&ratio),
        "3:1 weights should give roughly 3x counts, got {ratio}"
    );
}

#[test]
fn undecided_nodes_draw_nothing_with_weighted_inner() {
    let all_bot_weighted = initial(
        60,
        11,
        Init::WithUndecided {
            useful_fraction: 0.0,
            inner: Box::new(Init::Weighted {
                weights: vec![1.0, 1.0],
                range: 2,
            }),
        },
    );
    let all_bot_distinct = initial(
        60,
        11,
        Init::WithUndecided {
            useful_fraction: 0.0,
            inner: Box::new(Init::Distinct),
        },
    );
    assert!(all_bot_weighted.iter().all(Option::is_none));
    assert_eq!(
        all_bot_weighted, all_bot_distinct,
        "⊥ nodes must not consume RNG regardless of inner init"
    );
}

#[test]
fn weighted_big_range_shapes_and_sparsifies() {
    let init = initial(
        4000,
        13,
        Init::Weighted {
            weights: vec![1.0, 3.0],
            range: 1 << 32,
        },
    );
    let values: Vec<u64> = init.iter().flatten().copied().collect();
    let half = 1u64 << 31;
    let low = values.iter().filter(|v| **v < half).count();
    let high = values.len() - low;
    assert!(
        (2.0..4.0).contains(&(high as f64 / low as f64)),
        "3:1 bucket weights should shape the range, got {low} low / {high} high"
    );
    assert!(
        values.iter().all(|v| *v < (1 << 32)),
        "values must stay in range"
    );
    let distinct: std::collections::HashSet<u64> = values.iter().copied().collect();
    assert!(
        distinct.len() >= 3990,
        "2^32 range must be sparse: {} distinct of {}",
        distinct.len(),
        values.len()
    );
}

#[test]
fn weighted_zero_weight_bucket_stays_empty_in_range() {
    let init = initial(
        1000,
        3,
        Init::Weighted {
            weights: vec![1.0, 0.0],
            range: 100,
        },
    );
    assert!(
        init.iter().flatten().all(|v| *v < 50),
        "zero-weight upper bucket must receive no values"
    );
}

#[test]
fn weighted_big_range_is_deterministic() {
    let spec = || Init::Weighted {
        weights: vec![1.0, 2.0, 1.0],
        range: 1 << 40,
    };
    assert_eq!(initial(500, 77, spec()), initial(500, 77, spec()));
}

#[test]
fn with_undecided_applies_inner_init_over_the_useful_subset() {
    // Split{0.5} under useful_fraction 0.6 must split the 60 useful nodes
    // 30/30 — not slice the full-population split (which would give 50/10).
    let init = initial(
        100,
        7,
        Init::WithUndecided {
            useful_fraction: 0.6,
            inner: Box::new(Init::Split { fraction: 0.5 }),
        },
    );
    let zeros = init.iter().filter(|v| **v == Some(0)).count();
    let ones = init.iter().filter(|v| **v == Some(1)).count();
    let bots = init.iter().filter(|v| v.is_none()).count();
    assert_eq!((zeros, ones, bots), (30, 30, 40));
}
