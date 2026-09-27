//! On-demand per-node recovery state for the lab drilldown; never streamed.

pub(in crate::smr) use crate::digest::hash_hex;

/// One node's recovery state; the hashes are display shorthands, not the fork signal.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RecNodeDetail {
    pub node: u32,
    /// 0 no-reset · 1 reset · 2 ⊥, as in `NodeGlance::r`.
    pub r: u8,
    /// None = ⊥.
    pub log_len: Option<u32>,
    pub executed_len: u32,
    pub checkpoint_window: u64,
    /// None ⟺ the checkpoint's P is ⊥ (genesis), not an empty prefix.
    pub checkpoint_p_len: Option<u32>,
    pub s_hash: String,
    pub checkpoint_s_hash: String,
}
