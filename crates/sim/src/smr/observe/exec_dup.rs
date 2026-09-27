//! Double-execution oracle: a (client, sn) executed twice on one server.
//!
//! Prefix consistency (Def 1.4) cannot see a slot every server repeats, so this latch is kept
//! apart from `safety_ok`. Only `Entry::Cmd` counts: S_i holds nothing else, since ⊥ only sets
//! sn(c) and x_d is a no-op, so an executed command and a ⊥ for its slot are one execution.
//! A different op in an already executed slot does count.

use protocol::compact::{ClientId, Entry};
use protocol::shared_state::ExecutedView;
use rustc_hash::{FxHashMap, FxHashSet};

type Slot = (ClientId, u64);

pub(in crate::smr) struct ExecDupOracle {
    /// First canonical position of every slot; spans the whole verified order, never released.
    canon_first: FxHashMap<Slot, u64>,
    canon_covered: u64,
    /// Per-server slots, built only once the prefix latch is gone.
    nodes: Vec<Option<NodeSlots>>,
    tripped: Option<usize>,
}

/// Positions below `base` are the canonical order's; `slots` holds those in `[base, len)`.
struct NodeSlots {
    base: u64,
    len: u64,
    slots: FxHashSet<Slot>,
}

fn slot(e: &Entry) -> Option<Slot> {
    match e {
        Entry::Cmd(c) => Some((c.client, c.sn)),
        Entry::Null { .. } | Entry::Nop(_) => None,
    }
}

impl ExecDupOracle {
    pub(in crate::smr) fn new(n: usize) -> Self {
        ExecDupOracle {
            canon_first: FxHashMap::default(),
            canon_covered: 0,
            nodes: (0..n).map(|_| None).collect(),
            tripped: None,
        }
    }

    pub(in crate::smr) fn tripped_round(&self) -> Option<usize> {
        self.tripped
    }

    /// Recovery's canonical index found a repeated entry; the first round is kept.
    pub(in crate::smr) fn trip_at(&mut self, round: usize) {
        if self.tripped.is_none() {
            self.trip(round);
        }
    }

    fn trip(&mut self, round: usize) {
        self.tripped = Some(round);
        self.canon_first = FxHashMap::default();
        self.nodes.iter_mut().for_each(|n| *n = None);
    }

    /// While every executed sequence is a prefix of the canonical order, a server repeats a slot
    /// exactly when the canonical order does, in the same round.
    pub(in crate::smr) fn observe_canonical(
        &mut self,
        start: u64,
        retained: &[Entry],
        round: usize,
    ) {
        if self.tripped.is_some() {
            return;
        }
        assert!(
            start <= self.canon_covered,
            "the canonical order starts at {start}, past the {} positions the oracle saw",
            self.canon_covered
        );
        let skip = (self.canon_covered - start) as usize;
        for (i, e) in retained.iter().enumerate().skip(skip) {
            let at = start + i as u64;
            if let Some(s) = slot(e)
                && self.canon_first.insert(s, at).is_some()
            {
                self.trip(round);
                return;
            }
        }
        self.canon_covered = self.canon_covered.max(start + retained.len() as u64);
    }

    /// After the prefix latch: `replaced[i]` marks a server that may have adopted a peer's S
    /// this round (pre-step L_i = ⊥); every other server only appended.
    pub(in crate::smr) fn observe_nodes(
        &mut self,
        executed: &[ExecutedView<'_>],
        replaced: &[bool],
        round: usize,
    ) {
        if self.tripped.is_some() {
            return;
        }
        let canon_covered = self.canon_covered;
        let canon_first = &self.canon_first;
        for (i, view) in executed.iter().enumerate() {
            let node = match &mut self.nodes[i] {
                Some(node) if !replaced[i] => {
                    assert!(
                        view.offset() == node.base && view.logical_len() >= node.len,
                        "round {round}: server {i} changed its executed prefix without a ⊥ log"
                    );
                    node
                }
                vacant => vacant.insert(NodeSlots {
                    base: view.offset(),
                    len: view.offset(),
                    slots: FxHashSet::default(),
                }),
            };
            assert!(
                node.base <= canon_covered,
                "round {round}: server {i} forgot past the canonical order the oracle saw"
            );
            let base = node.base;
            let repeated = view
                .iter_from(node.len)
                .filter_map(slot)
                .any(|s| canon_first.get(&s).is_some_and(|&at| at < base) || !node.slots.insert(s));
            if repeated {
                self.trip(round);
                return;
            }
            node.len = view.logical_len();
        }
    }
}

#[cfg(test)]
mod tests;
