//! Spec-driven grid sweeps: parallel compute, serial CSV + ledger writes in expansion order.

use crate::spec::{InitSpec, ScenarioSpec, ScheduleSpec};
use crate::{Outcome, Scenario, runlog};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Once;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GridSpec {
    pub exp: String,
    pub base: ScenarioSpec,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pairs: Option<Vec<(usize, usize)>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub betas: Option<Vec<f64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub useful_fractions: Option<Vec<f64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub horizons: Option<Vec<usize>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub splits: Option<Vec<f64>>,
    pub ladder: Vec<LadderRung>,
}

/// Contiguous two-component partition (first s·n ids in component 0); pair only with id-independent inits.
pub fn split_partition(s: f64, n: usize) -> Vec<u32> {
    let first = (s * n as f64).round() as usize;
    (0..n).map(|i| u32::from(i >= first)).collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LadderRung {
    pub n: usize,
    pub seed_base: u64,
    pub seed_count: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RowKey {
    pub k: usize,
    pub ell: usize,
    pub n: usize,
    pub beta: Option<f64>,
    pub useful_fraction: Option<f64>,
    pub horizon: Option<usize>,
    pub split: Option<f64>,
    pub seed: u64,
}

/// Fixed nesting: pairs -> ladder -> betas -> useful_fractions -> horizons -> splits -> seeds.
pub fn expand(spec: &GridSpec) -> Result<Vec<(RowKey, Scenario)>, String> {
    if spec.ladder.is_empty() {
        return Err("ladder must have at least one rung".into());
    }
    if spec.splits.is_some() && spec.base.partition.is_some() {
        return Err("splits axis over an explicit base partition is ambiguous — drop one".into());
    }
    if spec.betas.is_some() {
        match spec.base.schedule {
            Some(ScheduleSpec::FreshPerRound { .. })
            | Some(ScheduleSpec::Permanent { .. })
            | Some(ScheduleSpec::Adaptive1Late { .. })
            | Some(ScheduleSpec::AdaptiveAlphaLate { .. }) => {}
            _ => {
                return Err(
                    "betas axis requires a fresh_per_round or permanent base schedule (adaptive_1late is Alg-1 only)".into(),
                );
            }
        }
    }
    if spec.useful_fractions.is_some() && !matches!(spec.base.init, InitSpec::WithUndecided { .. })
    {
        return Err("useful_fractions axis requires a with_undecided base init".into());
    }

    let pairs = spec
        .pairs
        .clone()
        .unwrap_or_else(|| vec![(spec.base.k, spec.base.ell)]);
    let betas: Vec<Option<f64>> = match &spec.betas {
        Some(bs) => bs.iter().copied().map(Some).collect(),
        None => vec![None],
    };
    let fractions: Vec<Option<f64>> = match &spec.useful_fractions {
        Some(us) => us.iter().copied().map(Some).collect(),
        None => vec![None],
    };
    let horizons: Vec<Option<usize>> = match &spec.horizons {
        Some(hs) => hs.iter().copied().map(Some).collect(),
        None => vec![None],
    };
    let splits: Vec<Option<f64>> = match &spec.splits {
        Some(ss) => ss.iter().copied().map(Some).collect(),
        None => vec![None],
    };

    let mut points = Vec::new();
    for &(k, ell) in &pairs {
        for rung in &spec.ladder {
            for &beta in &betas {
                for &useful in &fractions {
                    for &horizon in &horizons {
                        for &split in &splits {
                            for i in 0..rung.seed_count {
                                let seed = rung.seed_base + i;
                                let mut point = spec.base.clone();
                                point.k = k;
                                point.ell = ell;
                                point.n = rung.n;
                                point.seed = seed;
                                if let Some(h) = horizon {
                                    point.max_rounds = h;
                                }
                                if let Some(s) = split {
                                    point.partition = Some(split_partition(s, rung.n));
                                }
                                if let Some(b) = beta {
                                    point.schedule = Some(match point.schedule {
                                        Some(ScheduleSpec::Permanent { .. }) => {
                                            ScheduleSpec::Permanent { fraction: b }
                                        }
                                        Some(ScheduleSpec::Adaptive1Late { policy, .. }) => {
                                            ScheduleSpec::Adaptive1Late {
                                                fraction: b,
                                                policy,
                                            }
                                        }
                                        Some(ScheduleSpec::AdaptiveAlphaLate {
                                            alpha,
                                            policy,
                                            ..
                                        }) => ScheduleSpec::AdaptiveAlphaLate {
                                            fraction: b,
                                            alpha,
                                            policy,
                                        },
                                        _ => ScheduleSpec::FreshPerRound { fraction: b },
                                    });
                                }
                                if let (
                                    Some(u),
                                    InitSpec::WithUndecided {
                                        useful_fraction, ..
                                    },
                                ) = (useful, &mut point.init)
                                {
                                    *useful_fraction = u;
                                }
                                let scenario = Scenario::try_from(point).map_err(|e| {
                                    format!("({k},{ell}) n={} seed={seed}: {e}", rung.n)
                                })?;
                                points.push((
                                    RowKey {
                                        k,
                                        ell,
                                        n: rung.n,
                                        beta,
                                        useful_fraction: useful,
                                        horizon,
                                        split,
                                        seed,
                                    },
                                    scenario,
                                ));
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(points)
}

fn report_progress(exp: &str, done: usize, total: usize) {
    let step = (total / 50).max(1);
    if done.is_multiple_of(step) || done == total {
        eprintln!("[{exp}] {done}/{total} runs ({}%)", done * 100 / total);
    }
}

pub const CSV_HEADER: &str = "exp,k,ell,n,beta,useful_fraction,horizon,split,seed,outcome,rounds";

pub(crate) fn fmt_opt(v: Option<f64>) -> String {
    v.map(|x| format!("{x:.3}")).unwrap_or_default()
}

pub(crate) fn alg1_row(exp: &str, key: &RowKey, outcome: &Outcome) -> String {
    let (label, rounds) = match outcome {
        Outcome::Agreement { rounds, .. } => ("agreement", *rounds),
        Outcome::AllUndecided { rounds } => ("died", *rounds),
        Outcome::NoConvergence { rounds } => ("censored", *rounds),
    };
    format!(
        "{},{},{},{},{},{},{},{},{},{label},{rounds}\n",
        exp,
        key.k,
        key.ell,
        key.n,
        fmt_opt(key.beta),
        fmt_opt(key.useful_fraction),
        key.horizon.map(|h| h.to_string()).unwrap_or_default(),
        fmt_opt(key.split),
        key.seed
    )
}

// Bounds retention: a chunk of SMR reports is rate × horizon × chunk live at once.
const STREAM_CHUNK_DEFAULT: usize = 8;

fn stream_chunk() -> usize {
    std::env::var("SIM_STREAM_CHUNK")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(STREAM_CHUNK_DEFAULT)
}

fn append_to(path: &Path, data: &str) -> Result<(), String> {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("partial csv open failed: {e}"))?;
    f.write_all(data.as_bytes())
        .map_err(|e| format!("partial csv write failed: {e}"))
}

/// Runs `points` in parallel chunks of `SIM_STREAM_CHUNK`, emitting rows and ledger records in point order.
fn stream_grid<K: Sync, S: Sync, R: Send>(
    exp: &str,
    header: &str,
    points: &[(K, S)],
    ledger: Option<&Path>,
    partial: Option<&Path>,
    run: impl Fn(&S) -> R + Sync,
    mut emit: impl FnMut(&K, &S, &R, &mut String, Option<&Path>) -> Result<(), String>,
) -> Result<String, String> {
    let done = AtomicUsize::new(0);
    let ledger_path = runlog::ledger_target(ledger);

    let mut csv = String::from(header);
    csv.push('\n');
    for chunk in points.chunks(stream_chunk()) {
        let results: Vec<R> = chunk
            .par_iter()
            .map(|(_, scenario)| {
                let result = run(scenario);
                report_progress(exp, done.fetch_add(1, Ordering::Relaxed) + 1, points.len());
                result
            })
            .collect();

        let mut part = String::new();
        for ((key, scenario), result) in chunk.iter().zip(&results) {
            emit(key, scenario, result, &mut part, ledger_path.as_deref())?;
        }
        if let Some(p) = partial {
            append_to(p, &part)?;
        }
        csv.push_str(&part);
    }
    Ok(csv)
}

fn ledger_err(e: std::io::Error) -> String {
    format!("ledger write failed: {e}")
}

pub fn run_grid_streaming(
    spec: &GridSpec,
    ledger: Option<&Path>,
    partial: Option<&Path>,
) -> Result<String, String> {
    let points = expand(spec)?;
    stream_grid(
        &spec.exp,
        CSV_HEADER,
        &points,
        ledger,
        partial,
        crate::run,
        |key, scenario, outcome, part, ledger_path| {
            part.push_str(&alg1_row(&spec.exp, key, outcome));
            if let Some(path) = ledger_path {
                runlog::log_run_to(
                    path,
                    scenario,
                    outcome,
                    Some(serde_json::json!({"exp": spec.exp})),
                )
                .map_err(ledger_err)?;
            }
            Ok(())
        },
    )
}

use crate::smr::{CommandReport, CommandStatus, Proto, SmrReport, SmrScenario, SmrTerminal};
use crate::spec::{ProtoSpec, SmrScenarioSpec, TrafficSpec};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SmrGridSpec {
    pub exp: String,
    pub base: SmrScenarioSpec,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pairs: Option<Vec<(usize, usize)>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub betas: Option<Vec<f64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sigmas: Option<Vec<f64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rates: Option<Vec<usize>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub t_commits: Option<Vec<u64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub horizons: Option<Vec<usize>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub splits: Option<Vec<f64>>,
    pub ladder: Vec<LadderRung>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SmrRowKey {
    pub k: usize,
    pub ell: usize,
    pub n: usize,
    pub beta: Option<f64>,
    pub sigma: Option<f64>,
    pub rate: Option<usize>,
    pub t_commit: Option<u64>,
    pub horizon: Option<usize>,
    pub split: Option<f64>,
    pub seed: u64,
}

/// Fixed nesting: pairs -> ladder -> betas -> sigmas -> rates -> t_commits -> horizons -> splits -> seeds.
pub fn expand_smr(spec: &SmrGridSpec) -> Result<Vec<(SmrRowKey, SmrScenario)>, String> {
    if spec.ladder.is_empty() {
        return Err("ladder must have at least one rung".into());
    }
    if spec.splits.is_some() && spec.base.partition.is_some() {
        return Err("splits axis over an explicit base partition is ambiguous — drop one".into());
    }
    if spec.betas.is_some() {
        match spec.base.schedule {
            Some(ScheduleSpec::FreshPerRound { .. }) | Some(ScheduleSpec::Permanent { .. }) => {}
            _ => {
                return Err(
                    "betas axis requires a fresh_per_round or permanent base schedule".into(),
                );
            }
        }
    }
    if spec.rates.is_some() && spec.base.traffic.is_some() {
        return Err("rates axis over an explicit base traffic is ambiguous — drop one".into());
    }
    if spec.t_commits.is_some() && !matches!(spec.base.proto, ProtoSpec::Compact { .. }) {
        return Err("t_commits axis requires a compact base proto".into());
    }

    let pairs = spec
        .pairs
        .clone()
        .unwrap_or_else(|| vec![(spec.base.k, spec.base.ell)]);
    let opt_axis = |v: &Option<Vec<f64>>| -> Vec<Option<f64>> {
        match v {
            Some(xs) => xs.iter().copied().map(Some).collect(),
            None => vec![None],
        }
    };
    let betas = opt_axis(&spec.betas);
    let sigmas = opt_axis(&spec.sigmas);
    let rates: Vec<Option<usize>> = match &spec.rates {
        Some(rs) => rs.iter().copied().map(Some).collect(),
        None => vec![None],
    };
    let t_commits: Vec<Option<u64>> = match &spec.t_commits {
        Some(ts) => ts.iter().copied().map(Some).collect(),
        None => vec![None],
    };
    let horizons: Vec<Option<usize>> = match &spec.horizons {
        Some(hs) => hs.iter().copied().map(Some).collect(),
        None => vec![None],
    };
    let splits = opt_axis(&spec.splits);

    let mut points = Vec::new();
    for &(k, ell) in &pairs {
        for rung in &spec.ladder {
            for &beta in &betas {
                for &sigma in &sigmas {
                    for &rate in &rates {
                        for &t_commit in &t_commits {
                            for &horizon in &horizons {
                                for &split in &splits {
                                    for i in 0..rung.seed_count {
                                        let seed = rung.seed_base + i;
                                        let mut point = spec.base.clone();
                                        point.k = k;
                                        point.ell = ell;
                                        point.n = rung.n;
                                        point.seed = seed;
                                        if let Some(h) = horizon {
                                            point.max_rounds = h;
                                        }
                                        if let Some(s) = split {
                                            point.partition = Some(split_partition(s, rung.n));
                                        }
                                        if let Some(b) = beta {
                                            point.schedule = Some(match point.schedule {
                                                Some(ScheduleSpec::Permanent { .. }) => {
                                                    ScheduleSpec::Permanent { fraction: b }
                                                }
                                                _ => ScheduleSpec::FreshPerRound { fraction: b },
                                            });
                                        }
                                        if let Some(s) = sigma {
                                            point.sigma = s;
                                        }
                                        if let Some(r) = rate {
                                            point.traffic = Some(TrafficSpec::Pmf {
                                                arrivals_pmf: crate::smr::point_mass_pmf(r),
                                            });
                                        }
                                        if let (Some(t), ProtoSpec::Compact { t_commit_rounds }) =
                                            (t_commit, &mut point.proto)
                                        {
                                            *t_commit_rounds = t;
                                        }
                                        let scenario =
                                            SmrScenario::try_from(point).map_err(|e| {
                                                format!("({k},{ell}) n={} seed={seed}: {e}", rung.n)
                                            })?;
                                        points.push((
                                            SmrRowKey {
                                                k,
                                                ell,
                                                n: rung.n,
                                                beta,
                                                sigma,
                                                rate,
                                                t_commit,
                                                horizon,
                                                split,
                                                seed,
                                            },
                                            scenario,
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(points)
}

pub const SMR_CSV_HEADER: &str = "exp,k,ell,n,beta,sigma,rate,t_commit,horizon,split,seed,terminal,rounds,commands,complete,pending,dead,mean_te,safety,recovered_round,fork_ok,rollbacks_total,te_n,te_unsettled,te_p50,te_p90,te_p99,te_max,abort_kind";

fn nearest_rank(sorted: &[usize], p: f64) -> usize {
    let idx = (p * sorted.len() as f64).ceil().max(1.0) as usize - 1;
    sorted[idx]
}

// Each proto records only its own landmark, so the earliest present one is its settlement round.
fn settled_round(c: &CommandReport) -> Option<usize> {
    [
        c.prefix_fixed_round,
        c.committed_ack_round,
        c.executed_round,
    ]
    .into_iter()
    .flatten()
    .min()
}

// Horizon window: without it a short-horizon arm censors its own tail and flatters p99.
fn settlement_ages(report: &SmrReport, proto: Proto, rounds: usize) -> (Vec<usize>, usize) {
    let cutoff = match proto {
        Proto::Compact { t_commit_rounds } => rounds.saturating_sub(5 * t_commit_rounds as usize),
        _ => rounds,
    };
    let in_window: Vec<_> = report
        .commands
        .iter()
        .filter(|c| c.injection_round <= cutoff)
        .collect();
    let mut ages: Vec<usize> = in_window
        .iter()
        .filter(|c| c.status == CommandStatus::Complete)
        .filter_map(|c| settled_round(c).map(|r| r - c.injection_round))
        .collect();
    ages.sort_unstable();
    let unsettled = in_window.len() - ages.len();
    (ages, unsettled)
}

// Sub-floor and loaded recovery grids reach this assert by design; recorded as an outcome.
const STRIP_ABORT_MARKER: &str = "P must be a prefix of L";
const MONO_ABORT_MARKER: &str = "executed sequence regressed";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbortKind {
    Prefix,
    Mono,
}

impl AbortKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Prefix => "prefix",
            Self::Mono => "mono",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct StripAbort {
    pub round: Option<usize>,
    pub kind: AbortKind,
}

fn is_captured_abort(msg: &str) -> bool {
    msg.contains(STRIP_ABORT_MARKER) || msg.contains(MONO_ABORT_MARKER)
}

fn abort_kind(msg: &str) -> AbortKind {
    if msg.contains(MONO_ABORT_MARKER) {
        AbortKind::Mono
    } else {
        AbortKind::Prefix
    }
}

fn parse_abort_round(msg: &str) -> Option<usize> {
    for sep in ["at round ", "in round "] {
        if let Some((_, rest)) = msg.rsplit_once(sep) {
            let n: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(r) = n.parse() {
                return Some(r);
            }
        }
    }
    None
}

/// Run `f`, turning a boundary-strip or growth-regression abort into `Err`; other panics resume.
pub fn capture_strip_abort<R>(f: impl FnOnce() -> R) -> Result<R, StripAbort> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(value) => Ok(value),
        Err(payload) => {
            let abort = payload
                .downcast_ref::<&'static str>()
                .copied()
                .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                .filter(|m| is_captured_abort(m))
                .map(|m| StripAbort {
                    round: parse_abort_round(m),
                    kind: abort_kind(m),
                });
            match abort {
                Some(a) => Err(a),
                None => std::panic::resume_unwind(payload),
            }
        }
    }
}

// Wraps the prior hook and is never swapped back: per-grid swapping races across concurrent grids.
fn mute_strip_abort_hook() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if !info.payload_as_str().is_some_and(is_captured_abort) {
                previous(info);
            }
        }));
    });
}

fn smr_key_cols(exp: &str, key: &SmrRowKey) -> String {
    format!(
        "{exp},{},{},{},{},{},{},{},{},{},{}",
        key.k,
        key.ell,
        key.n,
        fmt_opt(key.beta),
        fmt_opt(key.sigma),
        key.rate.map(|r| r.to_string()).unwrap_or_default(),
        key.t_commit.map(|t| t.to_string()).unwrap_or_default(),
        key.horizon.map(|h| h.to_string()).unwrap_or_default(),
        fmt_opt(key.split),
        key.seed,
    )
}

const SMR_COLS_BEFORE_REPORT: usize = 13;

fn smr_abort_row(exp: &str, key: &SmrRowKey, abort: &StripAbort) -> String {
    let report_cols = SMR_CSV_HEADER.split(',').count() - SMR_COLS_BEFORE_REPORT - 1;
    format!(
        "{},abort,{}{},{}",
        smr_key_cols(exp, key),
        abort.round.map(|r| r.to_string()).unwrap_or_default(),
        ",".repeat(report_cols),
        abort.kind.as_str(),
    )
}

fn smr_row(exp: &str, key: &SmrRowKey, proto: Proto, report: &SmrReport) -> String {
    let (terminal, rounds) = match report.terminal {
        SmrTerminal::Ran { rounds } => ("ran", rounds),
        SmrTerminal::Dead { round } => ("dead", round),
        SmrTerminal::Failed { .. } => panic!("failed reports must use the abort row"),
    };
    let count = |s: CommandStatus| report.commands.iter().filter(|c| c.status == s).count();
    let (complete, pending, dead) = (
        count(CommandStatus::Complete),
        count(CommandStatus::Pending),
        count(CommandStatus::Dead),
    );
    let tes: Vec<usize> = report
        .commands
        .iter()
        .filter(|c| c.status == CommandStatus::Complete)
        .filter_map(|c| c.prefix_fixed_round.map(|r| r - c.injection_round))
        .collect();
    let mean_te = if tes.is_empty() {
        String::new()
    } else {
        format!("{:.2}", tes.iter().sum::<usize>() as f64 / tes.len() as f64)
    };
    let recovered = match proto {
        Proto::Recovery {
            t_window_rounds, ..
        } => report
            .recovered_round(key.n, t_window_rounds)
            .map(|r| r.to_string())
            .unwrap_or_default(),
        _ => String::new(),
    };
    let (fork_ok, rollbacks_total) = report
        .recovery
        .as_ref()
        .map(|rec| {
            (
                rec.fork_ok.to_string(),
                rec.rounds
                    .iter()
                    .map(|m| u64::from(m.rollbacks))
                    .sum::<u64>()
                    .to_string(),
            )
        })
        .unwrap_or_default();
    let (ages, te_unsettled) = settlement_ages(report, proto, rounds);
    let (te_p50, te_p90, te_p99, te_max) = if ages.is_empty() {
        (String::new(), String::new(), String::new(), String::new())
    } else {
        (
            nearest_rank(&ages, 0.50).to_string(),
            nearest_rank(&ages, 0.90).to_string(),
            nearest_rank(&ages, 0.99).to_string(),
            ages[ages.len() - 1].to_string(),
        )
    };
    format!(
        "{},{terminal},{rounds},{},{complete},{pending},{dead},{mean_te},{},{recovered},{fork_ok},{rollbacks_total},{},{te_unsettled},{te_p50},{te_p90},{te_p99},{te_max},",
        smr_key_cols(exp, key),
        report.commands.len(),
        report.safety_ok,
        ages.len(),
    )
}

pub fn run_smr_grid_streaming(
    spec: &SmrGridSpec,
    ledger: Option<&Path>,
    partial: Option<&Path>,
) -> Result<String, String> {
    let points = expand_smr(spec)?;
    mute_strip_abort_hook();
    stream_grid(
        &spec.exp,
        SMR_CSV_HEADER,
        &points,
        ledger,
        partial,
        |scenario| capture_strip_abort(|| crate::smr::run_smr_grid(scenario)),
        |key, scenario, report, part, ledger_path| {
            // Read off the run, not restated, so a row never claims an instrument it did not use.
            let extra = serde_json::json!({
                "exp": spec.exp,
                "truncate_history": crate::smr::LEAN_FORGETS_HISTORY,
                "observes_census": crate::smr::grid_observes_census(),
            });
            let with_kind = |kind: AbortKind| {
                let mut extra = extra.clone();
                extra["abort_kind"] = serde_json::json!(kind.as_str());
                Some(extra)
            };
            match report {
                Ok(report) => {
                    if let Some(failure) = &report.failure {
                        let abort = StripAbort {
                            round: Some(failure.attempted_round),
                            kind: AbortKind::Prefix,
                        };
                        part.push_str(&smr_abort_row(&spec.exp, key, &abort));
                        if let Some(path) = ledger_path {
                            runlog::log_smr_run_to(path, scenario, report, with_kind(abort.kind))
                                .map_err(ledger_err)?;
                        }
                    } else {
                        part.push_str(&smr_row(&spec.exp, key, scenario.proto, report));
                        if let Some(path) = ledger_path {
                            runlog::log_smr_run_to(path, scenario, report, Some(extra.clone()))
                                .map_err(ledger_err)?;
                        }
                    }
                }
                Err(abort) => {
                    part.push_str(&smr_abort_row(&spec.exp, key, abort));
                    if let Some(path) = ledger_path {
                        runlog::log_smr_abort_to(
                            path,
                            scenario,
                            abort.round,
                            with_kind(abort.kind),
                        )
                        .map_err(ledger_err)?;
                    }
                }
            }
            part.push('\n');
            Ok(())
        },
    )
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum AnySpec {
    Grid(GridSpec),
    Smr(SmrGridSpec),
}

pub fn grid_summary(csv: &str) -> Vec<String> {
    let Some((header, body)) = csv.split_once('\n') else {
        return Vec::new();
    };
    let cols: Vec<&str> = header.split(',').collect();
    let Some(seed_at) = cols.iter().position(|&c| c == "seed") else {
        return Vec::new();
    };
    let (verdict, good, label) = match cols.iter().position(|&c| c == "terminal") {
        Some(at) => (at, "ran", "ran"),
        None => (
            cols.iter()
                .position(|&c| c == "outcome")
                .expect("a grid CSV has an outcome or a terminal column"),
            "agreement",
            "survived",
        ),
    };

    let mut order: Vec<String> = Vec::new();
    let mut cells: std::collections::HashMap<String, (u64, u64)> = Default::default();
    for row in body.lines().filter(|l| !l.is_empty()) {
        let f: Vec<&str> = row.split(',').collect();
        let cell = format!(
            "({},{}) n={} {}",
            f[1],
            f[2],
            f[3],
            cols[4..seed_at]
                .iter()
                .zip(&f[4..seed_at])
                .map(|(name, value)| format!("{name}={value}"))
                .collect::<Vec<_>>()
                .join(" "),
        );
        let entry = cells.entry(cell.clone()).or_insert_with(|| {
            order.push(cell);
            (0, 0)
        });
        entry.1 += 1;
        if f[verdict] == good {
            entry.0 += 1;
        }
    }
    order
        .into_iter()
        .map(|cell| {
            let (won, total) = cells[&cell];
            format!("{cell}: {won}/{total} {label}")
        })
        .collect()
}

pub fn parse_specs(json: &str) -> Result<Vec<AnySpec>, String> {
    if json.trim_start().starts_with('[') {
        serde_json::from_str::<Vec<AnySpec>>(json).map_err(|e| format!("spec array: {e}"))
    } else {
        serde_json::from_str::<AnySpec>(json)
            .map(|s| vec![s])
            .map_err(|e| format!("spec: {e}"))
    }
}

/// Reads and parses a spec file; an empty array is an error.
pub fn load_specs(path: &str) -> Result<Vec<AnySpec>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("read spec {path}: {e}"))?;
    let specs = parse_specs(&text)?;
    if specs.is_empty() {
        return Err(format!("{path}: spec file holds no grids"));
    }
    Ok(specs)
}

/// Concatenates per-grid CSVs, keeping only the first part's header.
pub fn concat_csv(parts: &[String]) -> String {
    let mut csv = String::new();
    for (i, part) in parts.iter().enumerate() {
        if i == 0 {
            csv.push_str(part);
        } else {
            csv.push_str(part.split_once('\n').map(|(_, rest)| rest).unwrap_or(""));
        }
    }
    csv
}

#[cfg(test)]
mod tests;
