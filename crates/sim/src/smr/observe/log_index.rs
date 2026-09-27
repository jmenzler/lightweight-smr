//! Per-round inversion of the log scans: entry → the nodes holding it.

use protocol::chunked::ChunkSeq;
use protocol::compact::{Entry, Timed};
use rustc_hash::{FxHashMap, FxHasher};
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};

static HOLDER_VECS: AtomicU64 = AtomicU64::new(0);

/// Holder `Vec`s allocated since process start. Zero under a count-only build.
pub fn holder_vec_count() -> u64 {
    HOLDER_VECS.load(Ordering::Relaxed)
}

enum Index {
    Holders(FxHashMap<Entry, Vec<(u32, u32)>>),
    Counts(FxHashMap<Entry, (u32, u32)>),
}

pub(in crate::smr) struct LogIndex {
    index: Index,
    live: u32,
}

impl LogIndex {
    /// Index one round's logs by content (`None` = ⊥) and count distinct logs in the same walk.
    pub(in crate::smr) fn build_with_distinct<'a>(
        logs: impl Iterator<Item = Option<&'a ChunkSeq<Timed>>>,
        with_positions: bool,
    ) -> (Self, u32) {
        let mut holders: FxHashMap<Entry, Vec<(u32, u32)>> = FxHashMap::default();
        let mut counts: FxHashMap<Entry, (u32, u32)> = FxHashMap::default();
        // Content keys only: equal logs stripped differently count as one distinct log.
        let mut buckets: FxHashMap<(usize, u64), Vec<&'a ChunkSeq<Timed>>> = FxHashMap::default();
        let mut live = 0;
        let mut distinct = 0;
        for (id, log) in logs.enumerate() {
            let Some(log) = log else { continue };
            live += 1;
            let id = id as u32;
            let mut content = FxHasher::default();
            for (pos, t) in log.iter().enumerate() {
                let key = t.entry.clone();
                key.hash(&mut content);
                content.write_u64(t.round);
                if with_positions {
                    let ids = holders.entry(key).or_insert_with(|| {
                        HOLDER_VECS.fetch_add(1, Ordering::Relaxed);
                        Vec::new()
                    });
                    if ids.last().map(|&(node, _)| node) != Some(id) {
                        ids.push((id, pos as u32));
                    }
                } else {
                    let slot = counts.entry(key).or_insert((0, u32::MAX));
                    if slot.1 != id {
                        slot.0 += 1;
                        slot.1 = id;
                    }
                }
            }
            let bucket = buckets.entry((log.len(), content.finish())).or_default();
            if !bucket.contains(&log) {
                bucket.push(log);
                distinct += 1;
            }
        }
        let index = if with_positions {
            Index::Holders(holders)
        } else {
            Index::Counts(counts)
        };
        (LogIndex { index, live }, distinct)
    }

    #[cfg(test)]
    pub(in crate::smr) fn build<'a>(
        logs: impl Iterator<Item = Option<&'a ChunkSeq<Timed>>>,
    ) -> Self {
        let mut holders: FxHashMap<Entry, Vec<(u32, u32)>> = FxHashMap::default();
        let mut live = 0;
        for (id, log) in logs.enumerate() {
            let Some(log) = log else { continue };
            live += 1;
            let id = id as u32;
            for (pos, t) in log.iter().enumerate() {
                let ids = holders.entry(t.entry.clone()).or_default();
                if ids.last().map(|&(node, _)| node) != Some(id) {
                    ids.push((id, pos as u32));
                }
            }
        }
        LogIndex {
            index: Index::Holders(holders),
            live,
        }
    }

    pub(in crate::smr) fn holder_ids(&self, entry: &Entry) -> Vec<u32> {
        self.entry_holders(entry)
            .map_or_else(Vec::new, |ids| ids.iter().map(|&(node, _)| node).collect())
    }

    pub(in crate::smr) fn entry_holders(&self, entry: &Entry) -> Option<&[(u32, u32)]> {
        match &self.index {
            Index::Holders(h) => h.get(entry).map(Vec::as_slice),
            Index::Counts(_) => panic!(
                "holder positions were not built by this runner; they have no \
                 consumer under a scoped-out census, so asking for one is a bug"
            ),
        }
    }

    pub(in crate::smr) fn count(&self, entry: &Entry) -> u32 {
        match &self.index {
            Index::Holders(h) => h.get(entry).map_or(0, |ids| ids.len() as u32),
            Index::Counts(c) => c.get(entry).map_or(0, |&(n, _)| n),
        }
    }

    pub(in crate::smr) fn covers_all(&self, entry: &Entry) -> bool {
        self.live > 0 && self.count(entry) == self.live
    }
}

pub(in crate::smr) fn holder_pos(ids: &[(u32, u32)], node: u32) -> Option<u32> {
    ids.binary_search_by_key(&node, |&(id, _)| id)
        .ok()
        .map(|i| ids[i].1)
}

pub(in crate::smr) fn log_window_pos(
    holders: Option<&[(u32, u32)]>,
    node: u32,
    log: &ChunkSeq<Timed>,
    entry: &Entry,
    executed_len: u64,
) -> Option<u32> {
    let hit = holders.and_then(|ids| holder_pos(ids, node));
    debug_assert_eq!(
        hit,
        log.iter().position(|t| &t.entry == entry).map(|j| j as u32),
        "node {node}: indexed log position differs from the scan"
    );
    hit.map(|j| (executed_len + u64::from(j)) as u32)
}

pub(in crate::smr) struct LogCounts {
    counts: FxHashMap<Entry, (u32, u32)>,
}

impl LogCounts {
    pub(in crate::smr) fn build<'a>(
        logs: impl Iterator<Item = Option<&'a ChunkSeq<Timed>>>,
    ) -> Self {
        let mut counts: FxHashMap<Entry, (u32, u32)> = FxHashMap::default();
        for (id, log) in logs.enumerate() {
            let Some(log) = log else { continue };
            let id = id as u32;
            for t in log.iter() {
                let slot = counts.entry(t.entry.clone()).or_insert((0, u32::MAX));
                if slot.1 != id {
                    slot.0 += 1;
                    slot.1 = id;
                }
            }
        }
        LogCounts { counts }
    }

    pub(in crate::smr) fn count(&self, entry: &Entry) -> u32 {
        self.counts.get(entry).map_or(0, |&(c, _)| c)
    }
}

#[cfg(test)]
pub(in crate::smr) fn distinct_log_count<'a>(
    logs: impl Iterator<Item = &'a ChunkSeq<Timed>>,
) -> u32 {
    let mut buckets: FxHashMap<u64, Vec<&'a ChunkSeq<Timed>>> = FxHashMap::default();
    let mut distinct = 0;
    for log in logs {
        let mut h = FxHasher::default();
        for t in log.iter() {
            t.entry.hash(&mut h);
            h.write_u64(t.round);
        }
        let bucket = buckets.entry(h.finish()).or_default();
        if !bucket.contains(&log) {
            bucket.push(log);
            distinct += 1;
        }
    }
    distinct
}

#[cfg(test)]
mod tests;
