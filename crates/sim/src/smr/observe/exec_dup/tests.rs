use super::*;
use crate::smr::observe::safety::PrefixChecker;
use protocol::compact::ClientCommand;
use protocol::shared_state::SharedState;
use std::collections::BTreeMap;

fn cmd(client: u32, sn: u64, op: u64) -> Entry {
    Entry::Cmd(ClientCommand { client, sn, op })
}

fn state(entries: &[Entry]) -> SharedState {
    SharedState::from_entries(entries.to_vec(), BTreeMap::new())
}

/// One round of the engine's wiring: canonical path while the prefix check passes, per-node after.
fn round(
    checker: &mut PrefixChecker,
    oracle: &mut ExecDupOracle,
    prefix_ok: &mut bool,
    states: &[SharedState],
    replaced: &[bool],
    r: usize,
) {
    let views: Vec<ExecutedView<'_>> = states.iter().map(SharedState::executed).collect();
    if *prefix_ok && !checker.check_round(&views, 0) {
        *prefix_ok = false;
    }
    if *prefix_ok {
        let canonical = checker.canonical();
        oracle.observe_canonical(canonical.start(), canonical.retained(), r);
    } else {
        oracle.observe_nodes(&views, replaced, r);
    }
}

#[test]
fn a_repeat_on_every_server_passes_the_prefix_check_but_trips_the_oracle() {
    let mut checker = PrefixChecker::new(3);
    let mut oracle = ExecDupOracle::new(3);
    let mut ok = true;
    let clean = [cmd(1, 1, 10), cmd(2, 1, 20)];
    let dup = [cmd(1, 1, 10), cmd(2, 1, 20), cmd(1, 1, 10)];
    let s1 = vec![state(&clean), state(&clean), state(&clean[..1])];
    round(&mut checker, &mut oracle, &mut ok, &s1, &[false; 3], 1);
    assert_eq!(oracle.tripped_round(), None);
    let s2 = vec![state(&dup), state(&dup), state(&dup)];
    round(&mut checker, &mut oracle, &mut ok, &s2, &[false; 3], 2);
    assert!(
        ok,
        "every server holds the same sequence: prefix consistent"
    );
    assert_eq!(oracle.tripped_round(), Some(2));
}

#[test]
fn a_clean_history_never_trips() {
    let mut checker = PrefixChecker::new(2);
    let mut oracle = ExecDupOracle::new(2);
    let mut ok = true;
    let seq: Vec<Entry> = (1..=6).map(|sn| cmd(7, sn, sn * 100)).collect();
    for r in 1..=6 {
        let s = vec![state(&seq[..r]), state(&seq[..r / 2])];
        round(&mut checker, &mut oracle, &mut ok, &s, &[false; 2], r);
    }
    assert!(ok);
    assert_eq!(oracle.tripped_round(), None);
}

#[test]
fn the_same_slot_with_a_different_op_is_a_double_execution() {
    let mut oracle = ExecDupOracle::new(1);
    oracle.observe_canonical(0, &[cmd(1, 1, 10), cmd(1, 1, 11)], 4);
    assert_eq!(oracle.tripped_round(), Some(4));
}

#[test]
fn the_latch_keeps_its_first_round() {
    let mut oracle = ExecDupOracle::new(1);
    oracle.observe_canonical(0, &[cmd(1, 1, 10), cmd(1, 1, 10)], 3);
    oracle.observe_canonical(
        0,
        &[cmd(1, 1, 10), cmd(1, 1, 10), cmd(2, 1, 1), cmd(2, 1, 1)],
        9,
    );
    assert_eq!(oracle.tripped_round(), Some(3));
}

#[test]
fn after_a_split_a_repeat_on_one_branch_trips() {
    let mut checker = PrefixChecker::new(2);
    let mut oracle = ExecDupOracle::new(2);
    let mut ok = true;
    let (a, b, c) = (cmd(1, 1, 1), cmd(2, 1, 2), cmd(3, 1, 3));
    let start = [
        state(std::slice::from_ref(&a)),
        state(std::slice::from_ref(&a)),
    ];
    round(&mut checker, &mut oracle, &mut ok, &start, &[false; 2], 1);
    let split = [
        state(&[a.clone(), b.clone()]),
        state(&[a.clone(), c.clone()]),
    ];
    round(&mut checker, &mut oracle, &mut ok, &split, &[false; 2], 2);
    assert!(!ok, "the branches diverge");
    assert_eq!(oracle.tripped_round(), None, "no server repeats a slot yet");
    let later = [
        state(&[a.clone(), b.clone(), c.clone()]),
        state(&[a.clone(), c.clone(), b.clone()]),
    ];
    round(&mut checker, &mut oracle, &mut ok, &later, &[false; 2], 3);
    assert_eq!(
        oracle.tripped_round(),
        None,
        "each branch executes b and c once"
    );
    let dup = [
        state(&[a.clone(), b.clone(), c.clone()]),
        state(&[a.clone(), c.clone(), b.clone(), c.clone()]),
    ];
    round(&mut checker, &mut oracle, &mut ok, &dup, &[false; 2], 4);
    assert_eq!(oracle.tripped_round(), Some(4));
}

#[test]
fn a_repeat_of_a_forgotten_entry_is_still_caught() {
    let mut oracle = ExecDupOracle::new(1);
    let (a, b) = (cmd(1, 1, 1), cmd(2, 1, 2));
    oracle.observe_canonical(0, &[a.clone(), b.clone()], 1);
    let mut s = state(&[a.clone(), b.clone(), a.clone()]);
    s.forget_committed_prefix(2);
    oracle.observe_nodes(&[s.executed()], &[false], 2);
    assert_eq!(oracle.tripped_round(), Some(2));
}

#[test]
fn a_replaced_state_is_rescanned_not_appended() {
    let mut oracle = ExecDupOracle::new(1);
    let (a, b, c) = (cmd(1, 1, 1), cmd(2, 1, 2), cmd(3, 1, 3));
    let first = state(&[a.clone(), b.clone()]);
    oracle.observe_nodes(&[first.executed()], &[false], 1);
    // An adopted peer state of the same length: read as an append it would look unchanged.
    let adopted = state(&[a.clone(), c.clone()]);
    oracle.observe_nodes(&[adopted.executed()], &[true], 2);
    let grown = state(&[a.clone(), c.clone(), b.clone()]);
    oracle.observe_nodes(&[grown.executed()], &[false], 3);
    assert_eq!(oracle.tripped_round(), None, "b left with the old state");
    let dup = state(&[a.clone(), c.clone(), b.clone(), c.clone()]);
    oracle.observe_nodes(&[dup.executed()], &[false], 4);
    assert_eq!(oracle.tripped_round(), Some(4));
}
