//! The proof obligation a non-contiguous executed sequence has to discharge,
//! pinned BEFORE the representation exists.
//!
//! A chunked sequence cannot hand serde a `&[Entry]`. What it can do is emit the
//! same data-model events in the same order, and the claim this file makes is
//! that doing so produces byte-identical output in every enabled format — so a
//! chunk boundary is invisible on the wire whatever the chunk capacity is. The
//! tests run against today's contiguous representation, which is the point: the
//! property is banked green now, so a failure after the representation lands is
//! unambiguously the representation's.
#![cfg(feature = "serde")]

use protocol::compact::{ClientCommand, Entry};
use serde::Serialize;
use serde::ser::{SerializeSeq, Serializer};

/// A sequence emitted from segments — what a chunk walker is, minus the chunks.
/// The length is known up front and passed to `serialize_seq`.
struct Segmented<'a>(&'a [&'a [Entry]]);

impl Serialize for Segmented<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let total: usize = self.0.iter().map(|c| c.len()).sum();
        let mut seq = serializer.serialize_seq(Some(total))?;
        for chunk in self.0 {
            for entry in *chunk {
                seq.serialize_element(entry)?;
            }
        }
        seq.end()
    }
}

/// The same walker with the length withheld. Kept as a NEGATIVE fixture: the
/// self-describing format cannot tell the two apart, and the wire format can.
struct SegmentedNoLen<'a>(&'a [&'a [Entry]]);

impl Serialize for SegmentedNoLen<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(None)?;
        for chunk in self.0 {
            for entry in *chunk {
                seq.serialize_element(entry)?;
            }
        }
        seq.end()
    }
}

fn corpus() -> Vec<Entry> {
    let mut x: u64 = 0x5eed;
    let mut rand = move || {
        x = x
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        x >> 33
    };
    (0..37)
        .map(|_| match rand() % 3 {
            0 => Entry::Cmd(ClientCommand {
                client: (rand() % 5) as u32,
                sn: rand() % 4,
                op: rand() % 9,
            }),
            1 => Entry::Null {
                client: (rand() % 5) as u32,
                sn: rand() % 4,
            },
            _ => Entry::Nop(rand() % 7),
        })
        .collect()
}

/// Every segmentation at a fixed stride, which is exactly what a chunk capacity
/// constant K produces, plus the degenerate ones.
fn segmentations(entries: &[Entry]) -> Vec<Vec<&[Entry]>> {
    let mut out = vec![vec![entries], entries.chunks(1).collect()];
    for k in [2usize, 3, 5, 8, 13, 32, 64] {
        out.push(entries.chunks(k).collect());
    }
    // An empty leading segment: a front-drop can leave one, and a walker that
    // skipped empties rather than emitting nothing for them would still pass a
    // stride-only sweep.
    let mut with_empty: Vec<&[Entry]> = vec![&entries[..0]];
    with_empty.extend(entries.chunks(7));
    out.push(with_empty);
    out
}

#[test]
fn a_chunk_walk_serialises_to_the_bytes_the_flat_slice_does() {
    let entries = corpus();
    let flat_bincode = bincode::serialize(&entries).expect("flat bincode");
    let flat_json = serde_json::to_vec(&entries).expect("flat json");

    for (i, segments) in segmentations(&entries).iter().enumerate() {
        let walker = Segmented(segments);
        assert_eq!(
            bincode::serialize(&walker).expect("segmented bincode"),
            flat_bincode,
            "segmentation {i} ({} segments) moved the wire bytes",
            segments.len()
        );
        assert_eq!(
            serde_json::to_vec(&walker).expect("segmented json"),
            flat_json,
            "segmentation {i} ({} segments) moved the json bytes",
            segments.len()
        );
    }
}

/// `serialize_seq(Some(len))` is load-bearing and this is what proves it.
/// bincode-1 writes the element count from the hint and hard-fails without one,
/// while serde_json emits no count either way — so a `None` walker passes a
/// JSON-only differential and still breaks every state-bearing frame.
#[test]
fn withholding_the_length_passes_json_and_breaks_the_wire() {
    let entries = corpus();
    let segments: Vec<&[Entry]> = entries.chunks(8).collect();

    assert_eq!(
        serde_json::to_vec(&SegmentedNoLen(&segments)).expect("json tolerates no length"),
        serde_json::to_vec(&entries).expect("flat json"),
        "the self-describing format cannot tell the two apart — which is the trap"
    );
    assert!(
        bincode::serialize(&SegmentedNoLen(&segments)).is_err(),
        "bincode must refuse a length-less sequence; if this ever passes, the \
         wire-format argument for Some(len) has silently stopped holding"
    );
}
