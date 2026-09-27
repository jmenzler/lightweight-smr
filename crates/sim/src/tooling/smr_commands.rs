//! Per-command landmark JSON in the node collector's commands.json shape.

use crate::smr::{Proto, SmrScenario, injection_sns, run_smr};
use crate::spec::SmrScenarioSpec;
use serde::Serialize;
use std::collections::BTreeMap;

pub fn commands_json(spec: SmrScenarioSpec) -> Result<String, String> {
    let scenario = SmrScenario::try_from(spec)?;
    let is_compact = matches!(scenario.proto, Proto::Compact { .. });

    // Same sn as the client driver: manual = per-client injection rank, traffic = 1.
    let sns = injection_sns(&scenario.injections);
    let op_to_sn: BTreeMap<u64, u64> = scenario
        .injections
        .iter()
        .zip(&sns)
        .map(|(inj, &sn)| (inj.op, sn))
        .collect();

    let report = run_smr(&scenario);
    if let Some(failure) = &report.failure {
        return Err(format!(
            "SMR failed during {:?}: attempted round {}, completed round {}, observed round {}, node {}",
            failure.phase,
            failure.attempted_round,
            failure.completed_round,
            failure.observed_round,
            failure.node,
        ));
    }

    let commands: Vec<CommandRow> = report
        .commands
        .iter()
        .map(|c| CommandRow {
            client: c.client,
            sn: if c.auto { 1 } else { op_to_sn[&c.op] },
            op: c.op,
            injection_round: c.injection_round as u64,
            delivered_round: c.delivered_round.map(|r| r as u64),
            committed_ack_round: c.committed_ack_round.map(|r| r as u64),
            terminal: if is_compact {
                c.committed_ack_round.is_some()
            } else {
                c.delivered_round.is_some()
            },
        })
        .collect();

    // Node digests only track manually-injected ops; mirror that here.
    let landmarks: Vec<LandmarkRow> = report
        .commands
        .iter()
        .filter(|c| !c.auto)
        .map(|c| LandmarkRow {
            op: c.op,
            all_logs_round: c.all_logs_round.map(|r| r as u64),
            prefix_fixed_round: c.prefix_fixed_round.map(|r| r as u64),
        })
        .collect();

    Ok(format!(
        "{{\"client\":{},\"landmarks\":{}}}",
        commands_line(&commands),
        to_json(&landmarks)
    ))
}

/// Field order is the commands.json key order.
#[derive(Serialize)]
pub struct CommandRow {
    pub client: u32,
    pub sn: u64,
    pub op: u64,
    pub injection_round: u64,
    pub delivered_round: Option<u64>,
    pub committed_ack_round: Option<u64>,
    pub terminal: bool,
}

/// Field order is the commands.json key order.
#[derive(Serialize)]
pub struct LandmarkRow {
    pub op: u64,
    pub all_logs_round: Option<u64>,
    pub prefix_fixed_round: Option<u64>,
}

/// The client driver's final stdout line: `{"commands":[...]}`.
pub fn commands_line(commands: &[CommandRow]) -> String {
    format!("{{\"commands\":{}}}", to_json(commands))
}

pub fn to_json<T: Serialize + ?Sized>(rows: &T) -> String {
    serde_json::to_string(rows).expect("rows serialize")
}
