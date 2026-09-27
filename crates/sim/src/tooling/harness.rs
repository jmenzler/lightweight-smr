//! Shared scaffolding for the experiment runner bins.

use crate::smr::{ClientModel, Injection, Proto, SmrScenario};
use crate::{BlockSchedule, Config};

pub fn injection(round: usize, client: u32, op: u64) -> Injection {
    Injection {
        round,
        client,
        op,
        target: None,
    }
}

pub fn concurrent_injections(round: usize, count: u32, op_base: u64) -> Vec<Injection> {
    (0..count)
        .map(|c| injection(round, c + 1, op_base + u64::from(c)))
        .collect()
}

pub fn fresh_extended(
    n: usize,
    seed: u64,
    sigma: f64,
    beta: f64,
    injections: Vec<Injection>,
    max_rounds: usize,
) -> SmrScenario {
    SmrScenario {
        n,
        seed,
        cfg: Config::default(),
        sigma,
        proto: Proto::Extended,
        injections,
        max_rounds,
        schedule: BlockSchedule::FreshPerRound { fraction: beta },
        traffic: None,
        client_model: ClientModel::Unique,
        certs: false,
        manual_blocks: Vec::new(),
        partition: None,
        halt_on_violation: false,
        merge_policy: Default::default(),
        repeated_commit: Default::default(),
    }
}
