//! Blocking-schedule compilation and per-round mask resolution; all mask entropy comes from the caller's RNG.

use crate::{BlockSchedule, BlockTarget};
use rand::seq::index::sample;
use rand_chacha::ChaCha12Rng;

pub(crate) enum Blocking {
    Fresh {
        count: usize,
    },
    // Drawn by the Alg-1 run loop (needs node state); `round_mask` must never see this variant.
    Adaptive {
        count: usize,
    },
    AdaptiveAlpha {
        count: usize,
        alpha: u32,
    },
    Fixed(Vec<bool>),
    Windows(Vec<(usize, usize, Vec<bool>)>),
    PerRound {
        counts: Vec<usize>,
    },
    Sticky {
        background_count: usize,
        counts: Vec<usize>,
    },
}

// RNG contract: masks are sampled up front in declaration order.
pub(crate) fn compile_blocking(
    schedule: &BlockSchedule,
    n: usize,
    rng: &mut ChaCha12Rng,
) -> Blocking {
    match schedule {
        BlockSchedule::FreshPerRound { fraction } => Blocking::Fresh {
            count: fraction_count(*fraction, n),
        },
        BlockSchedule::Adaptive1Late { fraction, .. } => Blocking::Adaptive {
            count: fraction_count(*fraction, n),
        },
        BlockSchedule::AdaptiveAlphaLate {
            fraction, alpha, ..
        } => Blocking::AdaptiveAlpha {
            count: fraction_count(*fraction, n),
            alpha: *alpha,
        },
        BlockSchedule::Permanent { fraction } => {
            let count = fraction_count(*fraction, n);
            Blocking::Fixed(sample_blocked(rng, n, count))
        }
        BlockSchedule::Windows(windows) => Blocking::Windows(
            windows
                .iter()
                .map(|w| {
                    let mask = match &w.target {
                        BlockTarget::Nodes(ids) => {
                            let mut mask = vec![false; n];
                            for &id in ids {
                                mask[id as usize] = true;
                            }
                            mask
                        }
                        BlockTarget::SampleFraction(fraction) => {
                            sample_blocked(rng, n, fraction_count(*fraction, n))
                        }
                    };
                    (w.start_round, w.start_round + w.rounds - 1, mask)
                })
                .collect(),
        ),
        BlockSchedule::PerRoundFractions(fractions) => Blocking::PerRound {
            counts: fractions.iter().map(|f| fraction_count(*f, n)).collect(),
        },
        BlockSchedule::PerRoundSticky {
            background,
            targets,
        } => Blocking::Sticky {
            background_count: fraction_count(*background, n),
            counts: targets.iter().map(|f| fraction_count(*f, n)).collect(),
        },
    }
}

// RNG contract: Sticky draws background noise only when nonzero; Fixed/Windows draw nothing.
pub(crate) fn round_mask(
    blocking: &Blocking,
    round: usize,
    sticky_mask: &mut [bool],
    n: usize,
    rng: &mut ChaCha12Rng,
) -> Vec<bool> {
    match blocking {
        Blocking::Fresh { count } => sample_blocked(rng, n, *count),
        Blocking::Adaptive { .. } | Blocking::AdaptiveAlpha { .. } => {
            unreachable!("adaptive masks are drawn by the Alg-1 run loop, not round_mask")
        }
        Blocking::PerRound { counts } => {
            sample_blocked(rng, n, counts.get(round - 1).copied().unwrap_or(0))
        }
        Blocking::Sticky {
            background_count,
            counts,
        } => {
            let count = counts.get(round - 1).copied().unwrap_or(0);
            sticky_with_background(rng, sticky_mask, count, *background_count, n)
        }
        Blocking::Fixed(mask) => mask.clone(),
        Blocking::Windows(windows) => {
            let mut union = vec![false; n];
            for (start, end, mask) in windows {
                if (*start..=*end).contains(&round) {
                    for (u, &m) in union.iter_mut().zip(mask) {
                        *u |= m;
                    }
                }
            }
            union
        }
    }
}

/// Full per-round blocked-mask matrix (index 0 = round 1), in the driver loops' draw order.
pub fn mask_schedule(
    schedule: &BlockSchedule,
    n: usize,
    rounds: usize,
    rng: &mut ChaCha12Rng,
) -> Vec<Vec<bool>> {
    let blocking = compile_blocking(schedule, n, rng);
    let mut sticky = vec![false; n];
    (1..=rounds)
        .map(|round| round_mask(&blocking, round, &mut sticky, n, rng))
        .collect()
}

impl Blocking {
    pub(crate) fn done_blocking_after(&self, round: usize) -> bool {
        match self {
            Blocking::Fresh { count } => *count == 0,
            Blocking::Adaptive { count } => *count == 0,
            Blocking::AdaptiveAlpha { count, .. } => *count == 0,
            Blocking::Fixed(mask) => !mask.contains(&true),
            Blocking::Windows(windows) => windows
                .iter()
                .all(|(_, end, mask)| *end <= round || !mask.contains(&true)),
            Blocking::PerRound { counts } => counts.iter().skip(round).all(|&c| c == 0),
            Blocking::Sticky {
                background_count,
                counts,
            } => *background_count == 0 && counts.iter().skip(round).all(|&c| c == 0),
        }
    }
}

// RNG contract: the sticky adjustment draws first; background noise draws only when nonzero.
pub(crate) fn sticky_with_background(
    rng: &mut ChaCha12Rng,
    sticky: &mut [bool],
    count: usize,
    background_count: usize,
    n: usize,
) -> Vec<bool> {
    adjust_sticky_mask(rng, sticky, count, n);
    if background_count > 0 {
        let mut union = sample_blocked(rng, n, background_count);
        for (u, &s) in union.iter_mut().zip(sticky.iter()) {
            *u |= s;
        }
        union
    } else {
        sticky.to_vec()
    }
}

// RNG contract: steady and full-set transitions draw nothing.
pub(crate) fn adjust_sticky_mask(rng: &mut ChaCha12Rng, mask: &mut [bool], count: usize, n: usize) {
    let count = count.min(n);
    let blocked: Vec<usize> = (0..n).filter(|&i| mask[i]).collect();
    match count.cmp(&blocked.len()) {
        std::cmp::Ordering::Greater => {
            let free: Vec<usize> = (0..n).filter(|&i| !mask[i]).collect();
            let delta = count - blocked.len();
            if delta >= free.len() {
                for i in free {
                    mask[i] = true;
                }
            } else {
                for pick in sample(rng, free.len(), delta) {
                    mask[free[pick]] = true;
                }
            }
        }
        std::cmp::Ordering::Less => {
            let delta = blocked.len() - count;
            if delta >= blocked.len() {
                mask.fill(false);
            } else {
                for pick in sample(rng, blocked.len(), delta) {
                    mask[blocked[pick]] = false;
                }
            }
        }
        std::cmp::Ordering::Equal => {}
    }
}

pub(crate) fn fraction_count(fraction: f64, n: usize) -> usize {
    if n == 0 {
        return 0;
    }

    let mut count = ((fraction * n as f64).floor() as usize).min(n);
    // Multiplication and division can round to opposite sides of a boundary.
    while count > 0 && count as f64 / n as f64 > fraction {
        count -= 1;
    }
    while count < n && (count + 1) as f64 / n as f64 <= fraction {
        count += 1;
    }
    count
}

// Two independent draws (holders, then the rest); the second is skipped when holders cover the budget.
pub(crate) fn adaptive_mask(
    rng: &mut ChaCha12Rng,
    holders: &[usize],
    n: usize,
    count: usize,
) -> Vec<bool> {
    let mut blocked = vec![false; n];
    if count >= holders.len() {
        for &id in holders {
            blocked[id] = true;
        }
        let rest: Vec<usize> = (0..n).filter(|i| !blocked[*i]).collect();
        let spill = (count - holders.len()).min(rest.len());
        for i in sample(rng, rest.len(), spill) {
            blocked[rest[i]] = true;
        }
    } else {
        for i in sample(rng, holders.len(), count) {
            blocked[holders[i]] = true;
        }
    }
    blocked
}

pub(crate) fn sample_blocked(rng: &mut ChaCha12Rng, n: usize, count: usize) -> Vec<bool> {
    let mut blocked = vec![false; n];
    for i in sample(rng, n, count.min(n)) {
        blocked[i] = true;
    }
    blocked
}

#[cfg(test)]
mod tests;
