//! The median-rule merge's containment test (Alg 5 step 4 / Alg 6 in-window
//! merge): appending each candidate iff no entry with that `Entry` value is
//! already present, candidates taken in sampled-log order. These pin the
//! merge against the paper group's plain reference (`paper/reference.rs`), so an
//! implementation that answers containment differently — or keys it on
//! anything other than the entry — fails here.

use protocol::Config;
use protocol::chunked::ChunkSeq;
use protocol::compact::{ClientCommand, CompactNode, CompactReply, Entry, SharedState, Timed};
use protocol::log::Log;
use protocol::recovery::{Checkpoint, RState, RecoveryNode, RecoveryReply};
use rand::SeedableRng;
use rand_chacha::ChaCha12Rng;
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::reference::{self, RecoveryServer, median_merge};

/// ℓ = 3 with exactly 3 replies: the sample takes every log, and the merge
/// sorts them, so the result is independent of which order sampling drew —
/// the reference can be computed without mirroring the RNG.
const ELL: usize = 3;

fn rng() -> ChaCha12Rng {
    ChaCha12Rng::seed_from_u64(42)
}

fn cmd(client: u32, sn: u64, op: u64) -> ClientCommand {
    ClientCommand { client, sn, op }
}

fn timed(entry: Entry, round: u64) -> Timed {
    Timed { entry, round }
}

fn genesis() -> Checkpoint {
    Checkpoint {
        s: SharedState::default().into(),
        p: None,
        w: 0,
        certs: None,
    }
}

/// Deterministic pseudo-random logs: entries repeat across logs at differing
/// stamp rounds, which is what makes entry-keyed containment observable.
fn sample_logs(seed: u64) -> (Vec<Vec<Timed>>, Vec<(ClientCommand, u64)>) {
    let mut s = seed;
    let mut next = || {
        s = s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        s >> 33
    };
    let logs = (0..ELL)
        .map(|_| {
            let len = 3 + (next() % 6) as usize;
            (0..len).map(|_| gen_timed(&mut next)).collect()
        })
        .collect();
    let appends = (0..2)
        .map(|_| {
            let op = next() % 5;
            (cmd((op % 3) as u32, 1 + op % 2, op), next() % 4)
        })
        .collect();
    (logs, appends)
}

/// Nothing ages out at round 0 with this commit delay, and no conflicting
/// duplicate exists to null, so the node's log IS the merge result.
const NO_AGING: u64 = 1_000;

/// A universe in which no two commands can ever collide: the op is a function
/// of (client, sn), so Algorithm 5's duplicate-⊥ rule can never fire and the
/// compact merge's output is the bare union — which is what makes it
/// comparable to the recovery merge, whose box carries no nulling rule.
fn conflict_free_logs(seed: u64) -> (Vec<Vec<Timed>>, Vec<(ClientCommand, u64)>) {
    let mut s = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
    let mut next = || {
        s = s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        s >> 33
    };
    // A shared prefix on some cases, so the compact rule's common-prefix
    // restriction is actually exercised by the cross-mode comparison.
    let shared: Vec<Timed> = (0..next() % 4)
        .map(|_| {
            let c = (next() % 3) as u32;
            let sn = 1 + next() % 2;
            timed(Entry::Cmd(cmd(c, sn, c as u64 * 10 + sn)), next() % 3)
        })
        .collect();
    let logs = (0..ELL)
        .map(|_| {
            let mut log = shared.clone();
            for _ in 0..2 + next() % 5 {
                let c = (next() % 3) as u32;
                let sn = 1 + next() % 2;
                let entry = match next() % 4 {
                    0 => Entry::Nop(next() % 3),
                    1 => Entry::Null { client: c, sn },
                    _ => Entry::Cmd(cmd(c, sn, c as u64 * 10 + sn)),
                };
                log.push(timed(entry, next() % 4));
            }
            log
        })
        .collect();
    let appends = (0..next() % 3)
        .map(|_| {
            let c = (next() % 3) as u32;
            let sn = 1 + next() % 2;
            (cmd(c, sn, c as u64 * 10 + sn), next() % 4)
        })
        .collect();
    (logs, appends)
}

fn gen_timed(next: &mut impl FnMut() -> u64) -> Timed {
    let op = next() % 5;
    let stamp = next() % 4;
    let entry = match next() % 4 {
        0 => Entry::Nop(op),
        1 => Entry::Null {
            client: (op % 3) as u32,
            sn: 1,
        },
        _ => Entry::Cmd(cmd((op % 3) as u32, 1 + op % 2, op)),
    };
    timed(entry, stamp)
}

/// Logs over a colliding universe, in the five shared-prefix shapes the
/// restriction has to survive: 0 no shared prefix at all (the unrestricted
/// walk), 1 free, 2 every log equal (the prefix is the whole log, no union
/// work), 3 one log a strict prefix of the rest (empty suffix), 4 a prefix
/// agreeing entry-for-entry but not on stamps — the walk has to stop where the
/// lexicographic order stops, which is earlier than containment alone allows.
fn shaped_logs(seed: u64, shape: u64) -> (Vec<Vec<Timed>>, Vec<(ClientCommand, u64)>) {
    let mut s = seed.wrapping_mul(0xa076_1d64_78bd_642f) | 1;
    let mut next = || {
        s = s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        s >> 33
    };
    let p = match shape {
        0 => 0,
        4 => 1 + (next() % 6) as usize,
        _ => (next() % 7) as usize,
    };
    let prefix: Vec<Timed> = (0..p).map(|_| gen_timed(&mut next)).collect();
    let shared_suffix: Vec<Timed> = (0..next() % 5).map(|_| gen_timed(&mut next)).collect();
    let mut logs: Vec<Vec<Timed>> = (0..ELL)
        .map(|i| {
            let mut log = prefix.clone();
            match shape {
                2 => log.extend(shared_suffix.iter().cloned()),
                3 if i == 0 => {}
                _ => {
                    for _ in 0..next() % 6 {
                        log.push(gen_timed(&mut next));
                    }
                }
            }
            log
        })
        .collect();
    if shape == 4 {
        let at = (next() % p as u64) as usize;
        for (i, log) in logs.iter_mut().enumerate() {
            log[at].round = 90 + i as u64;
        }
    }
    let appends = (0..next() % 3)
        .map(|_| {
            let op = next() % 5;
            (cmd((op % 3) as u32, 1 + op % 2, op), next() % 4)
        })
        .collect();
    (logs, appends)
}

/// Where the chosen logs stop agreeing, computed straight — the coverage
/// oracle, never the differential's.
fn common_prefix(logs: &[Vec<Timed>]) -> usize {
    let shortest = logs.iter().map(Vec::len).min().unwrap_or(0);
    (0..shortest)
        .take_while(|&i| logs[1..].iter().all(|l| l[i] == logs[0][i]))
        .count()
}

/// The recovery merge restricted to the common prefix must land on the same
/// log as the full walk it replaces, entry for entry, in every prefix shape.
/// This is the differential the restriction rests on: `median_merge` walks
/// every position of every chosen log, from zero.
#[test]
fn recovery_restricted_union_matches_the_full_walk_reference() {
    let (mut with_prefix, mut mid_divergence, mut no_prefix) = (0, 0, 0);
    for case in 0..600u64 {
        let shape = case % 5;
        let (logs, appends) = shaped_logs(case, shape);
        let replies: Vec<RecoveryReply> = logs
            .iter()
            .map(|l| RecoveryReply {
                l_j: Some(Log::from_entries(l.clone())),
                c_j: genesis().into(),
                r_j: RState::NoReset,
            })
            .collect();
        let mut n = RecoveryNode::new(Config::default(), NO_AGING, false);
        n.step(&replies, &appends, 0, &mut rng());
        assert_eq!(
            n.log_entries().expect("non-⊥"),
            median_merge(&logs, &appends),
            "case {case} (shape {shape})"
        );

        let cp = common_prefix(&logs);
        let shortest = logs.iter().map(Vec::len).min().unwrap_or(0);
        with_prefix += usize::from(cp > 0);
        mid_divergence += usize::from(cp > 0 && cp < shortest);
        no_prefix += usize::from(cp == 0);
    }
    // The corpus has to contain what it claims to test: a differential whose
    // logs never share a prefix would agree with the reference no matter how
    // the restriction was drawn.
    assert!(
        with_prefix >= 200,
        "only {with_prefix} cases share a prefix"
    );
    assert!(
        mid_divergence >= 100,
        "only {mid_divergence} cases diverge mid-log"
    );
    assert!(no_prefix >= 100, "only {no_prefix} cases share no prefix");
}

fn spec_state(s: &SharedState) -> reference::State {
    reference::State {
        executed: s.untruncated().to_vec(),
        sn: s.sn_iter().collect(),
    }
}

fn spec_reply(reply: &RecoveryReply) -> reference::RecoveryReply {
    reference::RecoveryReply {
        l_j: reply.l_j.as_ref().map(|l| l.entries().to_vec()),
        c_j: reference::Checkpoint {
            s: spec_state(&reply.c_j.s),
            p: reply.c_j.p.clone(),
            w: reply.c_j.w,
        },
        r_j: reply.r_j,
    }
}

/// Replies whose checkpoints and reset states vary independently of their
/// logs, so the adoption and R paths are exercised alongside the merge.
/// Exactly ELL of them carry a log, which is what lets the reference skip
/// mirroring the subsample draw.
///
/// Half the seeds run "quiet": every reply sits at the node's own checkpoint
/// window, so nothing is adopted and the round's merge result is what
/// survives. Without that half the adoption path would overwrite the merge in
/// nearly every case and the differential would stop watching it.
fn recovery_replies(seed: u64, logs: &[Vec<Timed>]) -> Vec<RecoveryReply> {
    let mut s = seed.wrapping_add(0xdead_beef) | 1;
    let mut next = || {
        s = s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        s >> 33
    };
    let quiet = seed.is_multiple_of(2);
    let checkpoint = |next: &mut dyn FnMut() -> u64, holds_log: bool| {
        Arc::new(Checkpoint {
            s: SharedState::default().into(),
            p: (holds_log && !next().is_multiple_of(3))
                .then(|| vec![timed(Entry::Nop(next() % 5), 0)]),
            w: if quiet { 0 } else { next() % 3 },
            certs: None,
        })
    };
    let mut replies: Vec<RecoveryReply> = logs
        .iter()
        .map(|l| RecoveryReply {
            l_j: Some(Log::from_entries(l.clone())),
            c_j: checkpoint(&mut next, true),
            // Biased toward Reset: the node's R is NoReset as soon as ANY
            // reply is, so an even split would leave the all-Reset branch
            // exercised by only a handful of cases.
            r_j: if next() % 3 == 0 {
                RState::NoReset
            } else {
                RState::Reset
            },
        })
        .collect();
    for _ in 0..next() % 3 {
        replies.push(RecoveryReply {
            l_j: None,
            c_j: checkpoint(&mut next, false),
            // Biased toward Reset: the node's R is NoReset as soon as ANY
            // reply is, so an even split would leave the all-Reset branch
            // exercised by only a handful of cases.
            r_j: if next() % 3 == 0 {
                RState::NoReset
            } else {
                RState::Reset
            },
        });
    }
    replies
}

/// The whole post-step observable of the recovery node against an independent
/// re-derivation of its box — the differential that has to keep holding when
/// the merge body moves out from under it.
#[test]
fn recovery_step_matches_the_reference_observables() {
    let (mut adopted, mut merged_survived, mut reset_seen) = (0, 0, 0);
    for seed in 0..300 {
        let (logs, appends) = sample_logs(seed);
        let replies = recovery_replies(seed, &logs);
        let mut n = RecoveryNode::new(Config::default(), NO_AGING, false);
        n.step(&replies, &appends, 0, &mut rng());
        // Exactly ELL replies carry a log and the merge sorts them, so any draw order will do.
        let mut want = RecoveryServer::new(ELL, NO_AGING);
        let m: &[usize] = &[0, 1, 2];
        let spec_replies: Vec<reference::RecoveryReply> = replies.iter().map(spec_reply).collect();
        want.step(&spec_replies, &appends, Some(m));
        assert_eq!(n.log_entries(), want.l_i, "seed {seed}: log");
        assert_eq!(n.reset_state(), want.r_i, "seed {seed}: R");
        assert_eq!(n.checkpoint().p, want.c_i.p, "seed {seed}: checkpoint P");
        assert_eq!(n.checkpoint().w, want.c_i.w, "seed {seed}: checkpoint W");
        assert_eq!(
            spec_state(&n.checkpoint().s),
            want.c_i.s,
            "seed {seed}: checkpoint S"
        );
        assert_eq!(
            spec_state(n.shared_state()),
            want.s_i,
            "seed {seed}: shared state"
        );

        adopted += usize::from(want.c_i.w > 0);
        merged_survived += usize::from(want.c_i.w == 0 && want.l_i.is_some());
        reset_seen += usize::from(want.r_i == RState::Reset);
    }
    // The corpus has to contain what it claims to test: without rounds in
    // which the merge result SURVIVES adoption, the differential would agree
    // no matter how the merge was drawn.
    assert!(adopted >= 50, "only {adopted} cases adopt a checkpoint");
    assert!(
        merged_survived >= 50,
        "only {merged_survived} cases keep the merge result"
    );
    assert!(reset_seen >= 20, "only {reset_seen} cases end in Reset");
}

/// The two rules run the same merge on the same chosen logs: below the
/// mode-only layers — compact's nulling and drain, recovery's adoption — the
/// union they compute is one function, and this is the pin that says so.
#[test]
fn compact_and_recovery_agree_on_the_merge() {
    let mut with_prefix = 0;
    for seed in 0..300 {
        let (logs, appends) = conflict_free_logs(seed);
        let mut c = CompactNode::new(Config::default(), NO_AGING);
        c.step(
            &logs
                .iter()
                .map(|l| CompactReply::from_log(ChunkSeq::shared(l.clone()), None))
                .collect::<Vec<_>>(),
            &appends,
            0,
            &mut rng(),
        );
        let mut r = RecoveryNode::new(Config::default(), NO_AGING, false);
        r.step(
            &logs
                .iter()
                .map(|l| RecoveryReply {
                    l_j: Some(Log::from_entries(l.clone())),
                    c_j: genesis().into(),
                    r_j: RState::NoReset,
                })
                .collect::<Vec<_>>(),
            &appends,
            0,
            &mut rng(),
        );
        let want = median_merge(&logs, &appends);
        assert_eq!(
            c.log_entries().expect("non-⊥"),
            want,
            "seed {seed}: compact"
        );
        assert_eq!(
            r.log_entries().expect("non-⊥"),
            want,
            "seed {seed}: recovery"
        );

        let shortest = logs.iter().map(Vec::len).min().unwrap_or(0);
        with_prefix += usize::from(
            (0..shortest)
                .take_while(|&i| logs[1..].iter().all(|l| l[i] == logs[0][i]))
                .count()
                > 0,
        );
    }
    // A corpus whose logs never share a prefix would agree however compact
    // drew its common-prefix restriction.
    assert!(
        with_prefix >= 100,
        "only {with_prefix} cases share a prefix"
    );
}

/// The sharpest trap in single-sourcing the merge: ordering the chosen logs
/// destroys the draw order, and Algorithm 5 step 4's state adoption names the
/// first DRAWN reply carrying a state — not the first in content order. Here
/// the two disagree, so an implementation that adopted from the sorted
/// sequence would take the wrong state.
#[test]
fn compact_adopts_state_from_the_first_drawn_reply_not_the_first_sorted() {
    let carried = |op: u64| {
        Some(Arc::new(SharedState::from_entries(
            vec![Entry::Cmd(cmd(9, 1, op))],
            BTreeMap::new(),
        )))
    };
    // Content order is high < mid < low by their single entry; draw order is
    // the order below. The first DRAWN state carrier is `high` (op 111); the
    // first one in CONTENT order would be `high` too unless we place the
    // states so they differ — so `low` carries none and `mid` carries op 222.
    let high = CompactReply::from_log(
        ChunkSeq::shared(vec![timed(Entry::Nop(2), 0)]),
        carried(111),
    );
    let mid = CompactReply::from_log(
        ChunkSeq::shared(vec![timed(Entry::Nop(1), 0)]),
        carried(222),
    );
    let low = CompactReply::from_log(ChunkSeq::shared(vec![timed(Entry::Nop(0), 0)]), None);
    // Content order sorts to [low, mid, high], so the first state carrier in
    // that order is `mid`. Draw order puts `high` first.
    let replies = vec![high, mid, low];

    // The node must be ⊥ for b_i to be set, which is the only branch that
    // adopts a peer's state at all.
    let mut n = CompactNode::new(Config::default(), NO_AGING);
    n.step_chosen(&[], &[], 0, None);
    assert!(n.b_i(), "the node has to enter the step at ⊥");
    n.step_chosen(&replies, &[], 0, Some(&[0, 1, 2]));
    assert_eq!(
        n.shared_state().untruncated(),
        [Entry::Cmd(cmd(9, 1, 111))],
        "state came from the first DRAWN carrier, not the first sorted one"
    );
}

#[test]
fn compact_merge_matches_the_reference_double_loop() {
    for seed in 0..200 {
        let (logs, appends) = sample_logs(seed);
        let replies: Vec<CompactReply> = logs
            .iter()
            .map(|l| CompactReply::from_log(ChunkSeq::shared(l.clone()), None))
            .collect();
        let mut n = CompactNode::new(Config::default(), NO_AGING);
        n.step(&replies, &appends, 0, &mut rng());
        assert_eq!(
            n.log_entries().expect("non-⊥"),
            median_merge(&logs, &appends),
            "seed {seed}"
        );
    }
}

#[test]
fn recovery_merge_matches_the_reference_double_loop() {
    for seed in 0..200 {
        let (logs, appends) = sample_logs(seed);
        let replies: Vec<RecoveryReply> = logs
            .iter()
            .map(|l| RecoveryReply {
                l_j: Some(Log::from_entries(l.clone())),
                c_j: genesis().into(),
                r_j: RState::NoReset,
            })
            .collect();
        let mut n = RecoveryNode::new(Config::default(), NO_AGING, false);
        n.step(&replies, &appends, 0, &mut rng());
        assert_eq!(
            n.log_entries().expect("non-⊥"),
            median_merge(&logs, &appends),
            "seed {seed}"
        );
    }
}

/// A command already carried by the median, re-offered by another log with a
/// different stamp round, must NOT be appended a second time — containment is
/// keyed on the entry, never on (entry, round). This is the case that fails
/// if the membership index keys on `Timed`.
#[test]
fn a_restamped_entry_does_not_duplicate() {
    let x = Entry::Cmd(cmd(1, 1, 77));
    let logs = vec![
        vec![timed(x.clone(), 0)],
        vec![timed(x.clone(), 5)],
        vec![timed(x.clone(), 9)],
    ];
    let replies: Vec<CompactReply> = logs
        .iter()
        .map(|l| CompactReply::from_log(ChunkSeq::shared(l.clone()), None))
        .collect();
    let mut n = CompactNode::new(Config::default(), NO_AGING);
    n.step(&replies, &[], 0, &mut rng());
    let out = n.log_entries().expect("non-⊥");
    assert_eq!(out.len(), 1, "one entry, whatever its stamp: {out:?}");
    assert_eq!(out, median_merge(&logs, &[]));
}

/// The median is picked by log CONTENT, lexicographically — not by the order
/// replies arrived. Pinned because the merge seeds from the median, so a
/// comparator that ever compared anything else (a wrapper's `Ord`, a pointer)
/// would silently change every downstream number.
#[test]
fn the_median_is_the_content_lexicographic_middle() {
    let low = vec![timed(Entry::Nop(0), 0)];
    let mid = vec![timed(Entry::Nop(1), 0)];
    let high = vec![timed(Entry::Nop(2), 0)];
    assert!(low < mid && mid < high, "fixture ordering");
    // Every arrival order must yield the same merge, seeded by `mid`.
    for arrival in [
        [&low, &mid, &high],
        [&high, &mid, &low],
        [&mid, &high, &low],
        [&high, &low, &mid],
    ] {
        let logs: Vec<Vec<Timed>> = arrival.iter().map(|l| (*l).clone()).collect();
        let replies: Vec<CompactReply> = logs
            .iter()
            .map(|l| CompactReply::from_log(ChunkSeq::shared(l.clone()), None))
            .collect();
        let mut n = CompactNode::new(Config::default(), NO_AGING);
        n.step(&replies, &[], 0, &mut rng());
        let out = n.log_entries().expect("non-⊥");
        assert_eq!(out[0], timed(Entry::Nop(1), 0), "median seeds the merge");
        assert_eq!(out, median_merge(&logs, &[]));
    }
}

/// ⊥ reply logs are filtered before the merge and never enter containment:
/// with fewer than ℓ log-bearing replies the node goes ⊥ itself, and with
/// enough of them the ⊥ ones are simply absent from the merge.
#[test]
fn bot_reply_logs_stay_out_of_the_merge() {
    let held = vec![timed(Entry::Nop(7), 0)];
    let bot = RecoveryReply {
        l_j: None,
        c_j: genesis().into(),
        r_j: RState::NoReset,
    };
    let carrying = RecoveryReply {
        l_j: Some(Log::from_entries(held.clone())),
        c_j: genesis().into(),
        r_j: RState::NoReset,
    };

    // Two log-bearing replies out of three: below ℓ, so the log goes ⊥.
    let mut n = RecoveryNode::new(Config::default(), NO_AGING, false);
    n.step(
        &[carrying.clone(), carrying.clone(), bot.clone()],
        &[],
        0,
        &mut rng(),
    );
    assert!(n.log_entries().is_none(), "under ℓ log replies → ⊥");

    // Enough log-bearing replies: the ⊥ ones contribute nothing.
    let mut n = RecoveryNode::new(Config::default(), NO_AGING, false);
    n.step(
        &[carrying.clone(), carrying.clone(), carrying, bot],
        &[],
        0,
        &mut rng(),
    );
    assert_eq!(n.log_entries().expect("non-⊥"), held);
}
