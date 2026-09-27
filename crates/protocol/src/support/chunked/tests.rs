use super::*;

fn seq(range: std::ops::Range<u64>) -> ChunkSeq<u64> {
    ChunkSeq::from_vec(range.collect())
}

#[test]
fn every_construction_path_produces_the_same_extents() {
    let n = CHUNK as u64 * 3 + 7;
    let bulk = seq(0..n);

    let mut incremental = ChunkSeq::new();
    for i in 0..n {
        incremental.push(i);
    }

    let mut adopted = ChunkSeq::new();
    adopted.adopt(&seq(0..CHUNK as u64 * 2));
    adopted.extend(CHUNK as u64 * 2..n);

    for (label, got) in [("incremental", &incremental), ("adopted", &adopted)] {
        assert_eq!(got.sealed.len(), bulk.sealed.len(), "{label}: sealed count");
        assert_eq!(got.base_chunk, bulk.base_chunk, "{label}: base");
        assert_eq!(got.tail.len(), bulk.tail.len(), "{label}: tail");
        assert_eq!(got, &bulk, "{label}: content");
    }

    let cut = CHUNK as u64 + 3;
    let mut a = bulk.clone();
    let mut b = incremental.clone();
    let mut c = adopted.clone();
    for s in [&mut a, &mut b, &mut c] {
        s.drop_below(cut);
    }
    for (label, got) in [("incremental", &b), ("adopted", &c)] {
        assert_eq!(
            got.sealed.len(),
            a.sealed.len(),
            "{label}: sealed after drop"
        );
        assert_eq!(got.base_chunk, a.base_chunk, "{label}: base after drop");
        assert_eq!(got, &a, "{label}: content after drop");
    }
}

#[test]
fn a_front_drop_releases_whole_chunks_and_keeps_the_remainder() {
    let mut s = seq(0..CHUNK as u64 * 4);
    let held_before = s.retained_len();
    s.drop_below(CHUNK as u64 * 2 + 5);
    assert_eq!(s.start(), CHUNK as u64 * 2 + 5);
    assert_eq!(s.len(), CHUNK * 2 - 5);
    assert_eq!(s.retained_len(), held_before - 2 * CHUNK);
    assert_eq!(s.get(CHUNK as u64 * 2 + 5), Some(&(CHUNK as u64 * 2 + 5)));
    assert_eq!(s.iter().next(), Some(&(CHUNK as u64 * 2 + 5)));
}

#[test]
#[should_panic(expected = "below the dropped prefix")]
fn reading_below_the_line_panics_rather_than_guessing() {
    let mut s = seq(0..CHUNK as u64 * 2);
    s.drop_below(CHUNK as u64);
    let _ = s.get(CHUNK as u64 - 1);
}

#[test]
fn iteration_matches_the_flat_sequence_from_every_position() {
    let n = CHUNK as u64 * 3 + 11;
    let flat: Vec<u64> = (0..n).collect();
    let s = seq(0..n);
    for from in 0..=n + 2 {
        let want: Vec<u64> = flat[(from.min(n) as usize)..].to_vec();
        let got: Vec<u64> = s.iter_from(from).copied().collect();
        assert_eq!(got, want, "from {from}");
    }

    let cut = CHUNK as u64 + 9;
    let mut dropped = s.clone();
    dropped.drop_below(cut);
    for from in 0..=n {
        let lo = from.max(cut).min(n) as usize;
        let want: Vec<u64> = flat[lo..].to_vec();
        let got: Vec<u64> = dropped.iter_from(from).copied().collect();
        assert_eq!(got, want, "dropped, from {from}");
    }
}

#[test]
fn equality_ignores_how_much_each_side_dropped() {
    let mut long = seq(0..CHUNK as u64 * 3);
    long.drop_front(CHUNK);
    let short = ChunkSeq::from_vec((CHUNK as u64..CHUNK as u64 * 3).collect());
    assert_ne!(
        long.start(),
        short.start(),
        "the fixture needs unequal starts"
    );
    assert_eq!(long, short);
    assert_eq!(short, long);

    let mut different = short.clone();
    different.push(1234);
    assert_ne!(long, different);
}

// Intra-chunk forget: same chunk allocations, different entry at relative position 0.
#[test]
fn shared_chunks_are_claimed_only_between_logs_whose_frames_line_up() {
    let base = seq(0..CHUNK as u64 * 4);
    let mut nudged = base.clone();
    nudged.drop_front(3);

    assert!(
        Arc::ptr_eq(&base.sealed[0], &nudged.sealed[0]),
        "the fixture needs the same allocation on both sides"
    );
    assert_eq!(
        base.base_chunk, nudged.base_chunk,
        "and the same base chunk"
    );
    assert_ne!(base.start(), nudged.start(), "but a different live window");
    assert_ne!(
        base.at(0),
        nudged.at(0),
        "so relative position 0 is a different entry in each — which is the bug"
    );

    assert_eq!(
        base.shared_prefix_chunks(&nudged),
        0,
        "a chunk shared at unequal relative offsets agrees over nothing a walk can use"
    );
    assert_eq!(
        base.shared_prefix_chunks(&base.clone()),
        base.sealed.len(),
        "an aligned clone shares every sealed chunk by handle"
    );
}

#[test]
fn relative_access_tracks_the_live_window_across_a_drop() {
    let n = CHUNK as u64 * 3 + 11;
    let flat: Vec<u64> = (0..n).collect();
    let mut s = seq(0..n);
    for (rel, want) in flat.iter().enumerate() {
        assert_eq!(s.at(rel), Some(want), "rel {rel}");
    }
    assert_eq!(s.at(flat.len()), None);

    let cut = CHUNK + 9;
    s.drop_front(cut);
    assert_eq!(s.len(), flat.len() - cut);
    assert_eq!(
        s.start(),
        cut as u64,
        "the frame did not move, the window did"
    );
    for rel in 0..s.len() {
        assert_eq!(s.at(rel), Some(&flat[cut + rel]), "after drop, rel {rel}");
    }
    assert_eq!(
        s.iter_at(3).copied().collect::<Vec<_>>(),
        flat[cut + 3..].to_vec()
    );
}

#[test]
fn walk_comparisons_answer_what_the_flat_slices_answer() {
    let cases: [(Vec<u64>, Vec<u64>); 5] = [
        (
            (0..CHUNK as u64 * 2 + 5).collect(),
            (0..CHUNK as u64 * 2 + 5).collect(),
        ),
        (
            (0..CHUNK as u64 * 2 + 5).collect(),
            (0..CHUNK as u64 * 2 + 9).collect(),
        ),
        ((0..CHUNK as u64 * 3).collect(), (0..CHUNK as u64).collect()),
        (vec![1, 2, 3], vec![1, 2, 4]),
        (vec![], vec![7]),
    ];
    for (i, (a, b)) in cases.iter().enumerate() {
        let (sa, sb): (ChunkSeq<u64>, ChunkSeq<u64>) =
            (ChunkSeq::from_vec(a.clone()), ChunkSeq::from_vec(b.clone()));
        for from in 0..=a.len().min(b.len()) {
            assert_eq!(
                sa.iter_at(from).cmp(sb.iter_at(from)),
                a[from..].cmp(&b[from..]),
                "case {i}: order from {from}"
            );
        }
        let want = (0..a.len().min(b.len()))
            .take_while(|&j| a[j] == b[j])
            .count();
        let got = sa.iter().zip(sb.iter()).take_while(|(x, y)| x == y).count();
        assert_eq!(got, want, "case {i}: common prefix");
    }
}

#[test]
fn writing_through_a_shared_chunk_copies_it_first() {
    let donor = seq(0..CHUNK as u64 * 2);
    let mut successor = ChunkSeq::new();
    successor.adopt(&donor);
    assert_eq!(
        donor.chunk_ptrs().collect::<Vec<_>>(),
        successor.chunk_ptrs().collect::<Vec<_>>(),
        "the fixture must start out sharing"
    );

    *successor.make_mut_at(3) = 999;
    assert_eq!(successor.at(3), Some(&999));
    assert_eq!(donor.at(3), Some(&3), "the donor must be untouched");
    assert_ne!(
        donor.chunk_ptrs().next(),
        successor.chunk_ptrs().next(),
        "the written chunk must have been copied out of the sharing"
    );
    assert_eq!(
        donor.chunk_ptrs().nth(1),
        successor.chunk_ptrs().nth(1),
        "an untouched chunk must still be shared"
    );
}

#[test]
fn writing_an_unshared_entry_touches_no_allocation() {
    let mut s = seq(0..CHUNK as u64 * 2 + 3);
    let before: Vec<_> = s.chunk_ptrs().collect();
    *s.make_mut_at(1) = 42;
    *s.make_mut_at(CHUNK * 2 + 1) = 43;
    assert_eq!(s.at(1), Some(&42));
    assert_eq!(s.at(CHUNK * 2 + 1), Some(&43));
    assert_eq!(
        before,
        s.chunk_ptrs().collect::<Vec<_>>(),
        "a uniquely-held chunk is written in place"
    );
}

#[test]
fn adopting_shares_the_sealed_chunks_rather_than_copying_them() {
    let donor = seq(0..CHUNK as u64 * 3 + 5);
    let mut successor = ChunkSeq::new();
    successor.adopt(&donor);
    successor.push(9999);

    let donor_ptrs: Vec<_> = donor.chunk_ptrs().collect();
    let succ_ptrs: Vec<_> = successor.chunk_ptrs().collect();
    assert_eq!(
        donor_ptrs, succ_ptrs,
        "the successor must hold the donor's chunk allocations, not copies"
    );
    assert_eq!(donor.len(), CHUNK * 3 + 5, "the donor is untouched");
    assert_eq!(successor.len(), CHUNK * 3 + 6);
}

#[test]
fn sealing_keys_on_length_and_not_on_batching() {
    for batch in [1usize, 2, 7, CHUNK - 1, CHUNK, CHUNK + 1, CHUNK * 2] {
        let n = CHUNK * 3 + 13;
        let mut s: ChunkSeq<u64> = ChunkSeq::new();
        let all: Vec<u64> = (0..n as u64).collect();
        for part in all.chunks(batch) {
            s.extend(part.iter().copied());
        }
        assert_eq!(s.sealed_count(), n / CHUNK, "batch {batch}: sealed count");
        assert_eq!(s.to_vec(), all, "batch {batch}: content");
    }
}
