//! Recovery-protocol (Algorithm 6) round step; the between-window pass runs after (f), RNG-free.

use crate::smr::engine::stages::{amplify, apply_all};
use crate::smr::observe::canon_cursor::{CanonCursor, canon_lookup};
use crate::smr::observe::canonical::{CanonicalOrder, Executed, ForkSignature};
use crate::smr::observe::entry_index::EntryIndex;
use crate::smr::observe::log_index::{LogCounts, LogIndex, log_window_pos};
use crate::smr::observe::round_metrics::smr_round_metrics;
use crate::smr::tracker::{
    CommandTracker, advance_inert_prefix, latch_coverage, pos_stats, spreading_list,
};
use crate::smr::*;
use protocol::compact::{Entry, Timed, Triage};
use protocol::log::Log;
use protocol::recovery::{Checkpoint, PrefixMismatch, RState, RecoveryNode, RecoveryReply};
use protocol::shared_state::ExecutedView;
use rand::Rng;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

// Counted unconditionally: a gated counter's zero would read as "no volume" in a release binary.
static KEY_PROBES: AtomicU64 = AtomicU64::new(0);
static PRE_CALLS: AtomicU64 = AtomicU64::new(0);
static PRE_ENTRIES: AtomicU64 = AtomicU64::new(0);
static RECHECK_CALLS: AtomicU64 = AtomicU64::new(0);
static RECHECK_ENTRIES: AtomicU64 = AtomicU64::new(0);

/// (map probes, pre calls, pre entries, recheck calls, recheck entries) since process start.
pub fn boundary_key_counts() -> (u64, u64, u64, u64, u64) {
    (
        KEY_PROBES.load(Ordering::Relaxed),
        PRE_CALLS.load(Ordering::Relaxed),
        PRE_ENTRIES.load(Ordering::Relaxed),
        RECHECK_CALLS.load(Ordering::Relaxed),
        RECHECK_ENTRIES.load(Ordering::Relaxed),
    )
}

fn counted_pre(node: &RecoveryNode, round: u64, recheck: bool) -> Option<Vec<Timed>> {
    let pre = node.would_carry_pre(round);
    let entries = pre.as_ref().map_or(0, Vec::len) as u64;
    if recheck {
        RECHECK_CALLS.fetch_add(1, Ordering::Relaxed);
        RECHECK_ENTRIES.fetch_add(entries, Ordering::Relaxed);
    }
    PRE_CALLS.fetch_add(1, Ordering::Relaxed);
    PRE_ENTRIES.fetch_add(entries, Ordering::Relaxed);
    pre
}

#[cfg(test)]
pub(in crate::smr) struct RecBoundaryNodeSnapshot {
    checkpoint: Arc<Checkpoint>,
    log: Option<Log>,
    state: Arc<protocol::compact::SharedState>,
    r: RState,
}

pub(in crate::smr) struct RecRunState {
    t_window: u64,
    resend_until_acked: bool,
    prefix_mismatch: PrefixMismatch,
    pub(in crate::smr) rows: Vec<RecRoundMetrics>,
    pub(in crate::smr) fork_ok: bool,
    pending_rollbacks: u32,
    pending_rollback_depth: u32,
    pending_cp_fork_k: u32,
    pending_max_cp_p_len: u32,
    prev_len: Vec<u32>,
    canonical_pos: EntryIndex,
    lineage: BTreeMap<u64, ForkSignature>,
    last_window: Vec<Option<u64>>,
}

impl RecRunState {
    pub(in crate::smr) fn new(
        t_window: u64,
        n: usize,
        resend_until_acked: bool,
        prefix_mismatch: PrefixMismatch,
    ) -> Self {
        RecRunState {
            t_window,
            resend_until_acked,
            prefix_mismatch,
            rows: Vec::new(),
            fork_ok: true,
            pending_rollbacks: 0,
            pending_rollback_depth: 0,
            pending_cp_fork_k: 0,
            pending_max_cp_p_len: 0,
            prev_len: vec![0; n],
            canonical_pos: EntryIndex::new(),
            lineage: BTreeMap::new(),
            last_window: vec![None; n],
        }
    }

    pub(in crate::smr) fn heap_bytes_rows(&self) -> u64 {
        (self.rows.capacity() * size_of::<RecRoundMetrics>()
            + self.prev_len.capacity() * size_of::<u32>()
            + self.last_window.capacity() * size_of::<Option<u64>>()) as u64
    }

    pub(in crate::smr) fn heap_bytes_indexes(&self) -> u64 {
        self.canonical_pos.heap_bytes()
    }

    pub(in crate::smr) fn heap_bytes_lineage(&self) -> u64 {
        ((self.lineage.len() * (size_of::<u64>() + size_of::<ForkSignature>())) as u64 * 3) / 2
    }

    fn check_growth(&mut self, node: usize, executed_len: usize, round: usize) {
        assert!(
            executed_len >= self.prev_len[node] as usize,
            "driver bug or split brain: node {node} executed sequence regressed in round {round}"
        );
        self.prev_len[node] = executed_len as u32;
    }

    fn check_lineage(&mut self, node: usize, checkpoint: &Checkpoint, canon: &CanonicalOrder) {
        // A checkpoint's window strictly rises on any change, so an unchanged window is the same checkpoint.
        let last = self.last_window[node];
        assert!(
            last.is_none_or(|w| w <= checkpoint.w),
            "node {node}: checkpoint window regressed"
        );
        if last == Some(checkpoint.w) {
            return;
        }
        self.last_window[node] = Some(checkpoint.w);
        let signature = ForkSignature::of(canon, checkpoint);
        match self.lineage.get(&checkpoint.w) {
            Some(mark) => {
                if *mark != signature {
                    self.fork_ok = false;
                }
            }
            None => {
                let broken =
                    self.lineage
                        .range(..checkpoint.w)
                        .next_back()
                        .is_some_and(|(_, prev)| {
                            !signature.extends(
                                canon,
                                checkpoint,
                                prev.executed_len(),
                                prev.executed(),
                            )
                        });
                if broken {
                    self.fork_ok = false;
                }
                self.lineage.insert(checkpoint.w, signature);
            }
        }
    }
}

// Ties go to the LAST, matching the node's `max_by_key`.
fn last_max_window(replies: &[RecoveryReply]) -> usize {
    let mut best = 0;
    for (j, rep) in replies.iter().enumerate() {
        if rep.c_j.w >= replies[best].c_j.w {
            best = j;
        }
    }
    best
}

fn recovery_client_triage(node: &RecoveryNode, command: &ClientCommand) -> Triage {
    let committed = node.checkpoint().s.sn_get(command.client);
    if committed == Some(command.sn) {
        Triage::AckCommitted
    } else if command.sn > 0
        && committed.unwrap_or(0).checked_add(1) == Some(command.sn)
        && node.wants_amplify(command)
    {
        Triage::Amplify
    } else {
        Triage::Ignore
    }
}

pub(in crate::smr) fn r_code(r: RState) -> u8 {
    match r {
        RState::NoReset => 0,
        RState::Reset => 1,
        RState::Bot => 2,
    }
}

impl SmrState {
    pub(in crate::smr) fn step_rec(&mut self, mask: &[bool]) -> SmrStepStatus {
        let n = self.n;
        let cfg = self.cfg;
        let amp = self.amp;
        let round = self.round;
        let arrivals = std::mem::take(&mut self.arrivals_pending);
        let SmrNodes::Rec(nodes) = &mut self.nodes else {
            unreachable!("dispatched on Rec");
        };
        // The node swaps, never edits, its log, so the handle IS the pre-step snapshot.
        let snap_logs: Vec<Option<Log>> =
            nodes.iter().map(|nd| nd.log_perm_arc().cloned()).collect();
        let snap_cp_windows: Vec<u64> = nodes.iter().map(|nd| nd.checkpoint().w).collect();
        #[cfg(debug_assertions)]
        let snap_cps: Vec<_> = nodes.iter().map(|nd| nd.checkpoint().clone()).collect();
        let snap_rs: Vec<RState> = nodes.iter().map(RecoveryNode::reset_state).collect();
        let useful: Vec<bool> = (0..n).map(|i| snap_logs[i].is_some() && !mask[i]).collect();
        let useful_total = useful.iter().filter(|&&u| u).count() as u32;

        // (b) client stage: resend until §6-committed, or until acked in ack mode
        let ack_mode = self.rec.as_ref().is_some_and(|r| r.resend_until_acked);
        let mut delivered: Vec<(usize, ClientCommand)> = Vec::new();
        for &ti in &self.client_order {
            let t = &mut self.trackers[ti];
            let pending = if ack_mode {
                t.committed_ack_round.is_none()
            } else {
                t.executed_round.is_none()
            };
            if pending && round >= t.injection_round {
                let target = match t.target {
                    Some(id) => id as usize,
                    None => self.rng.random_range(0..n),
                };
                #[cfg(debug_assertions)]
                assert_eq!(
                    nodes[target].checkpoint(),
                    &snap_cps[target],
                    "stage (b) read post-step state"
                );
                let outcome = if mask[target] {
                    AttemptOutcome::TargetBlocked
                } else if snap_logs[target].is_none() {
                    AttemptOutcome::TargetBot
                } else if ack_mode {
                    match recovery_client_triage(&nodes[target], &t.cc) {
                        Triage::AckCommitted => {
                            t.committed_ack_round = Some(round);
                            AttemptOutcome::AckCommitted
                        }
                        Triage::Amplify => {
                            if t.delivered_round.is_none() {
                                t.delivered_round = Some(round);
                            }
                            delivered.push((target, t.cc));
                            AttemptOutcome::Delivered
                        }
                        Triage::Ignore => AttemptOutcome::Ignored,
                    }
                } else {
                    if t.delivered_round.is_none() {
                        t.delivered_round = Some(round);
                    }
                    delivered.push((target, t.cc));
                    AttemptOutcome::Delivered
                };
                t.attempts.push(DeliveryAttempt {
                    round,
                    target: target as u32,
                    outcome,
                });
            }
        }

        // (c) amplify stage: a resend re-amplifies only if the target's log lost the command
        let partition = self.partition.as_deref();
        let appends = amplify(
            &mut self.rng,
            &mut self.trackers,
            delivered,
            |server, cc| nodes[server].wants_amplify(cc),
            mask,
            partition,
            n,
            amp,
            round,
        );

        // (d) request stage, answered by `answer`: log-repliers ⊆ recovery-repliers (Lemma 6.2)
        let mut inboxes: Vec<Vec<RecoveryReply>> = Vec::with_capacity(n);
        let mut sources: Vec<Vec<u32>> = Vec::with_capacity(n);
        for i in 0..n {
            let mut replies = Vec::with_capacity(cfg.k);
            let mut senders = Vec::with_capacity(cfg.k);
            for _ in 0..cfg.k {
                let target = self.rng.random_range(0..n);
                if !mask[i]
                    && !mask[target]
                    && same_component(partition, i, target)
                    && let Some(reply) = nodes[target].answer()
                {
                    #[cfg(debug_assertions)]
                    assert_eq!(
                        nodes[target].checkpoint(),
                        &snap_cps[target],
                        "stage (d) read post-step state"
                    );
                    replies.push(reply);
                    senders.push(target as u32);
                }
            }
            inboxes.push(replies);
            sources.push(senders);
        }

        // (e) core step: all draws first, ascending id, reading only stage-(d) inboxes; applies are RNG-free.
        let choices: Vec<Option<Vec<usize>>> = (0..n)
            .map(|i| nodes[i].draw_reply_choice(&inboxes[i], &mut self.rng))
            .collect();
        apply_all(nodes, |i, node: &mut RecoveryNode| {
            node.step_chosen(&inboxes[i], &appends[i], choices[i].as_deref());
        });

        // (f) observation: no RNG from here on
        let mut adoptions: Vec<(u32, u32)> = Vec::new();
        let mut reset_votes: Vec<(u32, u32)> = Vec::new();
        for (i, replies) in inboxes.iter().enumerate() {
            if replies.len() < cfg.ell {
                continue;
            }
            if nodes[i].checkpoint().w > snap_cp_windows[i] {
                adoptions.push((sources[i][last_max_window(replies)], i as u32));
            }
            if snap_rs[i] != RState::NoReset
                && nodes[i].reset_state() == RState::NoReset
                && let Some(j) = replies.iter().position(|rep| rep.r_j == RState::NoReset)
            {
                reset_votes.push((sources[i][j], i as u32));
            }
        }
        drop(inboxes);

        let rec = self.rec.as_mut().expect("recovery run state");
        let post_logs: Vec<Option<&protocol::chunked::ChunkSeq<Timed>>> =
            nodes.iter().map(RecoveryNode::log_seq).collect();
        let (post_index, distinct_logs) =
            LogIndex::build_with_distinct(post_logs.iter().copied(), self.observe_spread);
        let executed_lens: Vec<usize> = nodes
            .iter()
            .map(|nd| nd.shared_state().logical_len() as usize)
            .collect();
        let round_metrics = smr_round_metrics(
            &post_logs,
            &executed_lens,
            mask,
            useful_total,
            distinct_logs,
            arrivals,
        );
        self.metrics.push(round_metrics);

        // Read before this round's check extends it.
        let pos_floor = self.checker.canonical_len() as u32;

        if self.safety_ok {
            let executed: Vec<ExecutedView<'_>> = nodes
                .iter()
                .map(|nd| nd.shared_state().executed())
                .collect();
            if !self.checker.check_round(&executed, self.frontier) {
                self.safety_ok = false;
            }
        }
        let cursors_exact = self.safety_ok;
        let canonical = self.checker.canonical();
        if rec
            .canonical_pos
            .extend_unique(canonical.start(), canonical.retained())
        {
            self.exec_dup.trip_at(round);
        }
        let canon_index = &rec.canonical_pos;

        let observe_spread = self.observe_spread;
        let useful_counts = observe_spread.then(|| {
            LogCounts::build(snap_logs.iter().zip(&useful).map(|(log, &u)| {
                if u {
                    log.as_ref().map(|l| &**l.entries())
                } else {
                    None
                }
            }))
        });

        let drop_settled_spread = self.drop_settled_spread;
        let inert = |t: &CommandTracker| {
            t.executed_round.is_some_and(|r| round > r + 1)
                && t.all_logs_round.is_some()
                && t.canon_cursor.is_none()
                && (!drop_settled_spread || t.spread_points == 0)
                && (!ack_mode || t.committed_ack_round.is_some())
        };
        self.inert_prefix = advance_inert_prefix(&self.trackers, self.inert_prefix, inert);
        let live_from = self.inert_prefix;
        let mut live_floor: Option<u32> = None;
        #[cfg(debug_assertions)]
        assert!(
            self.trackers[..live_from].iter().all(inert),
            "round {round}: skipped a tracker that is not inert"
        );
        for t in self.trackers[live_from..].iter_mut() {
            if round < t.injection_round {
                continue;
            }
            let entry = Entry::Cmd(t.cc);
            if t.executed_round.is_none_or(|r| round <= r + 1) {
                t.spread_points += 1;
                crate::smr::observe::census_count::window();
                let cursor = t
                    .canon_cursor
                    .get_or_insert_with(|| CanonCursor::new(pos_floor));
                live_floor = Some(live_floor.unwrap_or(u32::MAX).min(cursor.floor()));
                if observe_spread {
                    let useful_holders = useful_counts
                        .as_ref()
                        .expect("census on implies the counts were built")
                        .count(&entry);
                    let entry_holders = post_index.entry_holders(&entry);
                    let mut visits = 0usize;
                    let hit = cursors_exact.then(|| canon_index.get(&entry)).flatten();
                    let (pos_min, pos_med, pos_max) = pos_stats(
                        nodes
                            .iter()
                            .enumerate()
                            .filter(|(_, nd)| nd.log_seq().is_some())
                            .inspect(|_| visits += 1)
                            .filter_map(|(i, nd)| {
                                let ex = nd.shared_state().executed();
                                let executed = Executed {
                                    view: ex,
                                    canonical,
                                };
                                (if cursors_exact {
                                    let found = canon_lookup(hit, ex);
                                    debug_assert_eq!(
                                        found,
                                        executed.front_scan(&entry),
                                        "node {i}: canonical position differs \
                                         from the front scan"
                                    );
                                    found
                                } else {
                                    executed.front_scan(&entry)
                                })
                                .or_else(|| {
                                    log_window_pos(
                                        entry_holders,
                                        i as u32,
                                        nd.log_seq().expect("filtered non-⊥"),
                                        &entry,
                                        ex.logical_len(),
                                    )
                                })
                            })
                            .collect(),
                    );
                    crate::smr::observe::census_count::point(visits);
                    t.spread.push(SpreadPoint {
                        round,
                        useful_holders,
                        useful_total,
                        pos_min,
                        pos_med,
                        pos_max,
                    });
                    debug_assert_eq!(
                        t.spread_points as usize,
                        t.spread.len(),
                        "op {}: the point counter drifted from the curve",
                        t.op
                    );
                }
            } else {
                t.canon_cursor = None;
                if drop_settled_spread {
                    t.shed_spread();
                    if !ack_mode || t.committed_ack_round.is_some() {
                        t.shed_attempts();
                    }
                }
            }
            latch_coverage(t, &entry, round, &post_index, &post_logs);
        }
        drop(useful_counts);
        drop(snap_logs);

        let spreading = if drop_settled_spread {
            Vec::new()
        } else {
            spreading_list(
                &self.trackers,
                live_from,
                round,
                &post_index,
                &post_logs,
                |t| t.executed_round.is_none(),
            )
        };
        drop(post_index);

        let count_r = |want: RState| nodes.iter().filter(|nd| nd.reset_state() == want).count();
        rec.rows.push(RecRoundMetrics {
            noreset: count_r(RState::NoReset) as u32,
            reset: count_r(RState::Reset) as u32,
            bot_r: count_r(RState::Bot) as u32,
            window: (round as u64 - 1) / rec.t_window,
            rollbacks: std::mem::take(&mut rec.pending_rollbacks),
            max_checkpoint_window: nodes.iter().map(|nd| nd.checkpoint().w).max().unwrap_or(0),
            rollback_depth: std::mem::take(&mut rec.pending_rollback_depth),
            cp_fork_k: std::mem::take(&mut rec.pending_cp_fork_k),
            max_cp_p_len: std::mem::take(&mut rec.pending_max_cp_p_len),
        });

        let canonical = self.checker.canonical();

        for (i, nd) in nodes.iter().enumerate() {
            debug_assert!(
                nd.reset_state() != RState::Bot || nd.log_seq().is_none(),
                "node {i}: Lemma 6.2 violated in round {round}"
            );
            // check_growth must fire first and stay a hard assert in every build profile.
            rec.check_growth(i, nd.shared_state().logical_len() as usize, round);
            rec.check_lineage(i, nd.checkpoint(), canonical);
        }

        let glances = post_logs
            .iter()
            .zip(&executed_lens)
            .zip(nodes.iter())
            .map(|((log, &ex), nd)| NodeGlance {
                log_len: log.map(|l| l.len() as u32),
                executed_len: ex as u32,
                r: Some(r_code(nd.reset_state())),
            })
            .collect();
        drop(post_logs);

        // Between-window pass: RNG-free; all-⊥ is expected mid-surge, so recovery never reads Dead.
        #[cfg(test)]
        if (round as u64).is_multiple_of(rec.t_window) {
            self.rec_boundary_snapshot = Some(
                nodes
                    .iter()
                    .map(|node| RecBoundaryNodeSnapshot {
                        checkpoint: node.checkpoint_shared(),
                        log: node.log_perm_arc().cloned(),
                        state: Arc::clone(node.state_arc()),
                        r: node.reset_state(),
                    })
                    .collect(),
            );
        }
        if (round as u64).is_multiple_of(rec.t_window)
            && rec.prefix_mismatch == PrefixMismatch::Abort
            && let Some((node, violation)) = nodes
                .iter()
                .enumerate()
                .find_map(|(node, nd)| nd.validate_boundary().err().map(|v| (node, v)))
        {
            let failure = RecoveryFailure {
                node: node as u32,
                attempted_round: round,
                completed_round: round - 1,
                observed_round: round,
                phase: RecoveryFailurePhase::BoundaryPreflight,
                violation,
            };
            let status = SmrStepStatus {
                round,
                metrics: round_metrics,
                blocked: blocked_ids(mask),
                nodes: glances,
                spreading,
                dead: None,
                failure: Some(failure.clone()),
                rec: Some(RecStepInfo {
                    adoptions,
                    reset_votes,
                }),
            };
            self.failure = Some(failure);
            self.failed_status = Some(status.clone());
            return status;
        }
        #[cfg(test)]
        if (round as u64).is_multiple_of(rec.t_window) {
            // Drop test-only Arc handles so the boundary's copy-on-write does not clone.
            self.rec_boundary_snapshot = None;
        }
        if (round as u64).is_multiple_of(rec.t_window) {
            let next_window = round as u64 / rec.t_window;
            // Read before `end_window` re-mints the checkpoint window.
            let rolling = nodes
                .iter()
                .filter(|nd| nd.reset_state() == RState::Reset)
                .map(|nd| nd.checkpoint().w);
            rec.pending_rollbacks = 0;
            rec.pending_rollback_depth = 0;
            for cp_window in rolling {
                rec.pending_rollbacks += 1;
                rec.pending_rollback_depth = rec
                    .pending_rollback_depth
                    .max((next_window - cp_window) as u32);
            }
            // Per-class mint keyed sound-over-complete: a missed equality only costs memory, never correctness.
            let mut keyed: HashMap<(usize, u64, usize), (usize, Vec<Timed>)> = HashMap::new();
            let mut mint_for: Vec<Option<usize>> = vec![None; nodes.len()];
            for i in 0..nodes.len() {
                let Some(pre) =
                    counted_pre(&nodes[i], round as u64, false).filter(|_| self.safety_ok)
                else {
                    continue;
                };
                let key = (
                    std::ptr::from_ref(nodes[i].checkpoint()) as usize,
                    crate::smr::observe::canonical::timed_digest(&pre),
                    pre.len(),
                );
                KEY_PROBES.fetch_add(1, Ordering::Relaxed);
                match keyed.get(&key) {
                    // The digest is a prefilter; the kept representative prefix is the proof.
                    Some((j, rep)) => {
                        let j = *j;
                        let matches = *rep == pre;
                        #[cfg(debug_assertions)]
                        {
                            let fresh = counted_pre(&nodes[j], round as u64, true);
                            assert_eq!(
                                fresh.as_ref(),
                                Some(rep),
                                "the kept representative prefix diverged from a fresh derivation"
                            );
                        }
                        if matches {
                            mint_for[i] = Some(j);
                        }
                    }
                    None => {
                        keyed.insert(key, (i, pre));
                    }
                }
            }
            let mut minted: Vec<Option<Arc<Checkpoint>>> = vec![None; nodes.len()];
            for i in 0..nodes.len() {
                #[cfg(debug_assertions)]
                {
                    let nd = &nodes[i];
                    let (live, held) = (nd.shared_state(), &nd.checkpoint().s);
                    debug_assert!(
                        live.logical_len() == held.logical_len()
                            && live.sn_iter().eq(held.sn_iter()),
                        "round {round}: node {i} entered the boundary with a live state that \
                         had left its checkpoint behind — the mint-dedup key relies on the two \
                         agreeing, sequence numbers included"
                    );
                }
                match mint_for[i] {
                    Some(j) => {
                        let cp = minted[j].clone().expect("a class mints before its members");
                        nodes[i].end_window_shared(next_window, round as u64, &cp);
                    }
                    None => {
                        nodes[i].end_window(next_window, round as u64);
                        minted[i] = Some(nodes[i].checkpoint_shared());
                    }
                }
            }

            // Absorb the boundary-committed suffix before anything reads a position or digest against it.
            let longest = nodes
                .iter()
                .map(|nd| nd.shared_state().executed())
                .max_by_key(ExecutedView::logical_len)
                .expect("at least one node");
            self.checker.absorb(longest);
            let canonical = self.checker.canonical();
            let verified = canonical.len();
            // Under SkipBoundary a minority P that prefixes its own log commits; the safety latch records that.
            #[cfg(debug_assertions)]
            if self.safety_ok && rec.prefix_mismatch == PrefixMismatch::Abort {
                let inconsistent = nodes.iter().position(|nd| {
                    let st = nd.shared_state();
                    crate::smr::observe::canonical::state_digest(canonical, st)
                        != canonical.digest_at(st.logical_len())
                });
                assert!(
                    inconsistent.is_none(),
                    "round {round}: the boundary pass left inconsistent executed \
                     prefixes (node {})",
                    inconsistent.unwrap_or(0)
                );
            }

            let max_w = nodes.iter().map(|nd| nd.checkpoint().w).max().unwrap_or(0);
            // Distinct by fork signature, not `Checkpoint::eq`, whose equality asserts equal offsets.
            let mut distinct: Vec<ForkSignature> = Vec::new();
            let mut representatives: Vec<Arc<Checkpoint>> = Vec::new();
            let mut member_of: Vec<Option<usize>> = vec![None; nodes.len()];
            for (i, nd) in nodes.iter().enumerate() {
                let cp = nd.checkpoint();
                if cp.w != max_w {
                    continue;
                }
                let signature = ForkSignature::of(canonical, cp);
                match distinct.iter().position(|s| *s == signature) {
                    Some(class) => member_of[i] = Some(class),
                    None => {
                        distinct.push(signature);
                        representatives.push(nd.checkpoint_shared());
                    }
                }
            }
            rec.pending_cp_fork_k = distinct.len() as u32;
            rec.pending_max_cp_p_len = distinct
                .iter()
                .filter_map(ForkSignature::pre_len)
                .max()
                .unwrap_or(0) as u32;

            let pending = self.trackers[live_from..]
                .iter()
                .any(|t| t.executed_round.is_none());
            let executed_pos = if self.safety_ok && pending {
                if rec
                    .canonical_pos
                    .extend_unique(canonical.start(), canonical.retained())
                {
                    self.exec_dup.trip_at(round);
                }
                Some(&rec.canonical_pos)
            } else {
                None
            };
            for t in self.trackers[live_from..].iter_mut() {
                if t.executed_round.is_some() {
                    continue;
                }
                let entry = Entry::Cmd(t.cc);
                let mut any_useful = false;
                let all_executed = match executed_pos {
                    Some(pos) => {
                        let at = pos.get(&entry);
                        nodes.iter().all(|nd| {
                            if nd.log_seq().is_none() {
                                return true;
                            }
                            any_useful = true;
                            at.is_some_and(|p| u64::from(p) < nd.shared_state().logical_len())
                        })
                    }
                    None => nodes.iter().all(|nd| {
                        if nd.log_seq().is_none() {
                            return true;
                        }
                        any_useful = true;
                        Executed {
                            view: nd.shared_state().executed(),
                            canonical,
                        }
                        .front_scan(&entry)
                        .is_some()
                    }),
                };
                if any_useful && all_executed {
                    t.executed_round = Some(round);
                    if let Some(pool) = &mut self.pool {
                        pool.free(t.client);
                    }
                }
            }

            // (g) release and forget: RNG-free and deliberately last.
            if self.truncate_history
                && !self.safety_ok
                && let Some(spill) = &mut self.spill
            {
                spill.mark_diverged();
            }
            if self.truncate_history && self.safety_ok {
                let entering = self.release_line;
                if let Some(i) = nodes
                    .iter()
                    .position(|nd| nd.shared_state().logical_len() < entering)
                {
                    let report = crate::smr::observe::spill::diagnose(
                        self.spill
                            .as_ref()
                            .map(crate::smr::observe::spill::SpillWriter::path),
                        i,
                        round,
                        nodes[i].shared_state().executed(),
                    );
                    panic!(
                        "driver bug: node {i} holds {} executed commands at round \
                         {round}, below the observer release line {entering}. Box-legally \
                         unreachable: adoption only ever copies a checkpoint already \
                         held in the population and no node's checkpoint length ever \
                         regresses (Lemma 6.9 monotonicity), so the minimum over them \
                         — which this line is derived from — is monotone too, and no \
                         restored state can dip below it. A firing therefore means the \
                         OBSERVER released a position a node can still re-present. It \
                         does NOT mean the protocol misbehaved: an executed-length \
                         regression is caught earlier in the same round, in both \
                         assertion configurations, by check_growth (ABORT-MONO).\n{report}",
                        nodes[i].shared_state().logical_len()
                    );
                }

                let checkpoint_lens: Vec<u64> = nodes
                    .iter()
                    .map(|nd| nd.checkpoint().s.logical_len())
                    .collect();
                let rollback_floor = checkpoint_lens
                    .iter()
                    .copied()
                    .min()
                    .expect("at least one node");
                let restorable_floor = nodes
                    .iter()
                    .map(|nd| nd.checkpoint().s.executed_offset())
                    .min()
                    .expect("at least one node");
                let cursor_floor = live_floor.map_or(verified, |f| u64::from(f).min(verified));
                let release = entering.max(rollback_floor.min(cursor_floor).min(restorable_floor));
                debug_assert!(
                    release <= restorable_floor,
                    "round {round}: the release line {release} passed the deepest \
                     restorable offset {restorable_floor}"
                );

                // Spill before anything forgets: afterwards it is the only record of the committed stream.
                if self.spill.is_none() && self.spill_allowed {
                    let (_, commit, dirty) = crate::runlog::provenance();
                    self.spill = Some(
                        crate::smr::observe::spill::SpillWriter::create(
                            crate::smr::observe::spill::SpillHeader {
                                commit,
                                dirty,
                                seed: self.seed,
                                n,
                                t_window: rec.t_window,
                                proto: "recovery".into(),
                            },
                        )
                        .expect("spill create failed"),
                    );
                }
                if let Some(spill) = &mut self.spill {
                    let consistent = nodes.iter().all(|nd| {
                        let st = nd.shared_state();
                        crate::smr::observe::canonical::state_digest(canonical, st)
                            == canonical.digest_at(st.logical_len())
                    });
                    if consistent {
                        spill
                            .boundary(
                                canonical,
                                next_window,
                                round,
                                checkpoint_lens,
                                nodes
                                    .iter()
                                    .map(|nd| nd.shared_state().logical_len())
                                    .collect(),
                                release,
                            )
                            .expect("spill boundary write failed");
                    } else {
                        spill.mark_diverged();
                    }
                }

                self.release_line = release;
                self.checker.release_to(release);

                let mut frontier = self.frontier;
                for node in nodes.iter_mut() {
                    node.forget_committed_prefix(node.checkpoint().s.logical_len());
                    frontier = frontier.max(node.shared_state().executed_offset());
                }
                self.frontier = frontier;

                #[cfg(debug_assertions)]
                {
                    for (i, nd) in nodes.iter().enumerate() {
                        assert!(
                            nd.shared_state().executed_offset() >= release,
                            "round {round}: node {i} forgot to {} but the observer \
                             released to {release}, so nothing retains what it dropped",
                            nd.shared_state().executed_offset()
                        );
                    }
                    if let Some(floor) = live_floor {
                        assert!(
                            release <= u64::from(floor),
                            "round {round}: the release line {release} passed a live \
                             cursor floor {floor}"
                        );
                    }
                }
            }

            // Interning runs last and only while the safety latch holds.
            if self.safety_ok {
                for (i, node) in nodes.iter_mut().enumerate() {
                    if let Some(class) = member_of[i] {
                        node.share_checkpoint(&representatives[class]);
                    }
                }
            }
        }

        SmrStepStatus {
            round,
            metrics: round_metrics,
            blocked: blocked_ids(mask),
            nodes: glances,
            spreading,
            dead: None,
            failure: None,
            rec: Some(RecStepInfo {
                adoptions,
                reset_votes,
            }),
        }
    }
}

#[cfg(test)]
mod tests;
