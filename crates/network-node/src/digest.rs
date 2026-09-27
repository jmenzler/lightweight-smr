//! Log digests; the entry encoding is `sim::digest`'s so sim and node digests stay comparable.

use protocol::compact::Timed;
use sim::digest::{FNV_BASIS, fold, fold_entry};

pub use sim::digest::{
    fnv64, fnv64_iter as fnv_entries, fnv64_ops as fnv_ops, fold_entry as fnv_chain,
};

/// Hash of a compact log including each entry's attached age, which is part of log identity.
pub fn fnv_timed<'a>(log: impl Iterator<Item = &'a Timed>) -> u64 {
    log.fold(FNV_BASIS, |h, t| {
        fold(fold_entry(h, &t.entry), &t.round.to_le_bytes())
    })
}
