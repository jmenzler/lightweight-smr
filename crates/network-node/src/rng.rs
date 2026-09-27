//! Per-role RNG streams: ChaCha12 seeded from length-framed SHA-256(run_seed || role || id).

use crate::NodeId;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha12Rng;
use sha2::{Digest, Sha256};

pub fn derive_rng(run_seed: u64, role: &str, id: u32) -> ChaCha12Rng {
    let mut h = Sha256::new();
    h.update(run_seed.to_le_bytes());
    h.update((role.len() as u64).to_le_bytes());
    h.update(role.as_bytes());
    h.update(id.to_le_bytes());
    ChaCha12Rng::from_seed(h.finalize().into())
}

/// k pull targets uniform over `0..n` with replacement, own id included.
pub fn draw_targets(rng: &mut ChaCha12Rng, n: usize, k: usize) -> Vec<NodeId> {
    (0..k).map(|_| rng.random_range(0..n) as NodeId).collect()
}
