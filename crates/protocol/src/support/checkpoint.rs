//! The log and state operations Algorithm 6 names on a checkpoint's prefix P:
//! L_i := P, execute P on S_i, remove P from L_i, and the aged prefix P_i.

use crate::chunked::ChunkSeq;
use crate::compact::{Entry, Timed, aged_prefix_len, x_d};
use crate::log::Log;
use crate::support::shared_state::{
    CLONE_SITE_RECOVERY_COW_EXECUTE, RepeatedCommit, SharedState, cow_state,
};
use std::sync::Arc;

/// P is not a prefix of L_i at a window boundary, which the paper rules out w.h.p. (Lemma 6.7).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RecoveryPrefixViolation {
    pub pre_len: usize,
    pub log_len: usize,
    pub first_mismatch: usize,
    pub expected: Option<Timed>,
    pub actual: Option<Timed>,
    pub command_prefix_ok: bool,
}

impl std::fmt::Display for RecoveryPrefixViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "P must be a prefix of L: strip would corrupt the log (pre len {}, log len {}, first mismatch {}, expected {:?}, actual {:?}, cmd_ok={})",
            self.pre_len,
            self.log_len,
            self.first_mismatch,
            self.expected,
            self.actual,
            self.command_prefix_ok,
        )
    }
}

impl std::error::Error for RecoveryPrefixViolation {}

pub(crate) fn validate_prefix<'a>(
    pre: &[Timed],
    log_len: usize,
    mut log: impl Iterator<Item = &'a Timed>,
) -> Result<(), RecoveryPrefixViolation> {
    let mut first_mismatch = None;
    let mut command_prefix_ok = log_len >= pre.len();
    for (index, expected) in pre.iter().enumerate() {
        let actual = log.next();
        if actual != Some(expected) && first_mismatch.is_none() {
            first_mismatch = Some((index, expected.clone(), actual.cloned()));
        }
        if actual.map(|t| &t.entry) != Some(&expected.entry) {
            command_prefix_ok = false;
        }
    }
    let Some((first_mismatch, expected, actual)) = first_mismatch else {
        return Ok(());
    };
    Err(RecoveryPrefixViolation {
        pre_len: pre.len(),
        log_len,
        first_mismatch,
        expected: Some(expected),
        actual,
        command_prefix_ok,
    })
}

/// Panics unless P is a prefix of L_i: a violation must abort visibly, not corrupt the log.
pub(crate) fn assert_prefix(p: &[Timed], log: &ChunkSeq<Timed>, round: u64) {
    if let Err(violation) = validate_prefix(p, log.len(), log.iter()) {
        panic!("{violation} at round {round}");
    }
}

/// L_i := P with its carried index rebuilt; P = ⊥ gives L_i = ⊥.
pub(crate) fn log_from_prefix(p: Option<&[Timed]>) -> Option<Log> {
    p.map(|p| Log::from_entries(p.to_vec()))
}

/// Executes P on S_i and returns how many entries the execute-once state machine left out.
/// A P of no-ops only leaves S_i's allocation untouched.
pub(crate) fn execute_prefix(
    state: &mut Arc<SharedState>,
    p: &[Timed],
    repeated: RepeatedCommit,
) -> u64 {
    if !p.iter().any(|t| !matches!(t.entry, Entry::Nop(_))) {
        return 0;
    }
    let state = cow_state(state, CLONE_SITE_RECOVERY_COW_EXECUTE);
    p.iter()
        .filter(|t| !state.commit(&t.entry, repeated))
        .count() as u64
}

/// Removes the first `len` entries (P) from L_i; an emptied log gets the dummy x_d.
pub(crate) fn strip_prefix(log: &mut Log, len: usize, round: u64) {
    log.strip_front(len, x_d(round));
}

/// P_i: the longest prefix of `log` whose entries are all at least `t` rounds old.
pub(crate) fn aged_prefix(log: &[Timed], round: u64, t: u64) -> Vec<Timed> {
    log[..aged_prefix_len(log, round, t)].to_vec()
}
