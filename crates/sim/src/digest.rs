//! Canonical FNV-1a 64 digest over executed sequences; every crate must hash `Entry` with this encoding.

use protocol::compact::{ClientId, Entry};
use protocol::extended::Command;

pub const FNV_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

pub fn fold(mut h: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

/// Fold one entry into a running digest; chaining from `FNV_BASIS` equals `fnv64`.
pub fn fold_entry(h: u64, entry: &Entry) -> u64 {
    match entry {
        Entry::Cmd(cc) => {
            let h = fold(h, &[0]);
            let h = fold(h, &cc.client.to_le_bytes());
            let h = fold(h, &cc.sn.to_le_bytes());
            fold(h, &cc.op.to_le_bytes())
        }
        Entry::Null { client, sn } => {
            let h = fold(h, &[1]);
            let h = fold(h, &client.to_le_bytes());
            fold(h, &sn.to_le_bytes())
        }
        Entry::Nop(round) => {
            let h = fold(h, &[2]);
            fold(h, &round.to_le_bytes())
        }
    }
}

pub fn fnv64(entries: &[Entry]) -> u64 {
    entries.iter().fold(FNV_BASIS, fold_entry)
}

pub fn fnv64_iter<'a>(entries: impl Iterator<Item = &'a Entry>) -> u64 {
    entries.fold(FNV_BASIS, fold_entry)
}

pub fn fnv64_sn(sn: impl Iterator<Item = (ClientId, u64)>) -> u64 {
    let mut h = FNV_BASIS;
    for (client, committed) in sn {
        h = fold(h, &client.to_le_bytes());
        h = fold(h, &committed.to_le_bytes());
    }
    h
}

pub fn fnv64_ops(ops: impl Iterator<Item = Command>) -> u64 {
    ops.fold(FNV_BASIS, |h, op| fold(h, &op.to_le_bytes()))
}

pub fn hash_hex(entries: &[Entry]) -> String {
    format!("{:016x}", fnv64(entries))
}

#[cfg(test)]
mod tests;
