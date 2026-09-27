use super::*;
use protocol::compact::ClientCommand;

fn cmd(op: u64) -> Entry {
    Entry::Cmd(ClientCommand {
        client: op as u32,
        sn: 1,
        op,
    })
}

fn state(entries: &[Entry], forget: u64) -> SharedState {
    let mut s = SharedState::from_entries(entries.to_vec(), Default::default());
    s.forget_committed_prefix(forget);
    s
}

#[test]
fn a_forgotten_prefix_does_not_move_the_chain_value() {
    let entries: Vec<Entry> = (1..=6).map(cmd).collect();
    let canon = CanonicalOrder::from_entries(&entries);
    let whole = state_digest(&canon, &state(&entries, 0));
    for forget in 0..=entries.len() as u64 {
        assert_eq!(
            state_digest(&canon, &state(&entries, forget)),
            whole,
            "forgetting {forget} moved the chain value"
        );
    }
}

#[test]
fn a_divergence_above_the_offset_moves_the_chain_value() {
    let entries: Vec<Entry> = (1..=6).map(cmd).collect();
    let canon = CanonicalOrder::from_entries(&entries);
    let mut forked = entries.clone();
    forked[4] = cmd(99);
    for forget in 0..=4 {
        assert_ne!(
            state_digest(&canon, &state(&entries, forget)),
            state_digest(&canon, &state(&forked, forget)),
            "forget {forget}: a fork above the offset hashed the same"
        );
    }
}

#[test]
fn the_same_entries_at_different_positions_differ() {
    let a = CanonicalOrder::from_entries(&[cmd(1), cmd(2)]);
    let b = CanonicalOrder::from_entries(&[cmd(9), cmd(1), cmd(2)]);
    assert_ne!(a.digest_at(2), b.digest_at(3));
}

#[test]
fn releasing_keeps_absolute_positions_and_digests() {
    let entries: Vec<Entry> = (1..=6).map(cmd).collect();
    let full = CanonicalOrder::from_entries(&entries);
    let mut cut = CanonicalOrder::from_entries(&entries);
    cut.release_to(4);
    assert_eq!(cut.start(), 4);
    assert_eq!(cut.len(), full.len());
    for at in 4..=6 {
        assert_eq!(cut.digest_at(at), full.digest_at(at), "at {at}");
    }
    assert_eq!(cut.slice(4, 6), full.slice(4, 6));
}

#[test]
#[should_panic(expected = "below the released prefix")]
fn reading_a_released_digest_panics_rather_than_answering() {
    let mut canon = CanonicalOrder::from_entries(&[cmd(1), cmd(2), cmd(3)]);
    canon.release_to(2);
    canon.digest_at(1);
}

#[test]
fn a_position_below_the_offset_resolves_through_the_canonical_order() {
    let entries: Vec<Entry> = (1..=5).map(cmd).collect();
    let canon = CanonicalOrder::from_entries(&entries);
    let s = state(&entries, 3);
    let ex = Executed {
        view: s.executed(),
        canonical: &canon,
    };
    for op in 1..=5 {
        assert_eq!(
            ex.front_scan(&cmd(op)),
            Some(op as u32 - 1),
            "op {op} was not found across the offset"
        );
    }
    assert_eq!(ex.front_scan(&cmd(9)), None);
}

#[test]
fn retained_front_scan_prefers_an_earlier_duplicate_to_the_live_tail() {
    let entries = [cmd(1), cmd(7), cmd(2), cmd(7), cmd(3)];
    let mut canon = CanonicalOrder::from_entries(&entries);
    canon.release_to(1);
    let state = state(&entries, 3);
    let ex = Executed {
        view: state.executed(),
        canonical: &canon,
    };

    assert_eq!(canon.start(), 1);
    assert_eq!(ex.front_scan(&cmd(7)), Some(1));
}
