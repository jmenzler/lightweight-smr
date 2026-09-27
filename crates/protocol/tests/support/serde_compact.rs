//! Wire types must round-trip under the serde feature — the networked node
//! ships them inside PullReply payloads.
#![cfg(feature = "serde")]

use protocol::compact::{ClientCommand, Entry, SharedState, Timed};
use std::collections::BTreeMap;

#[test]
fn compact_wire_types_roundtrip_json() {
    let cmd = ClientCommand {
        client: 3,
        sn: 2,
        op: 77,
    };
    let entries = vec![
        Entry::Cmd(cmd),
        Entry::Null { client: 4, sn: 1 },
        Entry::Nop(0),
    ];
    let timed = Timed {
        entry: entries[0].clone(),
        round: 9,
    };
    let state = SharedState::from_entries(entries.clone(), BTreeMap::from([(3u32, 2u64), (4, 1)]));

    let cmd2: ClientCommand = serde_json::from_str(&serde_json::to_string(&cmd).unwrap()).unwrap();
    assert_eq!(cmd2, cmd);
    let entries2: Vec<Entry> =
        serde_json::from_str(&serde_json::to_string(&entries).unwrap()).unwrap();
    assert_eq!(entries2, entries);
    let timed2: Timed = serde_json::from_str(&serde_json::to_string(&timed).unwrap()).unwrap();
    assert_eq!(timed2, timed);
    let state2: SharedState =
        serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
    assert_eq!(state2, state);
}
