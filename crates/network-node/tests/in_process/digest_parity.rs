//! The executed-prefix digest must be ONE encoding, not two. `crates/network-node`
//! and `crates/sim` hash the same `Entry` sequence to the same u64, or the
//! sim-vs-real comparison compares incomparable numbers.

use protocol::compact::{ClientCommand, Entry};

fn cmd(client: u32, sn: u64, op: u64) -> Entry {
    Entry::Cmd(ClientCommand { client, sn, op })
}

fn cases() -> Vec<(&'static str, Vec<Entry>)> {
    vec![
        ("empty", vec![]),
        ("cmd", vec![cmd(1, 1, 7)]),
        ("null", vec![Entry::Null { client: 1, sn: 1 }]),
        ("nop", vec![Entry::Nop(5)]),
        (
            "mixed",
            vec![
                cmd(1, 1, 7),
                Entry::Null { client: 2, sn: 1 },
                Entry::Nop(9),
            ],
        ),
    ]
}

#[test]
fn node_and_sim_hash_every_entry_variant_identically() {
    for (name, entries) in cases() {
        assert_eq!(
            network_node::digest::fnv64(&entries),
            sim::digest::fnv64(&entries),
            "{name}: node and sim disagree on the executed-prefix digest"
        );
    }
}

#[test]
fn folding_entry_by_entry_matches_hashing_the_whole_slice() {
    for (name, entries) in cases() {
        let folded = entries
            .iter()
            .fold(sim::digest::FNV_BASIS, sim::digest::fold_entry);
        assert_eq!(
            folded,
            sim::digest::fnv64(&entries),
            "{name}: incremental fold diverges from the slice digest"
        );
    }
}
