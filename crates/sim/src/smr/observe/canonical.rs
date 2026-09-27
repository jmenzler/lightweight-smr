//! Observer's canonical committed order and its position-anchored digest chain.

use crate::digest::{FNV_BASIS, fold, fold_entry};
use protocol::compact::{Entry, SharedState, Timed};
use protocol::recovery::Checkpoint;
use protocol::shared_state::ExecutedView;

fn extend(chain: u64, at: u64, entry: &Entry) -> u64 {
    fold_entry(fold(chain, &at.to_le_bytes()), entry)
}

pub(in crate::smr) fn timed_digest(entries: &[Timed]) -> u64 {
    entries.iter().fold(FNV_BASIS, |h, t| {
        fold_entry(fold(h, &t.round.to_le_bytes()), &t.entry)
    })
}

pub(in crate::smr) struct CanonicalOrder {
    start: u64,
    entries: Vec<Entry>,
    digests: Vec<u64>,
}

impl CanonicalOrder {
    pub(in crate::smr) fn new() -> Self {
        CanonicalOrder {
            start: 0,
            entries: Vec::new(),
            digests: vec![FNV_BASIS],
        }
    }

    pub(in crate::smr) fn len(&self) -> u64 {
        self.start + self.entries.len() as u64
    }

    pub(in crate::smr) fn start(&self) -> u64 {
        self.start
    }

    pub(in crate::smr) fn retained(&self) -> &[Entry] {
        &self.entries
    }

    fn index(&self, at: u64, what: &str) -> usize {
        assert!(
            at >= self.start,
            "canonical position {at} is below the released prefix (start {}) \
             while reading {what} — the observer released a position something \
             still reads, which the release line exists to rule out",
            self.start
        );
        assert!(
            at <= self.len(),
            "canonical position {at} is past the verified order ({}) while \
             reading {what}",
            self.len()
        );
        (at - self.start) as usize
    }

    pub(in crate::smr) fn slice(&self, from: u64, to: u64) -> &[Entry] {
        let (lo, hi) = (self.index(from, "a slice"), self.index(to, "a slice"));
        &self.entries[lo..hi]
    }

    pub(in crate::smr) fn digest_at(&self, at: u64) -> u64 {
        self.digests[self.index(at, "a digest")]
    }

    pub(in crate::smr) fn extend_from_slice(&mut self, entries: &[Entry]) {
        self.extend_from_iter(entries.iter());
    }

    pub(in crate::smr) fn extend_from_iter<'a>(
        &mut self,
        entries: impl Iterator<Item = &'a Entry>,
    ) {
        for entry in entries {
            let at = self.len();
            let chain = extend(*self.digests.last().expect("seeded"), at, entry);
            self.entries.push(entry.clone());
            self.digests.push(chain);
        }
    }

    pub(in crate::smr) fn release_to(&mut self, upto: u64) {
        let upto = upto.min(self.len());
        if upto <= self.start {
            return;
        }
        let drop = (upto - self.start) as usize;
        self.entries.drain(..drop);
        self.digests.drain(..drop);
        self.start = upto;
    }

    pub(in crate::smr) fn heap_bytes(&self) -> u64 {
        (self.entries.capacity() * size_of::<Entry>() + self.digests.capacity() * size_of::<u64>())
            as u64
    }

    #[cfg(test)]
    pub(in crate::smr) fn from_entries(entries: &[Entry]) -> Self {
        let mut order = CanonicalOrder::new();
        order.extend_from_slice(entries);
        order
    }
}

/// Chain value at `upto`: the state's own entries above its offset, the observer's value below.
pub(in crate::smr) fn state_digest_upto(
    canon: &CanonicalOrder,
    state: &SharedState,
    upto: u64,
) -> u64 {
    let ex = state.executed();
    let base = upto.min(ex.offset());
    let mut chain = canon.digest_at(base);
    ex.for_each_range(ex.offset(), upto, |at, entry| {
        chain = extend(chain, at, entry);
    });
    chain
}

pub(in crate::smr) fn state_digest(canon: &CanonicalOrder, state: &SharedState) -> u64 {
    state_digest_upto(canon, state, state.logical_len())
}

/// Checkpoint identity for recovery; `Checkpoint::eq` asserts on differing forget offsets.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::smr) struct ForkSignature {
    executed_len: u64,
    executed: u64,
    sn: u64,
    // None is ⊥, distinct from the empty prefix.
    pre: Option<(usize, u64)>,
}

impl ForkSignature {
    pub(in crate::smr) fn of(canon: &CanonicalOrder, checkpoint: &Checkpoint) -> Self {
        ForkSignature {
            executed_len: checkpoint.s.logical_len(),
            executed: state_digest(canon, &checkpoint.s),
            sn: crate::digest::fnv64_sn(checkpoint.s.sn_iter()),
            pre: checkpoint.p.as_ref().map(|p| (p.len(), timed_digest(p))),
        }
    }

    pub(in crate::smr) fn executed_len(&self) -> u64 {
        self.executed_len
    }

    pub(in crate::smr) fn executed(&self) -> u64 {
        self.executed
    }

    pub(in crate::smr) fn pre_len(&self) -> Option<usize> {
        self.pre.map(|(len, _)| len)
    }

    pub(in crate::smr) fn extends(
        &self,
        canon: &CanonicalOrder,
        checkpoint: &Checkpoint,
        upto: u64,
        mark: u64,
    ) -> bool {
        self.executed_len >= upto && state_digest_upto(canon, &checkpoint.s, upto) == mark
    }
}

#[derive(Clone, Copy)]
pub(in crate::smr) struct Executed<'a> {
    pub(in crate::smr) view: ExecutedView<'a>,
    pub(in crate::smr) canonical: &'a CanonicalOrder,
}

impl<'a> Executed<'a> {
    pub(in crate::smr) fn position_from(&self, entry: &Entry, from: u64) -> Option<u32> {
        let offset = self.view.offset();
        if from < offset {
            let lo = from.max(self.canonical.start());
            let hi = offset.min(self.canonical.len());
            if lo < hi
                && let Some(p) = self.canonical.slice(lo, hi).iter().position(|e| e == entry)
            {
                return Some((lo + p as u64) as u32);
            }
        }
        self.view.position_from(entry, from).map(|p| p as u32)
    }

    pub(in crate::smr) fn front_scan(&self, entry: &Entry) -> Option<u32> {
        self.position_from(entry, 0)
    }
}

#[cfg(test)]
mod tests;
