//! The merge's entry-touch decomposition, pinned on a hand-counted merge.
//!
//! Round 22's lever order turns on which term of the merge dominates, and the
//! sampling profiler cannot answer it: `from_carried` inlines into
//! `step_chosen` and owns zero samples of its own. These counters are the
//! denominator instead, so they have to be exact rather than indicative.
//!
//! ONE test in this file on purpose: the counters are process-wide, and a
//! second `#[test]` in the same binary would race this one's deltas.

use protocol::Config;
use protocol::chunked::ChunkSeq;
use protocol::compact::{ClientCommand, CompactNode, CompactReply, Entry, Timed};
use protocol::merge::{merge_shape_counts, merge_touch_counts};
use std::sync::Arc;

/// Round 0 with this commit delay ages nothing out, so no drain rebases the
/// index and the merge is the whole of what runs.
const NO_AGING: u64 = 1_000;

fn timed(client: u32, round: u64) -> Timed {
    Timed {
        entry: Entry::Cmd(ClientCommand {
            client,
            sn: 1,
            op: 1,
        }),
        round,
    }
}

#[test]
fn the_merge_counts_every_entry_it_touches() {
    let (seed0, gather0, probes0, folds0) = merge_touch_counts();
    let (merges0, chosen0, cp0, alloc0) = merge_shape_counts();

    // Three logs sharing a one-entry prefix, each with its own one-entry tail:
    // cp = 1, every tail is fresh, and nothing is appended.
    let logs = [
        vec![timed(0, 0), timed(1, 0)],
        vec![timed(0, 0), timed(2, 0)],
        vec![timed(0, 0), timed(3, 0)],
    ];
    let replies: Vec<CompactReply> = logs
        .iter()
        .map(|l| CompactReply::from_log(Arc::new(ChunkSeq::from_vec(l.clone())), None))
        .collect();
    let cfg = Config::new(3, 3).expect("k = ell is legal for odd ell > 1");
    let mut node = CompactNode::new(cfg, NO_AGING);
    node.step_chosen(&replies, &[], 0, Some(&[0, 1, 2]));

    let merged = node.log_entries().expect("ell replies never go ⊥");
    assert_eq!(
        merged.len(),
        4,
        "the shared entry plus three distinct tails"
    );

    let (seed, gather, probes, folds) = merge_touch_counts();
    assert_eq!(seed - seed0, 2, "the median is copied whole: two entries");
    assert_eq!(
        gather - gather0,
        2,
        "`from_carried` re-gathers the median's own entries, one per position"
    );
    assert_eq!(
        probes - probes0,
        3,
        "one probe per chosen log's tail past the common prefix, none below it"
    );
    assert_eq!(
        folds - folds0,
        4,
        "the fold writes the median's two slots and the two fresh ones"
    );

    let (merges, chosen, cp, alloc) = merge_shape_counts();
    assert_eq!(merges - merges0, 1, "one merge took the copy path");
    assert_eq!(chosen - chosen0, 6, "three chosen logs of two entries each");
    assert_eq!(cp - cp0, 1, "the shared entry, counted once per merge");
    assert_eq!(alloc - alloc0, 4, "the merged log the round allocated");
}
