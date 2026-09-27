//! Compact-rule (Algorithm 5) round step.

use crate::smr::engine::stages::{amplify, apply_all};
use crate::smr::observe::canon_cursor::{CanonCursor, canon_lookup};
use crate::smr::observe::canonical::Executed;
use crate::smr::observe::log_index::{LogCounts, LogIndex, log_window_pos};
use crate::smr::observe::round_metrics::smr_round_metrics;
use crate::smr::tracker::{
    CommandTracker, advance_inert_prefix, latch_coverage, pos_stats, spreading_list,
};
use crate::smr::*;
use protocol::compact::{
    ClientCommand, CompactNode, CompactReply, Entry, Log, SharedState, Triage,
};
use protocol::shared_state::ExecutedView;
use rand::Rng;
use std::collections::BTreeMap;
use std::sync::Arc;

impl SmrState {
    pub(in crate::smr) fn step_comp(&mut self, mask: &[bool]) -> SmrStepStatus {
        let n = self.n;
        let cfg = self.cfg;
        let amp = self.amp;
        let round = self.round;
        let arrivals = std::mem::take(&mut self.arrivals_pending);
        let SmrNodes::Comp(nodes) = &mut self.nodes else {
            unreachable!("dispatched on Comp");
        };
        // Nodes swap, never edit, their log and index, so these handles ARE the pre-step snapshot.
        let snap_logs: Vec<Option<Log>> =
            nodes.iter().map(|nd| nd.log_perm_arc().cloned()).collect();
        #[cfg(debug_assertions)]
        let snap_states: Vec<_> = if snap_logs.iter().any(Option::is_none) {
            nodes.iter().map(|nd| nd.shared_state().clone()).collect()
        } else {
            Vec::new()
        };
        let useful: Vec<bool> = (0..n).map(|i| snap_logs[i].is_some() && !mask[i]).collect();
        let useful_total = useful.iter().filter(|&&u| u).count() as u32;

        // (b) client stage: resend every round until AckCommitted
        let mut delivered: Vec<(usize, ClientCommand)> = Vec::new();
        for &ti in &self.client_order {
            let t = &mut self.trackers[ti];
            if t.committed_ack_round.is_none() && round >= t.injection_round {
                let target = match t.target {
                    Some(id) => id as usize,
                    None => self.rng.random_range(0..n),
                };
                let outcome = if mask[target] {
                    AttemptOutcome::TargetBlocked
                } else {
                    match nodes[target].on_client_command(&t.cc) {
                        Triage::Amplify => {
                            if t.delivered_round.is_none() {
                                t.delivered_round = Some(round);
                            }
                            delivered.push((target, t.cc));
                            AttemptOutcome::Amplified
                        }
                        Triage::AckCommitted => {
                            t.committed_ack_round = Some(round);
                            AttemptOutcome::AckCommitted
                        }
                        Triage::Ignore => AttemptOutcome::Ignored,
                    }
                };
                t.attempts.push(DeliveryAttempt {
                    round,
                    target: target as u32,
                    outcome,
                });
                // Alg 5 "if L_i ≠ ⊥ and sn(x) = sn(c)": the pool slot is drawable from the next round's stage (a0).
                if outcome == AttemptOutcome::AckCommitted {
                    let freed = t.client;
                    if let Some(pool) = &mut self.pool {
                        pool.free(freed);
                    }
                }
            }
        }

        // (c) amplify stage
        let partition = self.partition.as_deref();
        let appends = amplify(
            &mut self.rng,
            &mut self.trackers,
            delivered,
            |_, _| true,
            mask,
            partition,
            n,
            amp,
            round,
        );

        // (d) log-request stage: Alg 5 "i sends k requests ... and attaches a bit b_i", answered by `answer`
        let mut inboxes: Vec<Vec<CompactReply>> = Vec::with_capacity(n);
        // Sharing across targets is sound only while the latch holds and no null is reachable.
        let states_by_len = self.safety_ok && !self.nulls_possible;
        let mut by_len: BTreeMap<(u64, usize), Arc<SharedState>> = BTreeMap::new();
        let mut by_target: Vec<Option<Arc<SharedState>>> = vec![None; n];
        for i in 0..n {
            let b_i = nodes[i].b_i();
            let mut replies = Vec::with_capacity(cfg.k);
            for _ in 0..cfg.k {
                let target = self.rng.random_range(0..n);
                if !mask[i]
                    && !mask[target]
                    && same_component(partition, i, target)
                    && let Some(mut reply) = nodes[target].answer(b_i)
                {
                    if let Some(live) = &reply.s_j {
                        #[cfg(debug_assertions)]
                        assert_eq!(
                            live.as_ref(),
                            &snap_states[target],
                            "stage (d) read post-step state"
                        );
                        let snap = if states_by_len {
                            by_len
                                .entry((live.logical_len(), live.sn_len()))
                                .or_insert_with(|| Arc::clone(live))
                        } else {
                            by_target[target].get_or_insert_with(|| Arc::clone(live))
                        };
                        #[cfg(debug_assertions)]
                        assert_eq!(
                            snap.as_ref(),
                            live.as_ref(),
                            "node {target}: shared snapshot is not this node's state"
                        );
                        reply.s_j = Some(Arc::clone(snap));
                    }
                    replies.push(reply);
                }
            }
            inboxes.push(replies);
        }

        // (e) core step: all draws first, ascending id, reading only stage-(d) inbox lengths; applies are RNG-free.
        let choices: Vec<Option<Vec<usize>>> = (0..n)
            .map(|i| nodes[i].draw_reply_choice(inboxes[i].len(), &mut self.rng))
            .collect();
        apply_all(nodes, |i, node: &mut CompactNode| {
            node.step_chosen(
                &inboxes[i],
                &appends[i],
                round as u64,
                choices[i].as_deref(),
            );
        });

        // (f) observation: no RNG from here on
        let post_logs: Vec<Option<&protocol::chunked::ChunkSeq<protocol::compact::Timed>>> =
            nodes.iter().map(CompactNode::log_seq).collect();
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
        if self.safety_ok {
            let canonical = self.checker.canonical();
            self.exec_dup
                .observe_canonical(canonical.start(), canonical.retained(), round);
        } else if self.exec_dup.tripped_round().is_none() {
            let executed: Vec<ExecutedView<'_>> = nodes
                .iter()
                .map(|nd| nd.shared_state().executed())
                .collect();
            // Alg 5 swaps S_i for a peer's only on the b_i = 1 (L_i = ⊥) branches.
            let replaced: Vec<bool> = snap_logs.iter().map(Option::is_none).collect();
            self.exec_dup.observe_nodes(&executed, &replaced, round);
        }
        let cursors_exact = self.safety_ok;
        if cursors_exact {
            let canonical = self.checker.canonical();
            self.canon_index
                .extend_first(canonical.start(), canonical.retained());
        }
        let canon_index = &self.canon_index;
        let canonical = self.checker.canonical();

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
            t.committed_ack_round.is_some_and(|r| round > r + 1)
                && t.all_logs_round.is_some()
                && t.canon_cursor.is_none()
                && (!drop_settled_spread || t.spread_points == 0)
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
            if t.committed_ack_round.is_none_or(|r| round <= r + 1) {
                t.spread_points += 1;
                crate::smr::observe::census_count::window();
                // Outside the scoping branch: `live_floor` gates `forget_committed_prefix`.
                let opened = t
                    .canon_cursor
                    .get_or_insert_with(|| CanonCursor::new(pos_floor));
                live_floor = Some(live_floor.unwrap_or(u32::MAX).min(opened.floor()));
                if observe_spread {
                    let useful_holders = useful_counts
                        .as_ref()
                        .expect("census on implies the counts were built")
                        .count(&entry);
                    debug_assert_eq!(
                        useful_holders,
                        snap_logs
                            .iter()
                            .zip(&useful)
                            .filter(|&(log, &u)| u
                                && log
                                    .as_ref()
                                    .is_some_and(|l| l.entries().iter().any(|e| e.entry == entry)))
                            .count() as u32
                    );
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
                    t.shed_attempts();
                }
            }
            latch_coverage(t, &entry, round, &post_index, &post_logs);
        }

        let spreading = if drop_settled_spread {
            Vec::new()
        } else {
            spreading_list(
                &self.trackers,
                live_from,
                round,
                &post_index,
                &post_logs,
                |t| t.all_logs_round.is_none(),
            )
        };

        if round_metrics.nonbot_logs == 0 && self.dead.is_none() {
            self.dead = Some(round);
        }
        let glances = post_logs
            .iter()
            .zip(&executed_lens)
            .map(|(log, &ex)| NodeGlance {
                log_len: log.map(|l| l.len() as u32),
                executed_len: ex as u32,
                r: None,
            })
            .collect();
        drop(post_logs);

        // (g) truncation: after the walk; frozen once the safety latch is lost.
        if self.truncate_history && self.safety_ok {
            let floor = live_floor.map_or_else(|| self.checker.canonical_len() as u64, u64::from);
            self.frontier = self.frontier.max(floor);
            #[cfg(debug_assertions)]
            if let Some(floor) = live_floor {
                assert!(
                    self.frontier <= u64::from(floor),
                    "round {round}: the frontier {} passed a live cursor floor {floor}",
                    self.frontier
                );
            }
            for node in nodes.iter_mut() {
                let upto = self.frontier.min(node.shared_state().logical_len());
                node.forget_committed_prefix(upto);
                debug_assert_eq!(
                    node.shared_state().executed_offset(),
                    upto,
                    "round {round}: an offset is not min(frontier, own length)"
                );
            }
        }

        SmrStepStatus {
            round,
            metrics: round_metrics,
            blocked: blocked_ids(mask),
            nodes: glances,
            spreading,
            dead: self.dead,
            failure: None,
            rec: None,
        }
    }
}
