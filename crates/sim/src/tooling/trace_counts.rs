//! Per-round message counts for the scenario stream `sweep::run_grid_streaming` executes (tracing draws no RNG).

use crate::sweep::{CSV_HEADER, GridSpec, alg1_row, expand, fmt_opt};
use crate::{Outcome, RoundTrace, Scenario, Value, run_traced};

pub const COUNTS_CSV_HEADER: &str = "exp,k,ell,n,beta,split,seed,round,requests,replies";
pub const OUTCOMES_CSV_HEADER: &str = CSV_HEADER;

/// One round's counts: `requests` = k per unblocked node, `replies` = those the inbox gate admits.
pub fn round_counts(
    k: usize,
    entering: &[Option<Value>],
    round: &RoundTrace,
    partition: Option<&[u32]>,
) -> (usize, usize) {
    let n = entering.len();
    let mut blocked_mask = vec![false; n];
    for &id in &round.blocked {
        blocked_mask[id as usize] = true;
    }
    let requests = k * (n - round.blocked.len());
    let mut replies = 0usize;
    for (i, targets) in round.targets.iter().enumerate() {
        if blocked_mask[i] {
            continue;
        }
        for &t in targets {
            let t = t as usize;
            if !blocked_mask[t] && crate::same_component(partition, i, t) && entering[t].is_some() {
                replies += 1;
            }
        }
    }
    (requests, replies)
}

pub fn run_trace_counts(scenario: &Scenario) -> (Outcome, Vec<(usize, usize, usize)>) {
    let (outcome, trace) = run_traced(scenario);
    let mut rows = Vec::with_capacity(trace.rounds.len());
    let mut entering = &trace.initial;
    for (i, round) in trace.rounds.iter().enumerate() {
        let (requests, replies) = round_counts(
            scenario.cfg.k,
            entering,
            round,
            scenario.partition.as_deref(),
        );
        rows.push((i + 1, requests, replies));
        entering = &round.states;
    }
    (outcome, rows)
}

pub fn run_grid_counts(spec: &GridSpec) -> Result<(String, String), String> {
    let points = expand(spec)?;

    let mut counts_csv = String::from(COUNTS_CSV_HEADER);
    counts_csv.push('\n');
    let mut outcomes_csv = String::from(OUTCOMES_CSV_HEADER);
    outcomes_csv.push('\n');

    for (key, scenario) in &points {
        let (outcome, rows) = run_trace_counts(scenario);
        for (round, requests, replies) in rows {
            counts_csv.push_str(&format!(
                "{},{},{},{},{},{},{},{round},{requests},{replies}\n",
                spec.exp,
                key.k,
                key.ell,
                key.n,
                fmt_opt(key.beta),
                fmt_opt(key.split),
                key.seed
            ));
        }
        outcomes_csv.push_str(&alg1_row(&spec.exp, key, &outcome));
    }
    Ok((counts_csv, outcomes_csv))
}
