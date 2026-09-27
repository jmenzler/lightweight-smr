//! In-memory N-engine harness: perfect network, every message delivered within the round.

use crate::engine::RoundEngine;
use crate::record::RoundRecord;
use crate::spec::NodeRunSpec;
use crate::wire::Msg;

pub fn run(spec: &NodeRunSpec) -> Vec<Vec<RoundRecord>> {
    let n = spec.n();
    let mut engines: Vec<RoundEngine> = (0..n as u32)
        .map(|id| RoundEngine::new(spec, id).expect("engine"))
        .collect();
    let mut records: Vec<Vec<RoundRecord>> = vec![Vec::new(); n];
    for _ in 0..spec.max_rounds() {
        let mut in_flight: Vec<(u32, Msg)> = Vec::new();
        for e in engines.iter_mut() {
            in_flight.extend(e.on_round_start());
        }
        while let Some((to, m)) = in_flight.pop() {
            in_flight.extend(engines[to as usize].on_message(m));
        }
        for (id, e) in engines.iter_mut().enumerate() {
            records[id].push(e.on_round_end());
        }
    }
    records
}
