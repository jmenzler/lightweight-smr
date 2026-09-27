use super::*;
use protocol::compact::ClientCommand;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha12Rng;

fn cmd(op: u64) -> Entry {
    Entry::Cmd(ClientCommand {
        client: op as u32,
        sn: 1,
        op,
    })
}

fn timed(op: u64) -> Timed {
    Timed {
        entry: cmd(op),
        round: 0,
    }
}

fn seqs(rows: &[Option<Vec<Timed>>]) -> Vec<Option<ChunkSeq<Timed>>> {
    rows.iter()
        .map(|r| r.clone().map(ChunkSeq::from_vec))
        .collect()
}

fn logs(rows: &[Option<Vec<Timed>>]) -> LogIndex {
    LogIndex::build(seqs(rows).iter().map(Option::as_ref))
}

#[test]
fn holders_come_out_in_ascending_node_id() {
    let idx = logs(&[
        Some(vec![timed(1)]),
        None,
        Some(vec![timed(2), timed(1)]),
        Some(vec![timed(1)]),
    ]);
    assert_eq!(idx.holder_ids(&cmd(1)), [0, 2, 3]);
    assert_eq!(idx.holder_ids(&cmd(2)), [2]);
}

#[test]
fn a_repeated_entry_names_its_node_once() {
    let idx = logs(&[
        Some(vec![timed(1), timed(2), timed(1)]),
        Some(vec![timed(1)]),
    ]);
    assert_eq!(idx.holder_ids(&cmd(1)), [0, 1]);
    assert_eq!(idx.count(&cmd(1)), 2);
}

#[test]
fn holder_pos_is_the_position_the_front_scan_would_return() {
    let rows = [
        Some(vec![timed(1), timed(2), timed(1)]),
        None,
        Some(vec![timed(3), timed(1)]),
    ];
    let idx = LogIndex::build(seqs(&rows).iter().map(Option::as_ref));
    for (node, row) in rows.iter().enumerate() {
        for op in 1..=4 {
            let want = row
                .as_ref()
                .and_then(|l| l.iter().position(|t| t.entry == cmd(op)).map(|p| p as u32));
            assert_eq!(
                idx.entry_holders(&cmd(op))
                    .and_then(|ids| holder_pos(ids, node as u32)),
                want,
                "node {node}, op {op}"
            );
        }
    }
}

#[test]
fn an_absent_entry_has_no_holders() {
    let idx = logs(&[Some(vec![timed(1)]), Some(vec![timed(1)])]);
    assert_eq!(idx.holder_ids(&cmd(9)), [] as [u32; 0]);
    assert_eq!(idx.count(&cmd(9)), 0);
    assert!(!idx.covers_all(&cmd(9)));
}

#[test]
fn bot_nodes_are_excluded_from_coverage() {
    // Node 1 is ⊥, so coverage is decided by nodes 0 and 2 alone.
    let idx = logs(&[Some(vec![timed(1)]), None, Some(vec![timed(1), timed(2)])]);
    assert!(idx.covers_all(&cmd(1)));
    assert!(!idx.covers_all(&cmd(2)));
}

#[test]
fn coverage_is_vacuous_with_no_live_log() {
    let idx = logs(&[None, None]);
    assert!(!idx.covers_all(&cmd(1)));
    assert_eq!(idx.count(&cmd(1)), 0);
}

#[test]
fn an_entry_bearing_round_does_not_split_the_key() {
    let rows = [
        Some(vec![Timed {
            entry: cmd(1),
            round: 3,
        }]),
        Some(vec![Timed {
            entry: cmd(1),
            round: 9,
        }]),
    ];
    let idx = LogIndex::build(seqs(&rows).iter().map(Option::as_ref));
    assert!(idx.covers_all(&cmd(1)));
}

#[test]
fn a_masked_build_counts_only_the_nodes_it_was_given() {
    let rows = [
        Some(vec![timed(1)]),
        Some(vec![timed(1)]),
        Some(vec![timed(1)]),
    ];
    let useful = [true, false, true];
    let held = seqs(&rows);
    let idx = LogIndex::build(
        held.iter()
            .zip(useful)
            .map(|(log, u)| if u { log.as_ref() } else { None }),
    );
    assert_eq!(idx.count(&cmd(1)), 2);
    assert_eq!(idx.holder_ids(&cmd(1)), [0, 2]);
}

#[test]
fn near_equal_payload_variants_index_separately() {
    let cmd_e = Entry::Cmd(ClientCommand {
        client: 7,
        sn: 3,
        op: 3,
    });
    let null_e = Entry::Null { client: 7, sn: 3 };
    let nop_e = Entry::Nop(3);
    let log: Vec<Timed> = [cmd_e.clone(), null_e.clone(), nop_e.clone()]
        .into_iter()
        .map(|entry| Timed { entry, round: 0 })
        .collect();
    let rows = [Some(log)];
    let idx = LogIndex::build(seqs(&rows).iter().map(Option::as_ref));
    for (want_pos, e) in [(0, &cmd_e), (1, &null_e), (2, &nop_e)] {
        assert_eq!(idx.count(e), 1);
        assert_eq!(
            idx.entry_holders(e).and_then(|ids| holder_pos(ids, 0)),
            Some(want_pos)
        );
    }
}

#[test]
fn log_counts_agrees_with_the_full_index_on_every_query() {
    let rows = [
        Some(vec![timed(1), timed(2), timed(1)]),
        None,
        Some(vec![timed(2)]),
        Some(vec![timed(1), timed(3)]),
    ];
    let held = seqs(&rows);
    let idx = LogIndex::build(held.iter().map(Option::as_ref));
    let counts = LogCounts::build(held.iter().map(Option::as_ref));
    for op in 1..=4 {
        assert_eq!(counts.count(&cmd(op)), idx.count(&cmd(op)), "op {op}");
    }
}

#[test]
fn log_counts_masked_build_matches_the_masked_index() {
    let rows = [
        Some(vec![timed(1)]),
        Some(vec![timed(1)]),
        Some(vec![timed(1)]),
    ];
    let useful = [true, false, true];
    let held = seqs(&rows);
    let counts = LogCounts::build(
        held.iter()
            .zip(useful)
            .map(|(log, u)| if u { log.as_ref() } else { None }),
    );
    assert_eq!(counts.count(&cmd(1)), 2);
}

#[test]
fn distinct_log_count_matches_the_set_semantics() {
    let a = vec![timed(1), timed(2)];
    let a2 = vec![timed(1), timed(2)];
    let b = vec![timed(1)];
    let c = vec![
        Timed {
            entry: cmd(1),
            round: 5,
        },
        timed(2),
    ];
    let empty1: Vec<Timed> = Vec::new();
    let empty2: Vec<Timed> = Vec::new();
    let held: Vec<ChunkSeq<Timed>> = [&a, &a2, &b, &c, &a, &empty1, &empty2]
        .into_iter()
        .map(|v| ChunkSeq::from_vec(v.clone()))
        .collect();
    let want = [&a, &a2, &b, &c, &a, &empty1, &empty2]
        .into_iter()
        .map(Vec::as_slice)
        .collect::<std::collections::BTreeSet<_>>()
        .len() as u32;
    assert_eq!(distinct_log_count(held.iter()), want);
    assert_eq!(want, 4, "stamp round splits contents; empty logs merge");
    assert_eq!(
        distinct_log_count(std::iter::empty::<&ChunkSeq<Timed>>()),
        0
    );
}

fn alphabet() -> Vec<Entry> {
    vec![
        cmd(1),
        cmd(2),
        cmd(3),
        Entry::Null { client: 1, sn: 1 },
        Entry::Nop(1),
        Entry::Nop(2),
    ]
}

fn assert_fold_matches_separate_passes(rows: &[Option<Vec<Timed>>], case: &str) {
    let held = seqs(rows);
    let (folded, distinct) = LogIndex::build_with_distinct(held.iter().map(Option::as_ref), true);
    let want_index = LogIndex::build(held.iter().map(Option::as_ref));
    let want_distinct = distinct_log_count(held.iter().flatten());
    assert_eq!(distinct, want_distinct, "{case}: distinct count");
    for e in alphabet().iter().chain(std::iter::once(&cmd(9))) {
        assert_eq!(
            folded.holder_ids(e),
            want_index.holder_ids(e),
            "{case}: {e:?}"
        );
        assert_eq!(
            folded.entry_holders(e),
            want_index.entry_holders(e),
            "{case}: {e:?}"
        );
        assert_eq!(folded.count(e), want_index.count(e), "{case}: {e:?}");
        assert_eq!(
            folded.covers_all(e),
            want_index.covers_all(e),
            "{case}: {e:?}"
        );
    }
}

fn random_rows(seed: u64) -> Vec<Option<Vec<Timed>>> {
    let alphabet = alphabet();
    let mut rng = ChaCha12Rng::seed_from_u64(seed);
    let mut rows: Vec<Option<Vec<Timed>>> = Vec::new();
    for _ in 0..rng.random_range(1..12) {
        if rng.random_range(0..6) == 0 {
            rows.push(None);
        } else if !rows.is_empty() && rng.random_range(0..3) == 0 {
            rows.push(rows[rng.random_range(0..rows.len())].clone());
        } else {
            let log = (0..rng.random_range(0..5))
                .map(|_| Timed {
                    entry: alphabet[rng.random_range(0..alphabet.len())].clone(),
                    round: rng.random_range(0..3),
                })
                .collect();
            rows.push(Some(log));
        }
    }
    rows
}

fn prefix_sharing_rows(seed: u64) -> Vec<Option<Vec<Timed>>> {
    let alphabet = alphabet();
    let mut rng = ChaCha12Rng::seed_from_u64(seed);
    let mut rows: Vec<Option<Vec<Timed>>> = Vec::new();
    let mut last: Vec<Timed> = Vec::new();
    for _ in 0..rng.random_range(2..14) {
        if rng.random_range(0..8) == 0 {
            rows.push(None);
            continue;
        }
        let keep = rng.random_range(0..=last.len());
        let mut log = last[..keep].to_vec();
        for _ in 0..rng.random_range(0..4) {
            log.push(Timed {
                entry: alphabet[rng.random_range(0..alphabet.len())].clone(),
                round: rng.random_range(0..3),
            });
        }
        last.clone_from(&log);
        rows.push(Some(log));
    }
    rows
}

#[test]
fn the_fold_matches_separate_passes_on_randomized_log_sets() {
    for seed in 0..500 {
        assert_fold_matches_separate_passes(&random_rows(seed), &format!("seed {seed}"));
    }
}

#[test]
fn the_fold_matches_separate_passes_on_prefix_sharing_log_sets() {
    for seed in 0..500 {
        assert_fold_matches_separate_passes(
            &prefix_sharing_rows(seed),
            &format!("prefix seed {seed}"),
        );
    }
}

#[test]
fn the_fold_matches_separate_passes_across_the_prefix_boundary() {
    let base = vec![timed(1), timed(2), timed(3)];
    let nop = Timed {
        entry: Entry::Nop(1),
        round: 0,
    };
    // The stamp round is in the content hash but not in the holder key.
    let restamped = vec![
        timed(1),
        Timed {
            entry: cmd(2),
            round: 7,
        },
        timed(3),
    ];
    for (case, rows) in [
        (
            "strict prefix of its predecessor",
            vec![Some(base.clone()), Some(base[..2].to_vec())],
        ),
        (
            "extension of its predecessor",
            vec![Some(base[..2].to_vec()), Some(base.clone())],
        ),
        (
            "diverging last entry",
            vec![
                Some(base.clone()),
                Some(vec![timed(1), timed(2), nop.clone()]),
            ],
        ),
        (
            "a repeat straddling the boundary",
            vec![
                Some(vec![timed(1), timed(2)]),
                Some(vec![timed(1), timed(2), timed(1)]),
            ],
        ),
        (
            "a repeat inside the shared prefix",
            vec![
                Some(vec![timed(1), timed(1), timed(2)]),
                Some(vec![timed(1), timed(1), timed(3)]),
            ],
        ),
        (
            "empty after a long log",
            vec![Some(base.clone()), Some(Vec::new())],
        ),
        (
            "long after empty",
            vec![Some(Vec::new()), Some(base.clone())],
        ),
        (
            "⊥ between prefix sharers",
            vec![Some(base.clone()), None, Some(base.clone())],
        ),
        (
            "stamp round alone breaks the prefix",
            vec![Some(base.clone()), Some(restamped)],
        ),
        (
            "a three-log chain of shrinking prefixes",
            vec![
                Some(base.clone()),
                Some(vec![timed(1), timed(2), nop.clone()]),
                Some(vec![timed(1), nop]),
            ],
        ),
    ] {
        assert_fold_matches_separate_passes(&rows, case);
    }
}

#[test]
fn the_fold_matches_separate_passes_on_the_relations_between_two_logs() {
    let a = vec![timed(1), timed(2)];
    let stamped = vec![
        Timed {
            entry: cmd(1),
            round: 5,
        },
        timed(2),
    ];
    let same_len = vec![timed(1), timed(3)];
    let repeats = vec![timed(1), timed(2), timed(1)];
    for (case, rows) in [
        ("equal logs", vec![Some(a.clone()), Some(a.clone())]),
        ("stamp round alone", vec![Some(a.clone()), Some(stamped)]),
        ("equal length", vec![Some(a.clone()), Some(same_len)]),
        (
            "empty logs and ⊥",
            vec![Some(Vec::new()), None, Some(Vec::new()), Some(a.clone())],
        ),
        ("only ⊥", vec![None, None]),
        ("no logs at all", Vec::new()),
        ("a repeated entry", vec![Some(repeats), Some(a.clone())]),
        (
            "⊥ between holders",
            vec![Some(a.clone()), None, Some(a.clone())],
        ),
    ] {
        assert_fold_matches_separate_passes(&rows, case);
    }
}

#[test]
fn the_fold_counts_bot_and_empty_logs_the_way_the_metric_does() {
    let rows = [None, Some(Vec::new()), None];
    let held = seqs(&rows);
    let (idx, distinct) = LogIndex::build_with_distinct(held.iter().map(Option::as_ref), true);
    assert_eq!(distinct, 1);
    assert!(!idx.covers_all(&cmd(1)));
    let (_, distinct) =
        LogIndex::build_with_distinct([None::<&ChunkSeq<Timed>>, None].into_iter(), true);
    assert_eq!(distinct, 0);
}
