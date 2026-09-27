//! The shared state keeps `S` as (logical offset, tail) so a server can forget
//! committed commands (§4 p. 25). Everything an observer may still ask about
//! entries at or above the offset must answer exactly as before, and everything
//! below it must fail loudly rather than quietly answer from nowhere.

use protocol::compact::{ClientCommand, Entry, SharedState};
use std::collections::BTreeMap;

fn entry(op: u64) -> Entry {
    Entry::Cmd(ClientCommand {
        client: op as u32,
        sn: 1,
        op,
    })
}

fn state(ops: &[u64]) -> SharedState {
    SharedState::from_entries(
        ops.iter().copied().map(entry).collect(),
        BTreeMap::from([(1u32, 4u64)]),
    )
}

#[test]
fn forgetting_a_prefix_leaves_the_logical_sequence_alone() {
    let mut s = state(&[1, 2, 3, 4, 5]);
    let sn: Vec<_> = s.sn_iter().collect();
    s.forget_committed_prefix(2);
    assert_eq!(s.logical_len(), 5, "the sequence is only shorter to hold");
    assert_eq!(s.executed_offset(), 2);
    assert_eq!(
        s.sn_iter().collect::<Vec<_>>(),
        sn,
        "forgetting commands never forgets a client"
    );
    for i in 2..5 {
        assert_eq!(s.executed().get(i), Some(&entry(i + 1)));
    }
    assert_eq!(
        s.executed().get(5),
        None,
        "past the end is absent, not gone"
    );
}

#[test]
fn forgetting_is_idempotent_and_never_walks_backwards() {
    let mut s = state(&[1, 2, 3, 4]);
    s.forget_committed_prefix(3);
    for upto in [3, 2, 0] {
        s.forget_committed_prefix(upto);
        assert_eq!((s.executed_offset(), s.logical_len()), (3, 4));
    }
}

#[test]
fn forgetting_clamps_at_the_logical_length() {
    let mut s = state(&[1, 2, 3]);
    s.forget_committed_prefix(99);
    assert_eq!((s.executed_offset(), s.logical_len()), (3, 3));
    assert_eq!(s.logical_len(), s.executed_offset(), "nothing live remains");
}

#[test]
#[should_panic(expected = "below the forgotten prefix")]
fn reading_below_the_offset_panics_rather_than_answering() {
    let mut s = state(&[1, 2, 3]);
    s.forget_committed_prefix(2);
    let _ = s.executed().get(1);
}

#[test]
#[should_panic(expected = "truncated")]
fn asking_a_truncated_state_for_its_whole_history_panics() {
    let mut s = state(&[1, 2, 3]);
    s.forget_committed_prefix(1);
    let _ = s.untruncated();
}

#[test]
fn equal_logical_sequences_are_equal_whatever_the_offset() {
    let mut a = state(&[1, 2, 3]);
    let mut b = state(&[1, 2, 3]);
    assert_eq!(a, b);
    a.forget_committed_prefix(2);
    b.forget_committed_prefix(2);
    assert_eq!(a, b, "the same frontier keeps them equal");
}

#[test]
fn different_lengths_are_unequal_and_never_panic() {
    let a = state(&[1, 2, 3]);
    let mut b = state(&[1, 2]);
    assert_ne!(a, b);
    b.forget_committed_prefix(2);
    assert_ne!(
        a, b,
        "unequal lengths with unequal offsets is an ordinary no"
    );
}

/// Two observable states of equal logical length must have been truncated with
/// one frontier — that is what makes comparing the tails exact. A pair that
/// violates it is a driver bug, and answering `false` would hide it inside a
/// fork count.
#[test]
#[should_panic(expected = "one frontier")]
fn equal_lengths_with_unequal_offsets_panics() {
    let a = state(&[1, 2, 3]);
    let mut b = state(&[1, 2, 3]);
    b.forget_committed_prefix(2);
    let _ = a == b;
}

#[cfg(feature = "serde")]
#[test]
fn the_wire_form_is_the_materialized_sequence() {
    let s = state(&[1, 2, 3]);
    let json = serde_json::to_string(&s).expect("serializes");
    assert!(
        json.starts_with(r#"{"executed":[{"Cmd":"#),
        "the storage split must not reach the wire: {json}"
    );
    assert!(!json.contains("offset"), "{json}");
    let back: SharedState = serde_json::from_str(&json).expect("round-trips");
    assert_eq!(back, s);
    assert_eq!(back.executed_offset(), 0);
}

#[cfg(feature = "serde")]
#[test]
#[should_panic(expected = "truncated")]
fn a_truncated_state_refuses_to_serialize() {
    // The discarded prefix is gone, so there is no honest wire form; emitting
    // the tail alone would silently ship a short history.
    let mut s = state(&[1, 2, 3]);
    s.forget_committed_prefix(1);
    let _ = serde_json::to_string(&s);
}

// --- sn hybrid: one map on the surface, two lanes underneath ---

const BASE: u32 = protocol::compact::AUTO_CLIENT_BASE;

#[test]
fn sn_accessors_behave_like_one_map_across_both_id_namespaces() {
    let mut s = SharedState::default();
    s.sn_insert(7, 2);
    s.sn_insert(BASE, 1);
    s.sn_insert(BASE + 5, 9);
    s.sn_insert(BASE, 3);
    assert_eq!(s.sn_get(7), Some(2));
    assert_eq!(s.sn_get(BASE), Some(3), "overwrite keeps one entry");
    assert_eq!(s.sn_get(BASE + 5), Some(9));
    assert_eq!(s.sn_get(8), None);
    assert_eq!(s.sn_get(BASE + 3), None, "a gap slot is absent, not zero");
    assert_eq!(s.sn_len(), 3, "distinct present clients, gaps excluded");
    let walked: Vec<(u32, u64)> = s.sn_iter().collect();
    assert_eq!(
        walked,
        vec![(7, 2), (BASE, 3), (BASE + 5, 9)],
        "ascending client id, manual namespace first"
    );
}

#[test]
fn equal_sn_content_is_equal_regardless_of_lane_shape() {
    let a = SharedState::from_entries(vec![], BTreeMap::from([(BASE + 5, 7u64)]));
    let mut b = SharedState::default();
    b.sn_insert(BASE + 5, 7);
    assert_eq!(a, b, "same content, different construction path");
    let mut c = SharedState::default();
    c.sn_insert(BASE + 4, 7);
    assert_ne!(b, c);
    assert_eq!(a.sn_len(), 1, "gap slots never count");
}

#[cfg(feature = "serde")]
#[test]
fn the_sn_wire_form_is_byte_identical_to_the_plain_map() {
    #[derive(serde::Serialize)]
    #[serde(rename = "SharedState")]
    struct OldWire<'a> {
        executed: &'a [Entry],
        sn: &'a BTreeMap<u32, u64>,
    }
    let sn = BTreeMap::from([(3u32, 2u64), (BASE, 1), (BASE + 2, 5)]);
    let executed = vec![entry(9)];
    let s = SharedState::from_entries(executed.clone(), sn.clone());
    let old = OldWire {
        executed: &executed,
        sn: &sn,
    };
    assert_eq!(
        serde_json::to_string(&s).unwrap(),
        serde_json::to_string(&old).unwrap(),
        "hybrid must serialize exactly like the plain map did"
    );
    let back: SharedState = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
    assert_eq!(back, s, "round-trips through the map wire form");
}

#[cfg(feature = "serde")]
#[test]
fn the_sn_bincode_form_is_byte_identical_to_the_plain_map() {
    // bincode-1 trusts the serialize_map length hint, which makes the presence
    // counter wire-load-bearing — this is the byte pin that keeps it honest.
    #[derive(serde::Serialize)]
    #[serde(rename = "SharedState")]
    struct OldWire<'a> {
        executed: &'a [Entry],
        sn: &'a BTreeMap<u32, u64>,
    }
    for sn in [
        BTreeMap::new(),
        BTreeMap::from([(3u32, 2u64)]),
        BTreeMap::from([(BASE, 1u64), (BASE + 2, 5)]),
        BTreeMap::from([(3u32, 2u64), (BASE, 1), (BASE + 2, 5)]),
    ] {
        let executed = vec![entry(9)];
        let s = SharedState::from_entries(executed.clone(), sn.clone());
        let old = OldWire {
            executed: &executed,
            sn: &sn,
        };
        assert_eq!(
            bincode::serialize(&s).unwrap(),
            bincode::serialize(&old).unwrap(),
            "bincode bytes must not move for sn = {sn:?}"
        );
    }
}

#[test]
#[should_panic(expected = "absence in the auto lane")]
fn an_auto_lane_zero_fails_loudly_rather_than_corrupting_the_wire() {
    let mut s = SharedState::default();
    s.sn_insert(BASE + 1, 0);
}

#[cfg(feature = "serde")]
#[test]
fn crafted_auto_lane_zero_bytes_are_a_decode_error_not_a_panic() {
    let json = format!(r#"{{"executed":[],"sn":{{"{}":0}}}}"#, BASE + 1);
    let err = serde_json::from_str::<SharedState>(&json).unwrap_err();
    assert!(
        err.to_string().contains("absence"),
        "boundary maps the bad pair to an error: {err}"
    );
}
