//! Sans-IO round engine: `on_round_start` → any number of `on_message` → `on_round_end`.

use crate::NodeId;
use crate::digest::{fnv_chain, fnv_entries, fnv_ops, fnv_timed};
use crate::record::{PostState, RoundRecord, TrackedDigest};
use crate::rng::{derive_rng, draw_targets};
use crate::spec::{NodeRunSpec, ScenarioKind};
use crate::wire::{AckKind, Msg, ReplyPayload, encode};
use protocol::compact::{
    ClientCommand, CompactNode, CompactReply, Entry, SharedState, Timed, Triage,
};
use protocol::{Command, GossipNode, LogNode, MedianNode, PriorityNode};
use rand_chacha::ChaCha12Rng;
use std::sync::Arc;

enum ModeCore {
    Median(MedianNode),
    Gossip(GossipNode),
    Priority(PriorityNode),
    Extended(LogNode),
    Compact(CompactNode),
}

enum Snapshot {
    Bot,
    Value(u64),
    Log(Vec<Command>),
    Compact { log: Vec<Timed>, state: SharedState },
}

impl Snapshot {
    fn held(&self) -> bool {
        !matches!(self, Snapshot::Bot)
    }
}

#[derive(Default)]
struct Counters {
    req_sent: u32,
    rep_sent: u32,
    rep_recv: u32,
    app_sent: u32,
    app_recv: u32,
    cli_recv: u32,
    acks: u32,
    bytes_out: u64,
    bytes_in: u64,
    late_rep: u32,
    late_app: u32,
    dropped_past: u32,
    encode_err: u32,
}

pub struct RoundEngine {
    node_id: NodeId,
    n: usize,
    k: usize,
    amp: usize,
    core: ModeCore,
    rng: ChaCha12Rng,
    masks: Vec<Vec<bool>>,
    round: u64,
    snapshot: Snapshot,
    slots: Vec<Option<ReplyPayload>>,
    appends_ext: Vec<Command>,
    appends_comp: Vec<(ClientCommand, u64)>,
    queued: Vec<Msg>,
    tracked_ops: Vec<u64>,
    exec_hash: u64,
    exec_hashed: usize,
    max_frame: u32,
    pad: u32,
    c: Counters,
}

impl RoundEngine {
    pub fn new(spec: &NodeRunSpec, node_id: NodeId) -> Result<RoundEngine, String> {
        let (core, n, k, seed, schedule, max_rounds, amp) = match &spec.scenario {
            ScenarioKind::Smr(s) => {
                let scenario = sim::smr::SmrScenario::try_from(s.clone())?;
                let core = match scenario.proto {
                    sim::smr::Proto::Extended => {
                        ModeCore::Extended(LogNode::new(sim::smr::SEED_COMMAND, scenario.cfg))
                    }
                    sim::smr::Proto::Compact { t_commit_rounds } => {
                        ModeCore::Compact(CompactNode::new(scenario.cfg, t_commit_rounds))
                    }
                    sim::smr::Proto::Recovery { .. } => {
                        return Err("recovery proto is not supported in networked mode".into());
                    }
                };
                let amp = sim::smr::amp_count(scenario.sigma, scenario.n);
                (
                    core,
                    scenario.n,
                    scenario.cfg.k,
                    scenario.seed,
                    scenario.schedule,
                    scenario.max_rounds,
                    amp,
                )
            }
            ScenarioKind::Gossip(g) => {
                let scenario = spec.value_scenario()?;
                let initial = if (node_id as usize) < g.holders {
                    g.x
                } else {
                    g.x0
                };
                (
                    ModeCore::Gossip(GossipNode::new(initial, g.x, g.x0, scenario.cfg)),
                    scenario.n,
                    scenario.cfg.k,
                    scenario.seed,
                    scenario.schedule,
                    scenario.max_rounds,
                    0,
                )
            }
            ScenarioKind::Median(_) | ScenarioKind::Priority(_) => {
                let scenario = spec.value_scenario()?;
                // replay the sim's sequential init loop from the shared init stream
                let mut init_rng = derive_rng(scenario.seed, "init", 0);
                let mut own = None;
                for i in 0..scenario.n {
                    let v = sim::initial_state(&scenario.init, i, scenario.n, &mut init_rng);
                    if i == node_id as usize {
                        own = Some(v);
                    }
                }
                let own =
                    own.ok_or_else(|| format!("node id {node_id} outside 0..{}", scenario.n))?;
                let core = match &spec.scenario {
                    ScenarioKind::Median(_) => ModeCore::Median(match own {
                        Some(v) => MedianNode::new(v, scenario.cfg),
                        None => MedianNode::new_undecided(scenario.cfg),
                    }),
                    _ => ModeCore::Priority(PriorityNode::new(
                        own.expect("priority arm rejects undecided starts"),
                        scenario.cfg,
                    )),
                };
                (
                    core,
                    scenario.n,
                    scenario.cfg.k,
                    scenario.seed,
                    scenario.schedule,
                    scenario.max_rounds,
                    0,
                )
            }
        };
        if node_id as usize >= n {
            return Err(format!("node id {node_id} outside 0..{n}"));
        }
        let mut mask_rng = derive_rng(seed, "blocking", 0);
        let masks = sim::mask_schedule(&schedule, n, max_rounds, &mut mask_rng);
        Ok(RoundEngine {
            node_id,
            n,
            k,
            amp,
            core,
            rng: derive_rng(seed, "node", node_id),
            masks,
            round: 0,
            snapshot: Snapshot::Bot,
            slots: Vec::new(),
            appends_ext: Vec::new(),
            appends_comp: Vec::new(),
            queued: Vec::new(),
            tracked_ops: spec.tracked_ops.clone(),
            exec_hash: sim::digest::FNV_BASIS,
            exec_hashed: 0,
            max_frame: spec.max_frame_bytes,
            pad: spec.payload_bytes,
            c: Counters::default(),
        })
    }

    pub fn round(&self) -> u64 {
        self.round
    }

    fn blocked_now(&self) -> bool {
        self.masks
            .get((self.round.max(1) - 1) as usize)
            .map(|row| row[self.node_id as usize])
            .expect("mask schedule covers every round of the horizon")
    }

    fn frame_len(&mut self, msg: &Msg) -> u64 {
        match encode(msg, self.max_frame, self.pad) {
            Ok(f) => f.len() as u64,
            Err(e) => {
                eprintln!("node {}: encode failed: {e}", self.node_id);
                self.c.encode_err += 1;
                0
            }
        }
    }

    fn take_snapshot(&self) -> Snapshot {
        match &self.core {
            ModeCore::Median(node) => node.answer().map_or(Snapshot::Bot, Snapshot::Value),
            ModeCore::Gossip(node) => node.answer().map_or(Snapshot::Bot, Snapshot::Value),
            ModeCore::Priority(node) => node.answer().map_or(Snapshot::Bot, Snapshot::Value),
            ModeCore::Extended(node) => node
                .log()
                .map_or(Snapshot::Bot, |l| Snapshot::Log(l.to_vec())),
            ModeCore::Compact(node) => match node.log_entries() {
                Some(log) => Snapshot::Compact {
                    log: log.to_vec(),
                    // cloned now: reading it after the step would serve post-step state
                    state: node.shared_state().clone(),
                },
                None => Snapshot::Bot,
            },
        }
    }

    pub fn on_round_start(&mut self) -> Vec<(NodeId, Msg)> {
        self.round += 1;
        self.c = Counters::default();
        self.slots = vec![None; self.k];
        self.appends_ext.clear();
        self.appends_comp.clear();
        self.snapshot = self.take_snapshot();

        let mut out = Vec::new();
        let due: Vec<Msg> = {
            let round = self.round;
            let (now, later): (Vec<Msg>, Vec<Msg>) = std::mem::take(&mut self.queued)
                .into_iter()
                .partition(|m| msg_round(m) == round);
            self.queued = later;
            now
        };
        for msg in due {
            out.extend(self.on_message(msg));
        }

        // stream parity with the sim: draws happen even when blocked
        let targets = draw_targets(&mut self.rng, self.n, self.k);
        if self.blocked_now() {
            return Vec::new();
        }
        let needs_state = matches!(self.core, ModeCore::Compact(_)) && !self.snapshot.held();
        for (slot, target) in targets.into_iter().enumerate() {
            let msg = Msg::PullRequest {
                round: self.round,
                from: self.node_id,
                slot: slot as u16,
                needs_state,
            };
            self.c.req_sent += 1;
            self.c.bytes_out += self.frame_len(&msg);
            out.push((target, msg));
        }
        out
    }

    fn amplify(&mut self, client: u32, sn: u64, op: u64) -> Vec<(NodeId, Msg)> {
        let targets = draw_targets(&mut self.rng, self.n, self.amp);
        let mut out = Vec::with_capacity(targets.len());
        for target in targets {
            let msg = Msg::Append {
                round: self.round,
                from: self.node_id,
                client,
                sn,
                op,
            };
            self.c.app_sent += 1;
            self.c.bytes_out += self.frame_len(&msg);
            out.push((target, msg));
        }
        out
    }

    fn on_client_cmd(&mut self, client: u32, sn: u64, op: u64) -> Vec<(NodeId, Msg)> {
        self.c.cli_recv += 1;
        let ack = |kind: AckKind, round: u64| Msg::ClientAck {
            round,
            client,
            sn,
            kind,
        };
        match &mut self.core {
            ModeCore::Median(_) | ModeCore::Gossip(_) | ModeCore::Priority(_) => Vec::new(),
            ModeCore::Extended(node) => {
                let wants = node.wants_amplify(op);
                let mut out = vec![(client, ack(AckKind::Delivered, self.round))];
                self.c.acks += 1;
                if wants {
                    out.extend(self.amplify(client, sn, op));
                }
                out
            }
            ModeCore::Compact(node) => {
                let cc = ClientCommand { client, sn, op };
                let triage = node.on_client_command(&cc);
                // deviation: see node/networked-node-architecture.md (F16)
                if !matches!(triage, Triage::Ignore) {
                    self.c.acks += 1;
                }
                match triage {
                    Triage::Amplify => {
                        let mut out = vec![(client, ack(AckKind::Amplified, self.round))];
                        out.extend(self.amplify(client, sn, op));
                        // deviation: see node/networked-node-architecture.md (F12)
                        out
                    }
                    Triage::AckCommitted => vec![(client, ack(AckKind::AckCommitted, self.round))],
                    Triage::Ignore => vec![(client, ack(AckKind::Ignored, self.round))],
                }
            }
        }
    }

    pub fn on_message(&mut self, msg: Msg) -> Vec<(NodeId, Msg)> {
        // deviation: see node/networked-node-architecture.md (F14)
        if let Msg::ClientCmd { client, sn, op, .. } = msg {
            if self.blocked_now() {
                return Vec::new();
            }
            return self.on_client_cmd(client, sn, op);
        }

        let r = msg_round(&msg);
        if r == self.round + 1 {
            self.queued.push(msg);
            return Vec::new();
        }
        if r != self.round {
            match msg {
                Msg::PullReply { .. } if r < self.round => self.c.late_rep += 1,
                Msg::Append { .. } if r < self.round => self.c.late_app += 1,
                _ => self.c.dropped_past += 1,
            }
            return Vec::new();
        }
        if self.blocked_now() {
            return Vec::new();
        }
        match msg {
            Msg::PullRequest {
                from,
                slot,
                needs_state,
                ..
            } => {
                self.c.bytes_in += self.frame_len(&Msg::PullRequest {
                    round: self.round,
                    from,
                    slot,
                    needs_state,
                });
                let payload = match &self.snapshot {
                    Snapshot::Bot => None,
                    Snapshot::Value(v) => Some(ReplyPayload::Value(*v)),
                    Snapshot::Log(log) => Some(ReplyPayload::Log(log.clone())),
                    Snapshot::Compact { log, state } => Some(ReplyPayload::Compact {
                        log: log.clone(),
                        state: needs_state.then(|| state.clone()),
                    }),
                };
                match payload {
                    Some(payload) => {
                        let reply = Msg::PullReply {
                            round: self.round,
                            from: self.node_id,
                            slot,
                            payload,
                        };
                        self.c.rep_sent += 1;
                        self.c.bytes_out += self.frame_len(&reply);
                        vec![(from, reply)]
                    }
                    // x_i = ⊥: no reply (Alg 1 "if x_i ≠ ⊥ then ... send x_i back")
                    None => Vec::new(),
                }
            }
            Msg::PullReply { slot, payload, .. } => {
                self.c.bytes_in += self.frame_len(&Msg::PullReply {
                    round: self.round,
                    from: 0,
                    slot,
                    payload: payload.clone(),
                });
                let slot = slot as usize;
                if slot < self.slots.len() && self.slots[slot].is_none() {
                    self.slots[slot] = Some(payload);
                    self.c.rep_recv += 1;
                }
                Vec::new()
            }
            Msg::Append {
                round,
                client,
                sn,
                op,
                ..
            } => {
                self.c.app_recv += 1;
                self.c.bytes_in += self.frame_len(&Msg::Append {
                    round,
                    from: 0,
                    client,
                    sn,
                    op,
                });
                match &self.core {
                    ModeCore::Extended(_) => self.appends_ext.push(op),
                    ModeCore::Compact(_) => self
                        .appends_comp
                        .push((ClientCommand { client, sn, op }, round)),
                    _ => {}
                }
                Vec::new()
            }
            Msg::ClientAck { .. } | Msg::RoundDone { .. } | Msg::RoundGo { .. } => Vec::new(),
            Msg::ClientCmd { .. } => unreachable!("handled above"),
        }
    }

    pub fn on_round_end(&mut self) -> RoundRecord {
        let collected: Vec<ReplyPayload> = self.slots.iter().flatten().cloned().collect();
        let replies = collected.len() as u8;
        let values = payload_values(&collected);
        match &mut self.core {
            ModeCore::Median(node) => node.step(&values, &mut self.rng),
            ModeCore::Gossip(node) => node.step(&values, &mut self.rng),
            ModeCore::Priority(node) => node.step(&values, &mut self.rng),
            ModeCore::Extended(node) => {
                let logs: Vec<Arc<Vec<Command>>> = collected
                    .iter()
                    .filter_map(|p| match p {
                        ReplyPayload::Log(l) => Some(Arc::new(l.clone())),
                        _ => None,
                    })
                    .collect();
                node.step(&logs, &self.appends_ext, &mut self.rng);
            }
            ModeCore::Compact(node) => {
                let replies: Vec<CompactReply> = collected
                    .iter()
                    .filter_map(|p| match p {
                        ReplyPayload::Compact { log, state } => Some(CompactReply::from_log(
                            Arc::new(protocol::chunked::ChunkSeq::from_vec(log.clone())),
                            state.clone().map(Arc::new),
                        )),
                        _ => None,
                    })
                    .collect();
                node.step(&replies, &self.appends_comp, self.round, &mut self.rng);
            }
        }

        let post = match &self.core {
            ModeCore::Median(node) => PostState::Value(node.answer()),
            ModeCore::Gossip(node) => PostState::Value(node.answer()),
            ModeCore::Priority(node) => PostState::Value(node.answer()),
            ModeCore::Extended(node) => PostState::Log {
                log_len: node.log().map(|l| l.len()),
                exec_len: 0,
                log_hash: node.log().map(|l| fnv_ops(l.iter().copied())),
            },
            ModeCore::Compact(node) => PostState::Log {
                log_len: node.log_entries().map(|l| l.len()),
                exec_len: node.shared_state().untruncated().len(),
                log_hash: node.log_entries().map(|l| fnv_timed(l.iter())),
            },
        };
        let exec_prefix = self.exec_prefix_delta();
        let tracked = self.tracked_digests();
        let c = std::mem::take(&mut self.c);
        RoundRecord {
            r: self.round,
            blocked: self.blocked_now(),
            snap_held: self.snapshot.held(),
            post,
            replies,
            req_sent: c.req_sent,
            rep_sent: c.rep_sent,
            rep_recv: c.rep_recv,
            app_sent: c.app_sent,
            app_recv: c.app_recv,
            cli_recv: c.cli_recv,
            acks: c.acks,
            bytes_out: c.bytes_out,
            bytes_in: c.bytes_in,
            late_rep: c.late_rep,
            late_app: c.late_app,
            dropped_past: c.dropped_past,
            encode_err: c.encode_err,
            exec_prefix,
            t_last_reply_us: 0,
            t_step_done_us: 0,
            t_step_compute_us: 0,
            tracked,
        }
    }

    fn exec_prefix_delta(&mut self) -> Vec<(usize, u64)> {
        let ModeCore::Compact(node) = &self.core else {
            return Vec::new();
        };
        let executed = node.shared_state().untruncated();
        let mut out = Vec::new();
        for entry in executed.iter().skip(self.exec_hashed) {
            self.exec_hash = fnv_chain(self.exec_hash, entry);
            self.exec_hashed += 1;
            out.push((self.exec_hashed, self.exec_hash));
        }
        out
    }

    fn tracked_digests(&self) -> Vec<TrackedDigest> {
        self.tracked_ops
            .iter()
            .map(|&op| match &self.core {
                ModeCore::Extended(node) => {
                    let (pos, hash) = node
                        .log()
                        .and_then(|log| {
                            log.iter()
                                .position(|&c| c == op)
                                .map(|p| (Some(p), Some(fnv_ops(log[..=p].iter().copied()))))
                        })
                        .unwrap_or((None, None));
                    TrackedDigest {
                        op,
                        present: pos.is_some(),
                        pos,
                        prefix_hash: hash,
                    }
                }
                ModeCore::Compact(node) => {
                    let state = node.shared_state();
                    // the witness is rooted at index 0, so states here must never be truncated
                    let executed = state.untruncated();
                    let exec_pos = executed.iter().position(|e| e.op() == Some(op));
                    let (pos, hash) = match exec_pos {
                        Some(p) => (Some(p), Some(fnv_entries(executed.iter().take(p + 1)))),
                        None => match node.log_entries().and_then(|log| {
                            log.iter().position(|t| t.entry.op() == Some(op)).map(|w| {
                                let mut prefix: Vec<&Entry> = executed.iter().collect();
                                prefix.extend(log[..=w].iter().map(|t| &t.entry));
                                (executed.len() + w, fnv_entries(prefix.into_iter()))
                            })
                        }) {
                            Some((p, h)) => (Some(p), Some(h)),
                            None => (None, None),
                        },
                    };
                    TrackedDigest {
                        op,
                        present: pos.is_some(),
                        pos,
                        prefix_hash: hash,
                    }
                }
                _ => TrackedDigest {
                    op,
                    present: false,
                    pos: None,
                    prefix_hash: None,
                },
            })
            .collect()
    }
}

fn payload_values(collected: &[ReplyPayload]) -> Vec<u64> {
    collected
        .iter()
        .filter_map(|p| match p {
            ReplyPayload::Value(v) => Some(*v),
            _ => None,
        })
        .collect()
}

fn msg_round(msg: &Msg) -> u64 {
    match msg {
        Msg::PullRequest { round, .. }
        | Msg::PullReply { round, .. }
        | Msg::Append { round, .. }
        | Msg::ClientCmd { round, .. }
        | Msg::ClientAck { round, .. }
        | Msg::RoundDone { round, .. }
        | Msg::RoundGo { round } => *round,
    }
}
