//! Census equivalence counters: `WINDOWS` under both scopings, `POINTS` only when computed.

use std::sync::atomic::{AtomicU64, Ordering};

static WINDOWS: AtomicU64 = AtomicU64::new(0);
static POINTS: AtomicU64 = AtomicU64::new(0);
static NODE_VISITS: AtomicU64 = AtomicU64::new(0);

pub(in crate::smr) fn window() {
    WINDOWS.fetch_add(1, Ordering::Relaxed);
}

/// `node_visits` must be counted by the fold (⊥ filtered), never taken from `nodes.len()`.
pub(in crate::smr) fn point(node_visits: usize) {
    POINTS.fetch_add(1, Ordering::Relaxed);
    NODE_VISITS.fetch_add(node_visits as u64, Ordering::Relaxed);
}

/// `(windows, points, node visits)` since process start.
pub fn census_counts() -> (u64, u64, u64) {
    (
        WINDOWS.load(Ordering::Relaxed),
        POINTS.load(Ordering::Relaxed),
        NODE_VISITS.load(Ordering::Relaxed),
    )
}
