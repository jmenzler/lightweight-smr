use crate::helpers::{chain_snapshot, merge_json, peaks_json, recorded_schedule};
use sim::smr::{ManualBlock, SmrScenario, SmrState, SmrStepStatus, run_smr};
use sim::spec::{InjectionSpec, SmrScenarioSpec, TrafficPhaseSpec, TrafficSpec};
use std::collections::BTreeMap;
use wasm_bindgen::prelude::wasm_bindgen;

/// Run a full SMR scenario from spec JSON; returns `{report}` or `{error}` JSON.
#[wasm_bindgen]
pub fn run_smr_json(spec_json: &str) -> String {
    match run_smr_inner(spec_json) {
        Ok(json) => json,
        Err(error) => serde_json::json!({ "error": error }).to_string(),
    }
}

fn run_smr_inner(spec_json: &str) -> Result<String, String> {
    let spec: SmrScenarioSpec = serde_json::from_str(spec_json).map_err(|e| e.to_string())?;
    let mut scenario = SmrScenario::try_from(spec)?;
    // certs mount on the parsed value, never the spec, so the same spec replays gated off byte-identically
    scenario.certs = true;
    let report = run_smr(&scenario);
    assert_full_scope(&report);
    Ok(serde_json::json!({ "report": report }).to_string())
}

fn assert_full_scope(report: &sim::smr::SmrReport) {
    assert!(
        report
            .commands
            .iter()
            .all(|c| c.spread.observed().is_some()),
        "the lab was handed a report whose position census was scoped out; the \
         curve would render as zero rather than as absent"
    );
}

/// Stepwise SMR session whose history and injections replay byte-identically via `run_smr_json`.
#[wasm_bindgen]
pub struct LiveSmr {
    state: SmrState,
    base: SmrScenarioSpec,
    fractions: Vec<f64>,
    last: Option<SmrStepStatus>,
    sticky: bool,
    background: f64,
    mask: Vec<bool>,
    overlay: Vec<bool>,
    block_events: Vec<ManualBlock>,
    certs: Option<sim::certs::CertHarness>,
    certs_from_checkpoints: bool,
    captured: BTreeMap<u32, (u64, protocol::certificates::Certificate)>,
}

#[wasm_bindgen]
impl LiveSmr {
    /// Spec's `schedule` is ignored: no up-front mask draw may touch the RNG stream.
    #[wasm_bindgen(constructor)]
    pub fn new(spec_json: &str) -> Result<LiveSmr, String> {
        LiveSmr::new_with_mode(spec_json, false)
    }

    /// Same sticky semantics as `LiveSim::new_with_mode`.
    pub fn new_with_mode(spec_json: &str, sticky: bool) -> Result<LiveSmr, String> {
        let spec: SmrScenarioSpec = serde_json::from_str(spec_json).map_err(|e| e.to_string())?;
        let scenario = SmrScenario::try_from(spec.clone())?;
        if !scenario.client_model.is_unique() {
            return Err("client_model pool is batch-only: the lab has no pool support".into());
        }
        let background = match (sticky, &scenario.schedule) {
            (true, sim::BlockSchedule::FreshPerRound { fraction }) => *fraction,
            _ => 0.0,
        };
        let mask = vec![false; scenario.n];
        // certs on here, never in `self.base`, so the exported spec replays gated off
        let state = SmrState::new_with_certs(
            scenario.n,
            scenario.cfg,
            scenario.proto,
            scenario.sigma,
            scenario.seed,
            &scenario.injections,
            scenario.traffic.as_deref().unwrap_or(&[]),
            scenario.client_model,
            scenario.partition.as_deref(),
            true,
            scenario.merge_policy,
            scenario.repeated_commit,
        );
        let certs_from_checkpoints = matches!(
            scenario.proto,
            sim::smr::Proto::Recovery {
                resend_until_acked: true,
                ..
            }
        );
        let has_certs =
            certs_from_checkpoints || matches!(scenario.proto, sim::smr::Proto::Compact { .. });
        let certs = has_certs.then(|| {
            let mut harness = sim::certs::CertHarness::new(scenario.n);
            let sns = sim::smr::injection_sns(&scenario.injections);
            for (inj, sn) in scenario.injections.iter().zip(sns) {
                harness.register_client(inj.client);
                harness.record_issue(inj.client, sn, inj.op);
            }
            harness
        });
        Ok(LiveSmr {
            state,
            base: spec,
            fractions: Vec::new(),
            last: None,
            sticky,
            background,
            overlay: vec![false; scenario.n],
            block_events: Vec::new(),
            mask,
            certs,
            certs_from_checkpoints,
            captured: BTreeMap::new(),
        })
    }

    /// Advance one round at `fraction` blocking; no-op once dead so the replay length matches.
    pub fn step(&mut self, fraction: f64) -> String {
        if let Some(last) = self.last.clone()
            && (last.dead.is_some() || last.failed())
        {
            return status_json(&last, true);
        }
        // stage (a0) precedes every mask draw so both arms stay on the batch stream
        self.state.draw_arrivals();
        let count = self.state.blocked_count(fraction);
        let background_count = self.state.blocked_count(self.background);
        let mut mask = if self.sticky {
            self.state
                .sticky_round_mask(&mut self.mask, count, background_count)
        } else {
            self.state.sample_mask(count)
        };
        // the overlay unions after every draw, so a manual block leaves the RNG stream untouched
        for (m, &blocked) in mask.iter_mut().zip(&self.overlay) {
            *m |= blocked;
        }
        let status = self.state.step_masked(&mask);
        self.fractions.push(fraction);
        if !status.failed()
            && let Some(certs) = &mut self.certs
        {
            if self.certs_from_checkpoints {
                certs.observe_round_rec(&self.state);
            } else {
                certs.observe_round(&self.state);
            }
        }
        let json = status_json(&status, false);
        self.last = Some(status);
        json
    }

    fn failed_action(&self, action: &str) -> Option<String> {
        self.state.failure().map(|_| {
            serde_json::json!({
                "error": format!("failed session is immutable; cannot {action}")
            })
            .to_string()
        })
    }

    /// Submit a command live (lands next round); a pinned `target` deviates from the paper's random-server client.
    pub fn inject(&mut self, client: u32, op: u64, target: Option<u32>) -> String {
        if let Some(error) = self.failed_action("inject") {
            return error;
        }
        match self.state.inject(client, op, target) {
            Ok(round) => {
                if let Some(certs) = &mut self.certs {
                    certs.register_client(client);
                    let sn = certs.issued_count(client) + 1;
                    certs.record_issue(client, sn, op);
                }
                serde_json::json!({ "round": round }).to_string()
            }
            Err(error) => serde_json::json!({ "error": error }).to_string(),
        }
    }

    /// Block or release one node by hand; lands next round so the export still replays.
    pub fn set_block(&mut self, node: u32, blocked: bool) -> String {
        if let Some(error) = self.failed_action("change blocking") {
            return error;
        }
        let Some(slot) = self.overlay.get_mut(node as usize) else {
            return serde_json::json!({
                "error": format!("node {node} out of range for n={}", self.overlay.len())
            })
            .to_string();
        };
        let round = self.state.round() + 1;
        if *slot != blocked {
            *slot = blocked;
            if blocked {
                self.block_events.push(ManualBlock {
                    node,
                    from_round: round,
                    to_round: None,
                });
            } else if let Some(open) = self
                .block_events
                .iter()
                .rposition(|b| b.node == node && b.to_round.is_none())
            {
                if self.block_events[open].from_round == round {
                    self.block_events.remove(open);
                } else {
                    self.block_events[open].to_round = Some(round);
                }
            }
        }
        serde_json::json!({ "round": round }).to_string()
    }

    /// Live arrival-pmf edit; applies from the next round and replaces pending and future phases.
    pub fn set_traffic(&mut self, pmf_json: &str) -> String {
        if let Some(error) = self.failed_action("change traffic") {
            return error;
        }
        match serde_json::from_str::<Vec<f64>>(pmf_json)
            .map_err(|e| e.to_string())
            .and_then(|pmf| self.state.set_traffic_pmf(pmf))
        {
            Ok(()) => serde_json::json!({ "ok": true }).to_string(),
            Err(error) => serde_json::json!({ "error": error }).to_string(),
        }
    }

    /// The full report so far (metrics, per-command audit trails, terminal).
    pub fn report_json(&self) -> String {
        let report = self.state.report();
        assert_full_scope(&report);
        serde_json::to_string(&report).expect("report serializes")
    }

    pub fn node_detail(&self, node: u32) -> String {
        if let Some(error) = self.failed_action("read node detail") {
            return error;
        }
        match self.state.rec_node_detail(node as usize) {
            Some(detail) => serde_json::to_string(&detail).expect("detail serializes"),
            None => serde_json::json!({
                "error": "node detail needs the recovery rule and a node id below n"
            })
            .to_string(),
        }
    }

    /// The §6 committed sequence as a Merkle forest snapshot, delta-encoded from `from_len`.
    pub fn chain_json(&self, from_len: u64) -> String {
        if let Some(error) = self.failed_action("read the committed chain") {
            return error;
        }
        let Some(entries) = self.state.committed_entries() else {
            return serde_json::json!({
                "error": "the committed chain needs the recovery rule (Alg 6)"
            })
            .to_string();
        };
        if !self.state.safety_ok() {
            return serde_json::json!({
                "error": "chain frozen: prefix consistency violated"
            })
            .to_string();
        }
        chain_snapshot(&entries, from_len)
    }

    fn certs_or_error(&self) -> Result<&sim::certs::CertHarness, String> {
        self.certs.as_ref().ok_or_else(|| {
            match self.base.proto {
                sim::spec::ProtoSpec::Recovery { .. } => {
                    "recovery certificates need the resend-until-acked client (RQ11)"
                }
                _ => "certificates need the compact rule (extended has no executed state)",
            }
            .to_string()
        })
    }

    /// Forest + certificate snapshot for the live drawer (pure read).
    pub fn certs_json(&self) -> String {
        if let Some(error) = self.failed_action("read certificates") {
            return error;
        }
        let certs = match self.certs_or_error() {
            Ok(certs) => certs,
            Err(error) => return serde_json::json!({ "error": error }).to_string(),
        };
        // a prefix-inconsistent population must not have its attestations drawn as sound
        if !self.state.safety_ok() {
            return serde_json::json!({
                "error": "certificates frozen: prefix consistency violated"
            })
            .to_string();
        }
        let servers: Vec<serde_json::Value> = certs
            .server_states()
            .iter()
            .enumerate()
            .map(|(i, s)| {
                serde_json::json!({
                    "server": i,
                    "m": s.forest().len(),
                    "peaks": peaks_json(s.forest()),
                })
            })
            .collect();
        let merges: Vec<serde_json::Value> = certs
            .merges_last_round()
            .iter()
            .map(|(server, e)| {
                let mut merge = merge_json(e);
                merge["server"] = serde_json::json!(server);
                merge
            })
            .collect();
        let clients: Vec<serde_json::Value> = certs
            .clients_iter()
            .map(|c| {
                let acked = c.next_sn() - 1;
                let rows: Vec<serde_json::Value> = (1..=c.issued_count())
                    .map(|sn| {
                        let (pos, chain_len) = match c.chain_for(sn) {
                            Some((pos, chain)) => {
                                (serde_json::json!(pos), serde_json::json!(chain.len()))
                            }
                            None => (serde_json::Value::Null, serde_json::Value::Null),
                        };
                        serde_json::json!({
                            "sn": sn,
                            "pos": pos,
                            "chain_len": chain_len,
                            "committed": sn <= acked,
                        })
                    })
                    .collect();
                let stale = match self.captured.get(&c.client()) {
                    Some((sn, cert)) => {
                        let t = certs.verify_everywhere(c.client(), cert, &self.mask);
                        serde_json::json!({
                            "sn": sn,
                            "accepted": t.accepted,
                            "covered": t.covered,
                        })
                    }
                    None => serde_json::Value::Null,
                };
                serde_json::json!({
                    "client": c.client(),
                    "next_sn": c.next_sn(),
                    "certs": rows,
                    "stale": stale,
                })
            })
            .collect();
        // first max-m server: must match the lab's JS `best` tie-break (first wins)
        let best = certs
            .server_states()
            .iter()
            .enumerate()
            .max_by_key(|(i, s)| (s.forest().len(), std::cmp::Reverse(*i)))
            .map(|(i, _)| i)
            .unwrap_or(0);
        let committed: Vec<serde_json::Value> = self
            .state
            .executed_entries(best)
            .into_iter()
            .flat_map(protocol::shared_state::ExecSeq::iter)
            .map(|entry| match entry {
                protocol::compact::Entry::Cmd(cc) => {
                    serde_json::json!({ "kind": "cmd", "client": cc.client, "sn": cc.sn })
                }
                protocol::compact::Entry::Null { client, sn } => {
                    serde_json::json!({ "kind": "null", "client": client, "sn": sn })
                }
                protocol::compact::Entry::Nop(_) => serde_json::json!({ "kind": "nop" }),
            })
            .collect();
        serde_json::json!({
            "roots_consistent": certs.roots_consistent(),
            "servers": servers,
            "merges_last_step": merges,
            "clients": clients,
            "committed_server": best,
            "committed": committed,
        })
        .to_string()
    }

    /// Freeze the client's current bare-newest certificate for the §5 staleness demo.
    pub fn capture_stale(&mut self, client: u32) -> String {
        if let Some(error) = self.failed_action("capture a certificate") {
            return error;
        }
        let certs = match self.certs_or_error() {
            Ok(certs) => certs,
            Err(error) => return serde_json::json!({ "error": error }).to_string(),
        };
        if certs.clients_iter().all(|c| c.client() != client) {
            return serde_json::json!({ "error": format!("unknown client {client}") }).to_string();
        }
        let acked = certs.acked_sn(client);
        if acked == 0 {
            return serde_json::json!({
                "error": format!("client {client} has no committed command to capture yet")
            })
            .to_string();
        }
        let cert = certs
            .client(client)
            .build_certificate(acked)
            .expect("issued commands keep their entry");
        self.captured.insert(client, (acked, cert));
        serde_json::json!({ "sn": acked }).to_string()
    }

    /// On-demand verify-everywhere tallies for every issued certificate of every client.
    pub fn verify_json(&self) -> String {
        if let Some(error) = self.failed_action("verify certificates") {
            return error;
        }
        let certs = match self.certs_or_error() {
            Ok(certs) => certs,
            Err(error) => return serde_json::json!({ "error": error }).to_string(),
        };
        let round = self.last.as_ref().map(|s| s.round).unwrap_or(0);
        let clients: Vec<serde_json::Value> = certs
            .clients_iter()
            .map(|c| {
                let rows: Vec<serde_json::Value> = (1..=c.issued_count())
                    .map(|sn| {
                        let tally = match c.build_certificate(sn) {
                            Some(cert) => {
                                let t = certs.verify_everywhere(c.client(), &cert, &self.mask);
                                serde_json::json!({
                                    "accepted": t.accepted,
                                    "covered": t.covered,
                                })
                            }
                            None => serde_json::Value::Null,
                        };
                        serde_json::json!({ "sn": sn, "tally": tally })
                    })
                    .collect();
                serde_json::json!({ "client": c.client(), "rows": rows })
            })
            .collect();
        serde_json::json!({ "round": round, "clients": clients }).to_string()
    }

    pub fn export_scenario(&self) -> String {
        let mut spec = self.base.clone();
        spec.injections = self
            .state
            .manual_injections()
            .iter()
            .map(|i| InjectionSpec {
                round: i.round,
                client: i.client,
                op: i.op,
                target: i.target,
            })
            .collect();
        spec.traffic = match self.state.traffic_phases() {
            [] => None,
            [only] if only.from_round == 1 => Some(TrafficSpec::Pmf {
                arrivals_pmf: only.arrivals_pmf.clone(),
            }),
            phases => Some(TrafficSpec::Phases {
                phases: phases
                    .iter()
                    .map(|p| TrafficPhaseSpec {
                        from_round: p.from_round,
                        arrivals_pmf: p.arrivals_pmf.clone(),
                    })
                    .collect(),
            }),
        };
        spec.schedule = Some(recorded_schedule(
            self.sticky,
            &self.fractions,
            self.background,
        ));
        spec.manual_blocks = self.block_events.clone();
        spec.max_rounds = self.fractions.len();
        serde_json::to_string(&spec).expect("spec serializes")
    }
}

fn status_json(status: &SmrStepStatus, absorbed: bool) -> String {
    let mut value = serde_json::to_value(status).expect("status serializes");
    value["absorbed"] = serde_json::json!(absorbed);
    value.to_string()
}
