//! Absolute positions of the entries of a prefix-consistent executed sequence.

use protocol::compact::Entry;
use std::collections::BTreeMap;

/// Never released: dropping entries would blind the duplicate-freedom oracle.
pub(in crate::smr) struct EntryIndex {
    pos: BTreeMap<Entry, u32>,
    covered: u64,
}

impl EntryIndex {
    pub(in crate::smr) fn new() -> Self {
        EntryIndex {
            pos: BTreeMap::new(),
            covered: 0,
        }
    }

    fn skip(&self, from: u64) -> usize {
        self.covered.saturating_sub(from) as usize
    }

    /// True if a newly covered entry was already indexed (a double execution); it keeps its first position.
    #[must_use]
    pub(in crate::smr) fn extend_unique(&mut self, from: u64, seq: &[Entry]) -> bool {
        let mut repeated = false;
        for (i, e) in seq.iter().enumerate().skip(self.skip(from)) {
            let at = (from + i as u64) as u32;
            match self.pos.entry(e.clone()) {
                std::collections::btree_map::Entry::Occupied(_) => repeated = true,
                std::collections::btree_map::Entry::Vacant(slot) => {
                    slot.insert(at);
                }
            }
        }
        self.covered = self.covered.max(from + seq.len() as u64);
        repeated
    }

    pub(in crate::smr) fn extend_first(&mut self, from: u64, seq: &[Entry]) {
        for (i, e) in seq.iter().enumerate().skip(self.skip(from)) {
            self.pos
                .entry(e.clone())
                .or_insert((from + i as u64) as u32);
        }
        self.covered = self.covered.max(from + seq.len() as u64);
    }

    pub(in crate::smr) fn heap_bytes(&self) -> u64 {
        ((self.pos.len() * (size_of::<Entry>() + size_of::<u32>())) as u64 * 3) / 2
    }

    pub(in crate::smr) fn get(&self, entry: &Entry) -> Option<u32> {
        self.pos.get(entry).copied()
    }
}

#[cfg(test)]
mod tests;
