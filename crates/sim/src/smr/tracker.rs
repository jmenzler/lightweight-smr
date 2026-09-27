//! Per-command tracking: the client's retry state (world) and its landmarks (observation).

use super::observe::canon_cursor::CanonCursor;
use super::observe::log_index::LogIndex;
use super::{
    CommandReport, CommandStatus, DeliveryAttempt, Injection, SmrTerminal, SpreadCurve,
    SpreadPoint, SpreadingCommand,
};
use protocol::compact::{ClientCommand, Entry, Timed};
use protocol::extended::Command;

pub(super) fn pos_stats(mut positions: Vec<u32>) -> (Option<u32>, Option<u32>, Option<u32>) {
    if positions.is_empty() {
        return (None, None, None);
    }
    let mid = positions.len() / 2;
    let (_, &mut med, _) = positions.select_nth_unstable(mid);
    (
        positions.iter().min().copied(),
        Some(med),
        positions.iter().max().copied(),
    )
}

pub(super) struct CommandTracker {
    pub(super) client: u32,
    pub(super) op: u64,
    pub(super) cc: ClientCommand,
    pub(super) injection_round: usize,
    pub(super) target: Option<u32>,
    pub(super) auto: bool,
    pub(super) delivered_round: Option<usize>,
    pub(super) all_logs_round: Option<usize>,
    pub(super) prefix_fixed_round: Option<usize>,
    pub(super) committed_ack_round: Option<usize>,
    pub(super) executed_round: Option<usize>,
    /// Asserted never to un-fix (Lemma 3.8); a regression is a driver bug.
    pub(super) fixed_prefix: Option<Vec<Command>>,
    pub(super) spread: Vec<SpreadPoint>,
    /// `inert` reads this, not `spread`, so census scoping cannot move `live_floor`.
    pub(super) spread_points: u32,
    pub(super) attempts: Vec<DeliveryAttempt>,
    pub(super) amp_receivers: Vec<u32>,
    pub(super) canon_cursor: Option<CanonCursor>,
}

impl CommandTracker {
    pub(super) fn report(&self, terminal: SmrTerminal, observed: bool) -> CommandReport {
        let complete = self.prefix_fixed_round.is_some()
            || self.committed_ack_round.is_some()
            || self.executed_round.is_some();
        let status = if complete {
            CommandStatus::Complete
        } else if matches!(terminal, SmrTerminal::Dead { .. }) {
            CommandStatus::Dead
        } else {
            CommandStatus::Pending
        };
        CommandReport {
            client: self.client,
            op: self.op,
            injection_round: self.injection_round,
            delivered_round: self.delivered_round,
            all_logs_round: self.all_logs_round,
            prefix_fixed_round: self.prefix_fixed_round,
            committed_ack_round: self.committed_ack_round,
            executed_round: self.executed_round,
            auto: self.auto,
            spread: if observed {
                SpreadCurve::Observed(self.spread.clone())
            } else {
                SpreadCurve::NotObserved
            },
            attempts: self.attempts.clone(),
            amp_receivers: self.amp_receivers.clone(),
            status,
        }
    }

    pub(super) fn shed_spread(&mut self) {
        if !self.spread.is_empty() {
            self.spread = Vec::new();
        }
        // Unconditional: `inert` reads the counter even when `spread` is already empty.
        self.spread_points = 0;
    }

    pub(super) fn shed_attempts(&mut self) {
        if !self.attempts.is_empty() {
            self.attempts = Vec::new();
            self.amp_receivers = Vec::new();
        }
    }

    pub(super) fn observe(
        &mut self,
        round: usize,
        snapshot: &[Option<std::sync::Arc<Vec<Command>>>],
        useful: &[bool],
        useful_total: u32,
        post: &[Option<&[Command]>],
        observe_spread: bool,
    ) {
        if round < self.injection_round {
            return;
        }
        if self.prefix_fixed_round.is_none_or(|r| round <= r + 1) {
            self.spread_points += 1;
            super::observe::census_count::window();
            if observe_spread {
                let useful_holders = snapshot
                    .iter()
                    .zip(useful)
                    .filter(|&(log, &u)| u && log.as_ref().is_some_and(|l| l.contains(&self.op)))
                    .count() as u32;
                // Counted by the fold: `flatten` drops ⊥, so `post.len()` would overcount.
                let mut visits = 0usize;
                let (pos_min, pos_med, pos_max) = pos_stats(
                    post.iter()
                        .flatten()
                        .inspect(|_| visits += 1)
                        .filter_map(|log| log.iter().position(|&x| x == self.op).map(|p| p as u32))
                        .collect(),
                );
                super::observe::census_count::point(visits);
                self.spread.push(SpreadPoint {
                    round,
                    useful_holders,
                    useful_total,
                    pos_min,
                    pos_med,
                    pos_max,
                });
                debug_assert_eq!(
                    self.spread_points as usize,
                    self.spread.len(),
                    "op {}: the point counter drifted from the curve",
                    self.op
                );
            }
        }

        let nonbot: Vec<&[Command]> = post.iter().flatten().copied().collect();
        if nonbot.is_empty() {
            return;
        }
        let in_all = nonbot.iter().all(|log| log.contains(&self.op));
        match (self.all_logs_round, in_all) {
            (None, true) => self.all_logs_round = Some(round),
            (Some(r), false) => panic!(
                "driver bug: op {} left a log in round {round} after covering all logs in round {r}",
                self.op
            ),
            _ => {}
        }

        let prefix = in_all.then(|| {
            let first = nonbot[0];
            let pos = first.iter().position(|&x| x == self.op).expect("in_all");
            &first[..=pos]
        });
        let all_fixed = prefix.is_some_and(|p| {
            nonbot
                .iter()
                .all(|log| log.len() >= p.len() && &log[..p.len()] == p)
        });
        match (&self.fixed_prefix, all_fixed) {
            (None, true) => {
                self.prefix_fixed_round = Some(round);
                self.fixed_prefix = Some(prefix.expect("all_fixed").to_vec());
            }
            (Some(p), _) => {
                let still = nonbot
                    .iter()
                    .all(|log| log.len() >= p.len() && log[..p.len()] == p[..]);
                assert!(
                    still,
                    "driver bug: op {} prefix un-fixed in round {round} (fixed in round {:?})",
                    self.op, self.prefix_fixed_round
                );
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod pos_stats_tests;

/// sn per injection: 1 + earlier injections of the same client, by round then list order.
pub fn injection_sns(injections: &[Injection]) -> Vec<u64> {
    let mut by_round: Vec<usize> = (0..injections.len()).collect();
    by_round.sort_by_key(|&i| injections[i].round);
    let mut seen: std::collections::BTreeMap<u32, u64> = std::collections::BTreeMap::new();
    let mut sns = vec![0u64; injections.len()];
    for &i in &by_round {
        let count = seen.entry(injections[i].client).or_insert(0);
        *count += 1;
        sns[i] = *count;
    }
    sns
}

/// End of the leading run of inert trackers; conditions latch once, so it only advances.
pub(super) fn advance_inert_prefix(
    trackers: &[CommandTracker],
    from: usize,
    inert: impl Fn(&CommandTracker) -> bool,
) -> usize {
    let mut i = from;
    while i < trackers.len() && inert(&trackers[i]) {
        i += 1;
    }
    i
}

/// T_B landmark: latch-once, never asserted on regression (un-covering is legal here).
pub(super) fn latch_coverage(
    t: &mut CommandTracker,
    entry: &Entry,
    round: usize,
    post_index: &LogIndex,
    post_logs: &[Option<&protocol::chunked::ChunkSeq<Timed>>],
) {
    if t.all_logs_round.is_some() {
        return;
    }
    let covered = post_index.covers_all(entry);
    debug_assert_eq!(covered, {
        let nonbot: Vec<&protocol::chunked::ChunkSeq<Timed>> =
            post_logs.iter().flatten().copied().collect();
        !nonbot.is_empty()
            && nonbot
                .iter()
                .all(|log| log.iter().any(|e| &e.entry == entry))
    });
    if covered {
        t.all_logs_round = Some(round);
    }
}

pub(super) fn spreading_list(
    trackers: &[CommandTracker],
    live_from: usize,
    round: usize,
    post_index: &LogIndex,
    post_logs: &[Option<&protocol::chunked::ChunkSeq<Timed>>],
    open: impl Fn(&CommandTracker) -> bool,
) -> Vec<SpreadingCommand> {
    trackers[live_from..]
        .iter()
        .filter(|t| open(t) && round >= t.injection_round)
        .map(|t| {
            let entry = Entry::Cmd(t.cc);
            let holders = post_index.holder_ids(&entry);
            debug_assert_eq!(
                holders,
                post_logs
                    .iter()
                    .enumerate()
                    .filter(|(_, log)| log.is_some_and(|l| l.iter().any(|e| e.entry == entry)))
                    .map(|(id, _)| id as u32)
                    .collect::<Vec<u32>>()
            );
            SpreadingCommand {
                client: t.client,
                holders,
            }
        })
        .collect()
}

/// Decided once over manual injections: auto arrivals never share (client, sn) across ops.
pub(super) fn nulls_reachable(trackers: &[CommandTracker]) -> bool {
    let mut seen: std::collections::BTreeMap<(u32, u64), u64> = std::collections::BTreeMap::new();
    for t in trackers {
        if t.cc.client >= super::AUTO_CLIENT_BASE {
            return true;
        }
        if *seen.entry((t.cc.client, t.cc.sn)).or_insert(t.cc.op) != t.cc.op {
            return true;
        }
    }
    false
}

pub(super) fn trackers_for(injections: &[Injection]) -> (Vec<CommandTracker>, Vec<usize>) {
    let sns = injection_sns(injections);
    let trackers: Vec<CommandTracker> = injections
        .iter()
        .zip(&sns)
        .map(|(inj, &sn)| CommandTracker {
            client: inj.client,
            op: inj.op,
            cc: ClientCommand {
                client: inj.client,
                sn,
                op: inj.op,
            },
            injection_round: inj.round,
            target: inj.target,
            auto: false,
            delivered_round: None,
            all_logs_round: None,
            prefix_fixed_round: None,
            committed_ack_round: None,
            executed_round: None,
            fixed_prefix: None,
            spread: Vec::new(),
            spread_points: 0,
            attempts: Vec::new(),
            amp_receivers: Vec::new(),
            canon_cursor: None,
        })
        .collect();
    // Client stage runs in ascending client id; injections may come in any order.
    let mut client_order: Vec<usize> = (0..trackers.len()).collect();
    client_order.sort_by_key(|&i| trackers[i].client);
    (trackers, client_order)
}

#[cfg(test)]
mod tests;
