//! DEV-3 skip episodes: a node's stale checkpoint from its boundary skip to its adoption.

pub(in crate::smr) struct BoundarySkipTally {
    stale_since: Vec<Option<usize>>,
    events: u64,
    first_round: Option<usize>,
    last_round: Option<usize>,
    max_rejoin_rounds: Option<usize>,
}

impl BoundarySkipTally {
    pub(in crate::smr) fn new(n: usize) -> Self {
        BoundarySkipTally {
            stale_since: vec![None; n],
            events: 0,
            first_round: None,
            last_round: None,
            max_rejoin_rounds: None,
        }
    }

    pub(in crate::smr) fn observe(&mut self, stale: impl IntoIterator<Item = bool>, round: usize) {
        for (since, stale) in self.stale_since.iter_mut().zip(stale) {
            match (*since, stale) {
                (None, true) => {
                    *since = Some(round);
                    self.events += 1;
                    self.first_round.get_or_insert(round);
                    self.last_round = Some(round);
                }
                (Some(start), false) => {
                    *since = None;
                    let rounds = round - start;
                    self.max_rejoin_rounds =
                        Some(self.max_rejoin_rounds.map_or(rounds, |m| m.max(rounds)));
                }
                _ => {}
            }
        }
    }

    pub(in crate::smr) fn events(&self) -> u64 {
        self.events
    }

    pub(in crate::smr) fn first_round(&self) -> Option<usize> {
        self.first_round
    }

    pub(in crate::smr) fn last_round(&self) -> Option<usize> {
        self.last_round
    }

    /// Over completed episodes only; `unrejoined` counts the open ones.
    pub(in crate::smr) fn max_rejoin_rounds(&self) -> Option<usize> {
        self.max_rejoin_rounds
    }

    pub(in crate::smr) fn unrejoined(&self) -> usize {
        self.stale_since.iter().filter(|s| s.is_some()).count()
    }
}

#[cfg(test)]
mod tests;
