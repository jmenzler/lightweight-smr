//! World stages shared verbatim by the compact and recovery round steps; (b) and (d) stay per engine.

use crate::same_component;
use crate::smr::par_step_enabled;
use crate::smr::tracker::CommandTracker;
use protocol::compact::ClientCommand;
use rand::Rng;
use rand_chacha::ChaCha12Rng;

/// Stage (c): each admitted delivery, in ascending server id, sends `amp` append requests to uniform targets.
#[allow(clippy::too_many_arguments)]
pub(in crate::smr) fn amplify(
    rng: &mut ChaCha12Rng,
    trackers: &mut [CommandTracker],
    mut delivered: Vec<(usize, ClientCommand)>,
    admit: impl Fn(usize, &ClientCommand) -> bool,
    mask: &[bool],
    partition: Option<&[u32]>,
    n: usize,
    amp: usize,
    round: usize,
) -> Vec<Vec<(ClientCommand, u64)>> {
    let mut appends: Vec<Vec<(ClientCommand, u64)>> = vec![Vec::new(); n];
    delivered.sort_by_key(|&(server, _)| server);
    for &(server, cc) in &delivered {
        if admit(server, &cc) {
            let mut receivers: Vec<u32> = Vec::new();
            for _ in 0..amp {
                let target = rng.random_range(0..n);
                if !mask[target] && same_component(partition, server, target) {
                    appends[target].push((cc, round as u64));
                    receivers.push(target as u32);
                }
            }
            let tracker = trackers
                .iter_mut()
                .find(|t| t.cc == cc)
                .expect("tracked cc");
            if tracker.delivered_round == Some(round) && tracker.amp_receivers.is_empty() {
                receivers.sort_unstable();
                receivers.dedup();
                tracker.amp_receivers = receivers;
            }
        }
    }
    appends
}

/// Stage (e)'s RNG-free applies, in parallel under `SIM_PAR_STEP=on`.
pub(in crate::smr) fn apply_all<N: Send>(
    nodes: &mut [N],
    apply: impl Fn(usize, &mut N) + Sync + Send,
) {
    if par_step_enabled() {
        use rayon::prelude::*;
        nodes
            .par_iter_mut()
            .enumerate()
            .for_each(|(i, node)| apply(i, node));
    } else {
        for (i, node) in nodes.iter_mut().enumerate() {
            apply(i, node);
        }
    }
}
