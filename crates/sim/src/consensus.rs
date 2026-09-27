//! The single-value simulator: Algorithm 1 rounds over n nodes under blocking schedules and partitions.

use protocol::MedianNode;

pub(crate) mod blocking;
pub use blocking::mask_schedule;
use blocking::{
    Blocking, adaptive_mask, adjust_sticky_mask, compile_blocking, fraction_count, round_mask,
    sample_blocked, sticky_with_background,
};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha12Rng;

pub type Value = protocol::Value;
pub use protocol::Config;

#[derive(Debug, Clone, serde::Serialize)]
pub struct Scenario {
    pub n: usize,
    pub seed: u64,
    pub cfg: Config,
    pub init: Init,
    pub max_rounds: usize,
    pub schedule: BlockSchedule,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub partition: Option<Vec<u32>>,
}

// Target sampling stays unrestricted; cross-component targets just never answer (RNG stream unchanged).
pub(crate) fn same_component(partition: Option<&[u32]>, i: usize, t: usize) -> bool {
    partition.is_none_or(|c| c[i] == c[t])
}

pub(crate) fn check_partition(partition: Option<&[u32]>, n: usize) -> Result<(), String> {
    match partition {
        Some(c) if c.len() != n => {
            Err(format!("partition names {} nodes, expected n={n}", c.len()))
        }
        _ => Ok(()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdaptivePolicy {
    BlockHolders,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub enum BlockSchedule {
    FreshPerRound {
        fraction: f64,
    },
    Permanent {
        fraction: f64,
    },
    Windows(Vec<BlockWindow>),
    PerRoundFractions(Vec<f64>),
    /// α = 0 adaptive blocking; the name is a frozen mislabel kept for replay (1-late is `AdaptiveAlphaLate { alpha: 1 }`).
    Adaptive1Late {
        fraction: f64,
        policy: AdaptivePolicy,
    },
    /// Definition 1.1 α-late adaptive blocking (α ≥ 1); Alg-1 only.
    AdaptiveAlphaLate {
        fraction: f64,
        alpha: u32,
        policy: AdaptivePolicy,
    },
    PerRoundSticky {
        background: f64,
        targets: Vec<f64>,
    },
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct BlockWindow {
    /// 1-based, inclusive.
    pub start_round: usize,
    pub rounds: usize,
    pub target: BlockTarget,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub enum BlockTarget {
    Nodes(Vec<u32>),
    SampleFraction(f64),
}

#[derive(Debug, Clone, serde::Serialize)]
pub enum Init {
    Split {
        fraction: f64,
    },
    UniformRandom {
        k: Value,
    },
    Distinct,
    EvenSplit {
        values: usize,
    },
    Weighted {
        weights: Vec<f64>,
        range: u64,
    },
    /// Lemma 2.1/2.2 start: the first ⌈u₀·n⌉ nodes take `inner`'s value, the rest start ⊥.
    WithUndecided {
        useful_fraction: f64,
        inner: Box<Init>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum Outcome {
    Agreement { value: Value, rounds: usize },
    AllUndecided { rounds: usize },
    NoConvergence { rounds: usize },
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Trace {
    pub n: usize,
    pub seed: u64,
    pub initial: Vec<Option<Value>>,
    pub rounds: Vec<RoundTrace>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct RoundTrace {
    pub states: Vec<Option<Value>>,
    pub targets: Vec<Vec<u32>>,
    pub blocked: Vec<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct RoundMetrics {
    pub holders: u32,
    pub undecided: u32,
    pub blocked: u32,
    pub useful: u32,
    pub distinct_values: u32,
}

#[derive(Debug, serde::Serialize)]
pub struct Report {
    pub outcome: Outcome,
    pub metrics: Vec<RoundMetrics>,
}

pub fn run(scenario: &Scenario) -> Outcome {
    run_inner(scenario, false).0
}

pub fn run_report(scenario: &Scenario) -> Report {
    let (outcome, metrics, _) = run_inner(scenario, false);
    Report { outcome, metrics }
}

pub fn run_traced(scenario: &Scenario) -> (Outcome, Trace) {
    let (outcome, _, trace) = run_inner(scenario, true);
    (outcome, trace.expect("recording was requested"))
}

pub fn run_traced_report(scenario: &Scenario) -> (Report, Trace) {
    let (outcome, metrics, trace) = run_inner(scenario, true);
    (
        Report { outcome, metrics },
        trace.expect("recording was requested"),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoundStatus {
    pub metrics: RoundMetrics,
    pub agreed: Option<Value>,
    pub all_hold: bool,
    pub all_undecided: bool,
}

/// Incremental stepping core; its RNG stream is byte-identical to the batch runner's.
pub struct SimState {
    n: usize,
    cfg: Config,
    nodes: Vec<MedianNode>,
    rng: ChaCha12Rng,
    round: usize,
    trace: Option<Trace>,
    metrics: Vec<RoundMetrics>,
    agreed_since: Option<(Value, usize)>,
    partition: Option<Vec<u32>>,
}

impl SimState {
    pub fn new(
        n: usize,
        cfg: Config,
        init: &Init,
        seed: u64,
        record_trace: bool,
        partition: Option<&[u32]>,
    ) -> SimState {
        check_partition(partition, n).expect("invalid partition");
        // Single seeded RNG is the only entropy source; replay-from-seed depends on it.
        let mut rng = ChaCha12Rng::seed_from_u64(seed);
        let nodes: Vec<MedianNode> = (0..n)
            .map(|i| match initial_state(init, i, n, &mut rng) {
                Some(v) => MedianNode::new(v, cfg),
                None => MedianNode::new_undecided(cfg),
            })
            .collect();
        let trace = record_trace.then(|| Trace {
            n,
            seed,
            initial: nodes.iter().map(MedianNode::answer).collect(),
            rounds: Vec::new(),
        });
        SimState {
            n,
            cfg,
            nodes,
            rng,
            round: 0,
            trace,
            metrics: Vec::new(),
            agreed_since: None,
            partition: partition.map(<[u32]>::to_vec),
        }
    }

    pub fn sample_mask(&mut self, count: usize) -> Vec<bool> {
        sample_blocked(&mut self.rng, self.n, count)
    }

    pub fn step_sampled(&mut self, count: usize) -> RoundStatus {
        let blocked = sample_blocked(&mut self.rng, self.n, count);
        self.step_with(&blocked)
    }

    pub fn step_fraction(&mut self, fraction: f64) -> RoundStatus {
        self.step_sampled(self.blocked_count(fraction))
    }

    pub fn blocked_count(&self, fraction: f64) -> usize {
        fraction_count(fraction, self.n)
    }

    pub fn step_masked(&mut self, blocked: &[bool]) -> RoundStatus {
        self.step_with(blocked)
    }

    pub fn adjust_sticky(&mut self, mask: &mut [bool], count: usize) {
        adjust_sticky_mask(&mut self.rng, mask, count, self.n);
    }

    /// The batch driver's sticky round mask: adjust `sticky` to `count`, then union a background sample.
    pub fn sticky_round_mask(
        &mut self,
        sticky: &mut [bool],
        count: usize,
        background_count: usize,
    ) -> Vec<bool> {
        sticky_with_background(&mut self.rng, sticky, count, background_count, self.n)
    }

    pub(crate) fn rng_mut(&mut self) -> &mut ChaCha12Rng {
        &mut self.rng
    }

    pub(crate) fn holder_ids(&self) -> Vec<usize> {
        (0..self.n)
            .filter(|&i| self.nodes[i].answer().is_some())
            .collect()
    }

    pub fn round(&self) -> usize {
        self.round
    }

    pub fn metrics(&self) -> &[RoundMetrics] {
        &self.metrics
    }

    pub fn trace(&self) -> Option<&Trace> {
        self.trace.as_ref()
    }

    pub fn agreed_since(&self) -> Option<(Value, usize)> {
        self.agreed_since
    }

    pub fn into_parts(self) -> (Vec<RoundMetrics>, Option<Trace>) {
        (self.metrics, self.trace)
    }

    fn step_with(&mut self, blocked: &[bool]) -> RoundStatus {
        let n = self.n;
        let cfg = self.cfg;
        self.round += 1;
        let snapshot: Vec<Option<Value>> = self.nodes.iter().map(MedianNode::answer).collect();

        let partition = self.partition.as_deref();
        let mut targets_log: Vec<Vec<u32>> = Vec::new();
        let mut inboxes: Vec<Vec<Value>> = Vec::with_capacity(n);
        for i in 0..n {
            let mut replies = Vec::with_capacity(cfg.k);
            let mut targets = Vec::with_capacity(cfg.k);
            for _ in 0..cfg.k {
                let t = self.rng.random_range(0..n);
                targets.push(t as u32);
                if !blocked[i]
                    && !blocked[t]
                    && same_component(partition, i, t)
                    && let Some(v) = snapshot[t]
                {
                    replies.push(v);
                }
            }
            targets_log.push(targets);
            inboxes.push(replies);
        }

        for (node, inbox) in self.nodes.iter_mut().zip(&inboxes) {
            node.step(inbox, &mut self.rng);
        }

        if let Some(t) = self.trace.as_mut() {
            t.rounds.push(RoundTrace {
                states: self.nodes.iter().map(MedianNode::answer).collect(),
                targets: targets_log,
                blocked: (0..n).filter(|&i| blocked[i]).map(|i| i as u32).collect(),
            });
        }

        let holders = self
            .nodes
            .iter()
            .filter(|node| node.answer().is_some())
            .count() as u32;
        let round_metrics = RoundMetrics {
            holders,
            undecided: n as u32 - holders,
            blocked: blocked.iter().filter(|&&b| b).count() as u32,
            useful: snapshot
                .iter()
                .zip(blocked)
                .filter(|(s, b)| s.is_some() && !**b)
                .count() as u32,
            distinct_values: self
                .nodes
                .iter()
                .filter_map(MedianNode::answer)
                .collect::<std::collections::HashSet<_>>()
                .len() as u32,
        };
        self.metrics.push(round_metrics);

        let (agreed, all_undecided) = match population_state(&self.nodes) {
            Population::Agreed(value) => {
                if self.agreed_since.is_none() {
                    self.agreed_since = Some((value, self.round));
                }
                (Some(value), false)
            }
            Population::AllUndecided => (None, true),
            Population::Mixed => (None, false),
        };
        RoundStatus {
            metrics: round_metrics,
            agreed,
            all_hold: self.nodes.iter().all(|node| node.answer().is_some()),
            all_undecided,
        }
    }
}

pub(crate) fn run_inner(
    scenario: &Scenario,
    record_trace: bool,
) -> (Outcome, Vec<RoundMetrics>, Option<Trace>) {
    let n = scenario.n;
    let mut state = SimState::new(
        n,
        scenario.cfg,
        &scenario.init,
        scenario.seed,
        record_trace,
        scenario.partition.as_deref(),
    );

    // Blocking compiles before round 1; declaration order is part of the RNG stream.
    let blocking = compile_blocking(&scenario.schedule, n, state.rng_mut());
    let mut sticky_mask = vec![false; n];
    let mut holder_history: std::collections::VecDeque<Vec<usize>> =
        std::collections::VecDeque::new();

    for round in 1..=scenario.max_rounds {
        let mask = match &blocking {
            Blocking::Adaptive { count } if round > 1 => {
                let holders = state.holder_ids();
                adaptive_mask(state.rng_mut(), &holders, n, *count)
            }
            Blocking::Adaptive { count } => sample_blocked(state.rng_mut(), n, *count),
            Blocking::AdaptiveAlpha { count, alpha } => {
                if holder_history.len() > *alpha as usize {
                    adaptive_mask(state.rng_mut(), &holder_history[0], n, *count)
                } else {
                    sample_blocked(state.rng_mut(), n, *count)
                }
            }
            _ => round_mask(&blocking, round, &mut sticky_mask, n, state.rng_mut()),
        };
        let status = state.step_masked(&mask);
        if let Blocking::AdaptiveAlpha { alpha, .. } = &blocking {
            holder_history.push_back(state.holder_ids());
            if holder_history.len() > *alpha as usize + 1 {
                holder_history.pop_front();
            }
        }

        if status.all_undecided {
            let (metrics, trace) = state.into_parts();
            return (Outcome::AllUndecided { rounds: round }, metrics, trace);
        }
        // Absorbing only if all hold and nobody can be blocked later: with ⊥ nodes, holders can still go extinct.
        if status.agreed.is_some() && status.all_hold && blocking.done_blocking_after(round) {
            let (value, rounds) = state.agreed_since().expect("agreed implies recorded");
            let (metrics, trace) = state.into_parts();
            return (Outcome::Agreement { value, rounds }, metrics, trace);
        }
    }
    // The median rule never invents values, so a single surviving value is final.
    let outcome = if let Some((value, rounds)) = state.agreed_since() {
        Outcome::Agreement { value, rounds }
    } else {
        Outcome::NoConvergence {
            rounds: scenario.max_rounds,
        }
    };
    let (metrics, trace) = state.into_parts();
    (outcome, metrics, trace)
}

/// One weighted bucket draw (exactly one f64); the SMR traffic count-draw shares this shape.
pub fn draw_weighted_index(rng: &mut ChaCha12Rng, weights: &[f64]) -> usize {
    let total: f64 = weights.iter().sum();
    let mut r = rng.random::<f64>() * total;
    let mut pick = weights.iter().rposition(|w| *w > 0.0).unwrap_or(0);
    for (v, w) in weights.iter().enumerate() {
        if r < *w {
            pick = v;
            break;
        }
        r -= w;
    }
    pick
}

/// One node's initial value; call for i in 0..n over one RNG, since the draw order is the assignment.
pub fn initial_state(init: &Init, i: usize, n: usize, rng: &mut ChaCha12Rng) -> Option<Value> {
    match init {
        Init::Split { fraction } => Some(if (i as f64) < fraction * n as f64 {
            0
        } else {
            1
        }),
        Init::UniformRandom { k } => Some(rng.random_range(0..*k)),
        Init::Distinct => Some(i as Value),
        Init::EvenSplit { values } => Some((i * values / n) as Value),
        Init::Weighted { weights, range } => {
            let pick = draw_weighted_index(rng, weights);
            let m = weights.len() as u128;
            let lo = (*range as u128 * pick as u128 / m) as u64;
            let hi = (*range as u128 * (pick as u128 + 1) / m) as u64;
            // 1-value slices draw nothing, keeping dense sampling at one draw per node.
            if hi - lo > 1 {
                Some(lo + rng.random_range(0..hi - lo))
            } else {
                Some(lo)
            }
        }
        // ⊥ nodes draw nothing, so the stream equals the inner init at the smaller useful count.
        Init::WithUndecided {
            useful_fraction,
            inner,
        } => {
            let useful = (useful_fraction * n as f64).ceil() as usize;
            (i < useful)
                .then(|| initial_state(inner, i, useful.max(1), rng))
                .flatten()
        }
    }
}

enum Population {
    Agreed(Value),
    AllUndecided,
    Mixed,
}

fn population_state(nodes: &[MedianNode]) -> Population {
    let mut agreed: Option<Value> = None;
    let mut any_value = false;
    for node in nodes {
        if let Some(v) = node.x_i() {
            any_value = true;
            match agreed {
                None => agreed = Some(v),
                Some(a) if a != v => return Population::Mixed,
                _ => {}
            }
        }
    }
    if !any_value {
        return Population::AllUndecided;
    }
    // Def 1.2: all value-holding servers agree; ⊥ nodes allowed (instantaneous check).
    Population::Agreed(agreed.expect("any_value implies Some"))
}
