use super::*;
use protocol::compact::ClientCommand;

fn cmd(client: u32, sn: u64, op: u64) -> Entry {
    Entry::Cmd(ClientCommand { client, sn, op })
}

#[test]
fn the_empty_history_hashes_to_the_offset_basis() {
    assert_eq!(hash_hex(&[]), "cbf29ce484222325");
}

#[test]
fn every_field_and_the_discriminant_reach_the_digest() {
    let base = hash_hex(&[cmd(1, 1, 7)]);
    for other in [
        cmd(2, 1, 7),
        cmd(1, 2, 7),
        cmd(1, 1, 8),
        Entry::Null { client: 1, sn: 1 },
        Entry::Nop(1),
    ] {
        assert_ne!(
            base,
            hash_hex(std::slice::from_ref(&other)),
            "{other:?} collides"
        );
    }
    assert_ne!(
        hash_hex(&[cmd(1, 1, 7), cmd(2, 1, 7)]),
        hash_hex(&[cmd(2, 1, 7), cmd(1, 1, 7)]),
        "order is part of the history"
    );
}

#[test]
fn folding_entry_by_entry_matches_hashing_the_whole_slice() {
    let entries = vec![
        cmd(1, 1, 7),
        Entry::Null { client: 2, sn: 1 },
        Entry::Nop(9),
    ];
    assert_eq!(entries.iter().fold(FNV_BASIS, fold_entry), fnv64(&entries));
    assert_eq!(fnv64_iter(entries.iter()), fnv64(&entries));
}

#[test]
fn segmenting_the_entry_stream_cannot_move_the_digest() {
    let entries: Vec<Entry> = (0..29u64)
        .map(|i| match i % 3 {
            0 => cmd(i as u32 % 5, i % 4, i),
            1 => Entry::Null {
                client: i as u32 % 5,
                sn: i % 4,
            },
            _ => Entry::Nop(i),
        })
        .collect();
    let want = fnv64(&entries);

    for k in [1usize, 2, 3, 5, 8, 13, 32, 64] {
        let chunked: u64 = entries
            .chunks(k)
            .flat_map(<[Entry]>::iter)
            .fold(FNV_BASIS, fold_entry);
        assert_eq!(chunked, want, "stride {k} moved the digest");
    }

    let with_empty = std::iter::once(&entries[..0])
        .chain(entries.chunks(7))
        .flat_map(<[Entry]>::iter);
    assert_eq!(
        fnv64_iter(with_empty),
        want,
        "an empty leading segment moved the digest"
    );
}
