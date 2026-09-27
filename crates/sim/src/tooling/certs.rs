//! §5 certificate driver harness: per-server cert state, commit acks, and Thm 6 verification.

use protocol::certificates::{Certificate, ClientChains, MergeEvent, ServerCertState};
use protocol::compact::{ClientCommand, Entry};

use crate::smr::{AttemptOutcome, SmrState};
use std::collections::BTreeMap;

pub struct CertHarness {
    servers: Vec<ServerCertState>,
    fed: Vec<usize>,
    clients: BTreeMap<u32, ClientChains>,
    sn_by_op: BTreeMap<u64, (u32, u64)>,
    last_merges: Vec<(usize, MergeEvent)>,
}

/// Verification tally: `covered` useful servers can judge the certificate, `accepted` of those.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifyTally {
    pub covered: usize,
    pub accepted: usize,
}

impl CertHarness {
    pub fn new(n: usize) -> Self {
        Self {
            servers: (0..n).map(|_| ServerCertState::new()).collect(),
            fed: vec![0; n],
            clients: BTreeMap::new(),
            sn_by_op: BTreeMap::new(),
            last_merges: Vec::new(),
        }
    }

    pub fn register_client(&mut self, client: u32) {
        self.clients
            .entry(client)
            .or_insert_with(|| ClientChains::new(client));
    }

    pub fn issued_count(&self, client: u32) -> u64 {
        self.clients.get(&client).map_or(0, |c| c.issued_count())
    }

    pub fn record_issue(&mut self, client: u32, sn: u64, op: u64) {
        self.sn_by_op.insert(op, (client, sn));
        self.clients
            .get_mut(&client)
            .expect("client registered")
            .record_issue(&Entry::Cmd(ClientCommand { client, sn, op }));
    }

    pub fn acked_sn(&self, client: u32) -> u64 {
        self.clients[&client].next_sn() - 1
    }

    pub fn client(&self, client: u32) -> &ClientChains {
        &self.clients[&client]
    }

    /// Feed forests from executed growth and deliver acks; call once after every step.
    pub fn observe_round(&mut self, state: &SmrState) {
        self.last_merges.clear();
        let executed = state.executed_seqs();
        for (i, seq) in executed.iter().enumerate() {
            if seq.len() < self.fed[i] {
                // Cert state is a pure function of the executed sequence, so a rewritten sequence rebuilds by replay.
                self.servers[i] = ServerCertState::new();
                self.fed[i] = 0;
            }
            for entry in &seq[self.fed[i]..] {
                let events = self.servers[i].append(entry);
                self.last_merges.extend(events.into_iter().map(|e| (i, e)));
            }
            self.fed[i] = seq.len();
        }
        self.deliver_acks(state);
    }

    pub fn observe_round_rec(&mut self, state: &SmrState) {
        self.last_merges.clear();
        for i in 0..self.servers.len() {
            let certs = state.checkpoint_certs(i).expect(
                "recovery cert state is spec-gated; set certs: true in the scenario \
                 (or build the state with SmrState::new_with_certs)",
            );
            self.fed[i] = certs.forest().len() as usize;
            self.servers[i] = certs.clone();
        }
        self.deliver_acks(state);
    }

    // §5 p. 28: the ack comes from the one server whose triage acked, snapshotted at ack time.
    fn deliver_acks(&mut self, state: &SmrState) {
        let round = state.round();
        let report = state.report();
        for c in &report.commands {
            let Some(&(client, sn)) = self.sn_by_op.get(&c.op) else {
                continue;
            };
            if c.committed_ack_round != Some(round) {
                continue;
            }
            // Reads attempts at the ack round itself; a lean+certs pairing panics here by design.
            let acker = c
                .attempts
                .iter()
                .find(|a| a.round == round && a.outcome == AttemptOutcome::AckCommitted)
                .map(|a| a.target as usize)
                .expect("ack round implies an AckCommitted attempt");
            let note = self.servers[acker].notification(client);
            self.clients
                .get_mut(&client)
                .expect("client registered")
                .on_commit_ack(sn, note);
        }
    }

    pub fn merges_last_round(&self) -> &[(usize, MergeEvent)] {
        &self.last_merges
    }

    pub fn server_states(&self) -> &[ServerCertState] {
        &self.servers
    }

    pub fn clients_iter(&self) -> impl Iterator<Item = &ClientChains> {
        self.clients.values()
    }

    pub fn roots_consistent(&self) -> bool {
        let mut by_m: BTreeMap<usize, Vec<protocol::certificates::Hash>> = BTreeMap::new();
        for (i, s) in self.servers.iter().enumerate() {
            match by_m.get(&self.fed[i]) {
                Some(roots) => {
                    if *roots != s.forest().roots() {
                        return false;
                    }
                }
                None => {
                    by_m.insert(self.fed[i], s.forest().roots());
                }
            }
        }
        true
    }

    pub fn verify_everywhere(&self, client: u32, cert: &Certificate, mask: &[bool]) -> VerifyTally {
        let mut tally = VerifyTally {
            covered: 0,
            accepted: 0,
        };
        for (i, s) in self.servers.iter().enumerate() {
            if mask[i] {
                continue;
            }
            let covered = match cert {
                Certificate::Chained { pos, .. } => self.fed[i] as u64 > *pos,
                Certificate::Newest { .. } => s.last_two(client).is_some(),
            };
            if !covered {
                continue;
            }
            tally.covered += 1;
            tally.accepted += usize::from(s.verify(client, cert));
        }
        tally
    }
}
