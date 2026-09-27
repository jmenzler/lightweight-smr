use super::*;
use protocol::compact::ClientCommand;

fn cmd(op: u64) -> Entry {
    Entry::Cmd(ClientCommand {
        client: op as u32,
        sn: 1,
        op,
    })
}

#[test]
fn a_repeated_canonical_entry_is_reported_and_keeps_its_first_position() {
    let mut idx = EntryIndex::new();
    assert!(idx.extend_unique(0, &[cmd(1), cmd(2), cmd(1)]));
    assert_eq!(idx.get(&cmd(1)), Some(0));
    assert_eq!(idx.get(&cmd(2)), Some(1));
}

#[test]
fn a_repeat_appearing_in_a_later_round_is_caught_too() {
    let mut idx = EntryIndex::new();
    assert!(!idx.extend_unique(0, &[cmd(1), cmd(2)]));
    assert!(idx.extend_unique(0, &[cmd(1), cmd(2), cmd(3), cmd(2)]));
    assert_eq!(idx.get(&cmd(3)), Some(2));
}

#[test]
fn indexing_continues_past_a_repeat() {
    let mut idx = EntryIndex::new();
    assert!(idx.extend_unique(0, &[cmd(1), cmd(1), cmd(4)]));
    assert_eq!(idx.get(&cmd(4)), Some(2));
    assert!(!idx.extend_unique(0, &[cmd(1), cmd(1), cmd(4), cmd(5)]));
}

#[test]
fn extend_first_indexes_the_first_occurrence_of_every_entry() {
    let mut idx = EntryIndex::new();
    idx.extend_first(0, &[cmd(1), cmd(2), cmd(1)]);
    assert_eq!(
        (idx.get(&cmd(1)), idx.get(&cmd(2)), idx.get(&cmd(9))),
        (Some(0), Some(1), None)
    );
}

#[test]
fn extending_boundary_by_boundary_answers_like_a_fresh_build() {
    let boundaries = [
        vec![cmd(1), cmd(2)],
        vec![cmd(1), cmd(2), cmd(3), cmd(1)],
        vec![cmd(1), cmd(2), cmd(3), cmd(1), cmd(4)],
    ];
    let mut extended = EntryIndex::new();
    for seq in &boundaries {
        extended.extend_first(0, seq);
        let mut fresh = EntryIndex::new();
        fresh.extend_first(0, seq);
        for op in 1..=5 {
            assert_eq!(extended.get(&cmd(op)), fresh.get(&cmd(op)), "op {op}");
        }
    }
}

#[test]
fn re_covering_the_same_prefix_is_not_a_duplicate() {
    let mut idx = EntryIndex::new();
    assert!(!idx.extend_unique(0, &[cmd(1), cmd(2)]));
    assert!(!idx.extend_unique(0, &[cmd(1), cmd(2)]));
    assert!(!idx.extend_unique(0, &[cmd(1), cmd(2), cmd(3)]));
    assert!(!idx.extend_unique(0, &[cmd(1), cmd(2), cmd(3)]));
}

#[test]
fn a_released_front_does_not_shift_the_positions_behind_it() {
    let whole = [cmd(1), cmd(2), cmd(3), cmd(4), cmd(5)];
    let mut walked = EntryIndex::new();
    walked.extend_first(0, &whole);

    let mut released = EntryIndex::new();
    released.extend_first(0, &whole[..2]);
    // Positions 2.. arrive with the front already dropped.
    released.extend_first(2, &whole[2..]);
    for op in 1..=5 {
        assert_eq!(released.get(&cmd(op)), walked.get(&cmd(op)), "op {op}");
    }
    assert_eq!(released.get(&cmd(5)), Some(4));
}
