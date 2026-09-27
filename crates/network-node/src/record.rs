//! Per-round JSONL record, one line per node per round on stdout.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PostState {
    Value(Option<u64>),
    Log {
        log_len: Option<usize>,
        exec_len: usize,
        log_hash: Option<u64>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackedDigest {
    pub op: u64,
    pub present: bool,
    pub pos: Option<usize>,
    pub prefix_hash: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoundRecord {
    pub r: u64,
    pub blocked: bool,
    /// Held a servable (non-⊥) snapshot entering the round.
    pub snap_held: bool,
    pub post: PostState,
    pub replies: u8,
    pub req_sent: u32,
    pub rep_sent: u32,
    pub rep_recv: u32,
    pub app_sent: u32,
    pub app_recv: u32,
    pub cli_recv: u32,
    pub acks: u32,
    pub bytes_out: u64,
    pub bytes_in: u64,
    pub late_rep: u32,
    pub late_app: u32,
    pub dropped_past: u32,
    /// Encode failures (oversized frame): a config fault, never protocol silence.
    pub encode_err: u32,
    /// (executed length, chained prefix hash) per entry executed this round; compact only.
    #[serde(default)]
    pub exec_prefix: Vec<(usize, u64)>,
    pub t_last_reply_us: u64,
    pub t_step_done_us: u64,
    /// Time spent in `RoundEngine::on_round_end()` alone.
    pub t_step_compute_us: u64,
    pub tracked: Vec<TrackedDigest>,
}

impl RoundRecord {
    pub fn to_jsonl(&self) -> String {
        serde_json::to_string(self).expect("record serializes")
    }

    pub fn from_jsonl(line: &str) -> Result<RoundRecord, String> {
        serde_json::from_str(line).map_err(|e| e.to_string())
    }
}
