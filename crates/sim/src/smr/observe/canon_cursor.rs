//! Executed-position lookup through the canonical index; exact only while executed sequences stay prefix-consistent.

use protocol::shared_state::ExecutedView;

/// Floor-only: positions come from the shared canonical index.
pub(in crate::smr) struct CanonCursor {
    floor: u32,
}

impl CanonCursor {
    pub(in crate::smr) fn new(floor: u32) -> Self {
        CanonCursor { floor }
    }

    /// The frontier may not pass any live cursor's floor.
    pub(in crate::smr) fn floor(&self) -> u32 {
        self.floor
    }
}

pub(in crate::smr) fn canon_lookup(hit: Option<u32>, ex: ExecutedView<'_>) -> Option<u32> {
    hit.filter(|&p| u64::from(p) < ex.logical_len())
}

#[cfg(test)]
mod tests;
