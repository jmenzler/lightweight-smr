//! Framing edges beyond the acceptance roundtrips: per-entry payload padding
//! (the ∝ entry-size knob), truncation, garbage, declared-length abuse.

use network_node::wire::{Msg, ReplyPayload, decode, encode};
use protocol::compact::{ClientCommand, Entry, Timed};

const MAX: u32 = 64 * 1024 * 1024;

#[test]
fn padding_scales_with_entry_count_not_per_frame() {
    let log3 = Msg::PullReply {
        round: 1,
        from: 0,
        slot: 0,
        payload: ReplyPayload::Log(vec![1, 2, 3]),
    };
    let log6 = Msg::PullReply {
        round: 1,
        from: 0,
        slot: 0,
        payload: ReplyPayload::Log(vec![1, 2, 3, 4, 5, 6]),
    };
    let d3 = encode(&log3, MAX, 100).unwrap().len() - encode(&log3, MAX, 0).unwrap().len();
    let d6 = encode(&log6, MAX, 100).unwrap().len() - encode(&log6, MAX, 0).unwrap().len();
    assert_eq!(d3, 300, "3 entries × 100 pad bytes");
    assert_eq!(d6, 600, "6 entries × 100 pad bytes");

    // single-command messages pad once
    let cmd = Msg::ClientCmd {
        round: 1,
        client: 1,
        sn: 1,
        op: 9,
    };
    let dc = encode(&cmd, MAX, 100).unwrap().len() - encode(&cmd, MAX, 0).unwrap().len();
    assert_eq!(dc, 100);

    // control messages never pad
    let go = Msg::RoundGo { round: 5 };
    assert_eq!(
        encode(&go, MAX, 100).unwrap().len(),
        encode(&go, MAX, 0).unwrap().len()
    );
}

#[test]
fn padded_frames_still_roundtrip() {
    let m = Msg::PullReply {
        round: 2,
        from: 4,
        slot: 1,
        payload: ReplyPayload::Compact {
            log: vec![Timed {
                entry: Entry::Cmd(ClientCommand {
                    client: 1,
                    sn: 1,
                    op: 5,
                }),
                round: 1,
            }],
            state: None,
        },
    };
    let frame = encode(&m, MAX, 64).unwrap();
    assert_eq!(decode(&frame, MAX).unwrap(), m);
}

#[test]
fn truncated_and_garbage_frames_error_cleanly() {
    let m = Msg::RoundDone { round: 3, from: 1 };
    let frame = encode(&m, MAX, 0).unwrap();
    assert!(
        decode(&frame[..frame.len() - 1], MAX).is_err(),
        "truncated body"
    );
    assert!(
        decode(&frame[..2], MAX).is_err(),
        "shorter than the length prefix"
    );
    assert!(decode(&[], MAX).is_err(), "empty");

    let mut garbage = frame.clone();
    let last = garbage.len() - 1;
    garbage[last] ^= 0xFF;
    // Flipping the tail of a fixint enum body either decodes to junk fields
    // (caught by equality elsewhere) or errors — it must never panic.
    let _ = decode(&garbage, MAX);
}

#[test]
fn oversized_declared_length_is_rejected_before_allocation() {
    // Hand-craft a frame whose u32 prefix claims 1GiB.
    let mut frame = Vec::new();
    frame.extend_from_slice(&(1_073_741_824u32).to_le_bytes());
    frame.extend_from_slice(&[0u8; 16]);
    assert!(decode(&frame, MAX).is_err());
}

/// The bincode-1 fixint body is NOT self-describing, so any field added to
/// `SharedState` silently lengthens every state-bearing reply and moves the
/// byte-cost evidence the sim-validation experiment compares against. The
/// state's storage representation must therefore never reach the wire: this
/// pins the encoded body length of one fixed state-bearing PullReply.
#[test]
fn a_state_bearing_reply_keeps_its_encoded_length() {
    let entries: Vec<Entry> = (0..4)
        .map(|op| {
            Entry::Cmd(ClientCommand {
                client: 7,
                sn: op + 1,
                op,
            })
        })
        .collect();
    let m = Msg::PullReply {
        round: 3,
        from: 1,
        slot: 0,
        payload: ReplyPayload::Compact {
            log: vec![Timed {
                entry: entries[0].clone(),
                round: 2,
            }],
            state: Some(protocol::compact::SharedState::from_entries(
                entries,
                std::collections::BTreeMap::from([(7u32, 4u64)]),
            )),
        },
    };
    let frame = encode(&m, MAX, 0).expect("encodes");
    assert_eq!(
        frame.len(),
        191,
        "4-byte length prefix + a 187-byte body: 4 executed entries, one sn \
         pair, one log entry. Regenerate only as a deliberate wire change."
    );
}
