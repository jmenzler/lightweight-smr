//! Pure aggregation of per-node round records into sim-schema CSVs + convergence summary.

use crate::record::{PostState, RoundRecord};
use crate::spec::NodeRunSpec;
use sim::runlog::{SMR_METRICS_CSV_HEADER, VALUE_METRICS_CSV_HEADER};
use std::collections::{BTreeMap, BTreeSet};

const MESSAGES_CSV_HEADER: &str =
    "round,requests,replies,appends,client_msgs,acks,bytes_out,bytes_in";

#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    /// deviation: see node/networked-node-architecture.md (F17)
    pub convergence_round: Option<u64>,
    /// Round-timing tails across all nodes × rounds.
    pub timing: RoundTiming,
}

/// Max + nearest-rank p99 of the per-round timing fields, in microseconds.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RoundTiming {
    pub t_last_reply_us_max: u64,
    pub t_last_reply_us_p99: u64,
    pub t_step_done_us_max: u64,
    pub t_step_done_us_p99: u64,
    pub t_step_compute_us_max: u64,
    pub t_step_compute_us_p99: u64,
}

fn timing_stats(records: &[Vec<RoundRecord>]) -> RoundTiming {
    fn tail(mut vals: Vec<u64>) -> (u64, u64) {
        if vals.is_empty() {
            return (0, 0);
        }
        vals.sort_unstable();
        let p99 = vals[(vals.len() * 99).div_ceil(100) - 1];
        (*vals.last().expect("non-empty"), p99)
    }
    let all =
        |f: fn(&RoundRecord) -> u64| -> Vec<u64> { records.iter().flatten().map(f).collect() };
    let (reply_max, reply_p99) = tail(all(|r| r.t_last_reply_us));
    let (step_max, step_p99) = tail(all(|r| r.t_step_done_us));
    let (compute_max, compute_p99) = tail(all(|r| r.t_step_compute_us));
    RoundTiming {
        t_last_reply_us_max: reply_max,
        t_last_reply_us_p99: reply_p99,
        t_step_done_us_max: step_max,
        t_step_done_us_p99: step_p99,
        t_step_compute_us_max: compute_max,
        t_step_compute_us_p99: compute_p99,
    }
}

/// Validity gates; a run is only sim-comparable when `clean()`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Gates {
    pub late_rep: u64,
    pub late_app: u64,
    pub dropped_past: u64,
    pub encode_err: u64,
    pub rounds_consistent: bool,
    /// Strong Safety (Def 1.4): latches false on the first executed-prefix divergence.
    pub safety_ok: bool,
}

impl Gates {
    pub fn clean(&self) -> bool {
        self.late_rep == 0
            && self.late_app == 0
            && self.dropped_past == 0
            && self.encode_err == 0
            && self.rounds_consistent
            && self.safety_ok
    }

    fn accumulate(&mut self, rec: &RoundRecord) {
        self.late_rep += u64::from(rec.late_rep);
        self.late_app += u64::from(rec.late_app);
        self.dropped_past += u64::from(rec.dropped_past);
        self.encode_err += u64::from(rec.encode_err);
    }
}

#[derive(Default)]
struct MsgTotals {
    req: u64,
    rep: u64,
    app: u64,
    cli: u64,
    acks: u64,
    bytes_out: u64,
    bytes_in: u64,
}

impl MsgTotals {
    fn add(&mut self, rec: &RoundRecord) {
        self.req += u64::from(rec.req_sent);
        self.rep += u64::from(rec.rep_recv);
        self.app += u64::from(rec.app_recv);
        self.cli += u64::from(rec.cli_recv);
        self.acks += u64::from(rec.acks);
        self.bytes_out += rec.bytes_out;
        self.bytes_in += rec.bytes_in;
    }

    fn csv_row(&self, round: usize) -> String {
        format!(
            "{round},{},{},{},{},{},{},{}\n",
            self.req, self.rep, self.app, self.cli, self.acks, self.bytes_out, self.bytes_in
        )
    }
}

/// Per tracked op, the sim's SMR landmarks derived from the per-round digests.
#[derive(Debug, Clone, PartialEq)]
pub struct Landmark {
    pub op: u64,
    /// T_B — first round the op is in EVERY non-⊥ log.
    pub all_logs_round: Option<u64>,
    /// T_E — first round all non-⊥ nodes agree on its position and prefix, persisting to the horizon.
    pub prefix_fixed_round: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct Collected {
    /// Byte-compatible with the sim runlog CSV schema, except the extended arm's empty `lcp_len`.
    pub metrics_csv: String,
    pub messages_csv: String,
    pub summary: Summary,
    pub gates: Gates,
    /// SMR only; empty for the value modes.
    pub landmarks: Vec<Landmark>,
}

/// SMR aggregation into the sim's 10-column schema.
pub fn aggregate_smr(
    records: &[Vec<RoundRecord>],
    spec: &NodeRunSpec,
    arrivals: Option<&[u32]>,
) -> Result<Collected, String> {
    // extended has no derivable LCP: an empty cell, never a 0 that reads as computed
    let compact = matches!(
        &spec.scenario,
        crate::spec::ScenarioKind::Smr(s)
            if matches!(s.proto, sim::spec::ProtoSpec::Compact { .. })
    );
    if records.is_empty() || records.iter().all(|r| r.is_empty()) {
        return Err("no records to aggregate".to_string());
    }
    let rounds = records.iter().map(|r| r.len()).max().unwrap_or(0);
    let rounds_consistent = records.iter().all(|r| r.len() == rounds);

    let mut metrics_csv = format!("{SMR_METRICS_CSV_HEADER}\n");
    let mut messages_csv = format!("{MESSAGES_CSV_HEADER}\n");
    let mut gates = Gates {
        rounds_consistent,
        safety_ok: true,
        ..Gates::default()
    };
    let mut prefix_witness: BTreeMap<usize, u64> = BTreeMap::new();
    let mut tracked_ops: Vec<u64> = Vec::new();
    let mut tb: Vec<Option<u64>> = Vec::new();
    let mut te: Vec<Option<u64>> = Vec::new();

    for i in 0..rounds {
        let round_recs: Vec<&RoundRecord> = records.iter().filter_map(|r| r.get(i)).collect();
        let mut nonbot = 0u32;
        let mut blocked = 0u32;
        let mut useful = 0u32;
        let mut hashes = BTreeSet::new();
        let mut max_log_len = 0usize;
        let mut exec_lens = Vec::new();
        let mut totals = MsgTotals::default();
        for rec in &round_recs {
            match &rec.post {
                PostState::Log {
                    log_len,
                    exec_len,
                    log_hash,
                } => {
                    if let Some(len) = log_len {
                        nonbot += 1;
                        max_log_len = max_log_len.max(*len);
                    }
                    if let Some(h) = log_hash {
                        hashes.insert(*h);
                    }
                    exec_lens.push(*exec_len);
                }
                PostState::Value(_) => {
                    return Err("value-mode records need aggregate(), not aggregate_smr()".into());
                }
            }
            blocked += u32::from(rec.blocked);
            useful += u32::from(rec.snap_held && !rec.blocked);
            totals.add(rec);
            gates.accumulate(rec);
            for &(len, hash) in &rec.exec_prefix {
                match prefix_witness.get(&len) {
                    Some(&seen) if seen != hash => gates.safety_ok = false,
                    Some(_) => {}
                    None => {
                        prefix_witness.insert(len, hash);
                    }
                }
            }
            for d in &rec.tracked {
                if !tracked_ops.contains(&d.op) {
                    tracked_ops.push(d.op);
                    tb.push(None);
                    te.push(None);
                }
            }
        }
        let round = i as u64 + 1;
        let live: Vec<&RoundRecord> = round_recs
            .iter()
            .copied()
            .filter(|r| {
                matches!(
                    &r.post,
                    PostState::Log {
                        log_len: Some(_),
                        ..
                    }
                )
            })
            .collect();
        for (idx, op) in tracked_ops.iter().enumerate() {
            let digests: Vec<&crate::record::TrackedDigest> = live
                .iter()
                .filter_map(|r| r.tracked.iter().find(|d| d.op == *op))
                .collect();
            if digests.is_empty() {
                continue;
            }
            if digests.iter().all(|d| d.present) {
                tb[idx].get_or_insert(round);
                let first = (digests[0].pos, digests[0].prefix_hash);
                let agreed =
                    first.0.is_some() && digests.iter().all(|d| (d.pos, d.prefix_hash) == first);
                match (agreed, te[idx]) {
                    (true, None) => te[idx] = Some(round),
                    (false, Some(_)) => te[idx] = None,
                    _ => {}
                }
            } else if te[idx].is_some() {
                te[idx] = None;
            }
        }
        let min_exec = exec_lens.iter().min().copied().unwrap_or(0);
        let max_exec = exec_lens.iter().max().copied().unwrap_or(0);
        let arr = arrivals.and_then(|a| a.get(i)).copied().unwrap_or(0);
        let lcp = if compact {
            min_exec.to_string()
        } else {
            String::new()
        };
        metrics_csv.push_str(&format!(
            "{},{nonbot},{blocked},{useful},{},{max_log_len},{min_exec},{max_exec},{arr},{lcp}\n",
            i + 1,
            hashes.len()
        ));
        messages_csv.push_str(&totals.csv_row(i + 1));
    }

    let landmarks = tracked_ops
        .iter()
        .enumerate()
        .map(|(idx, &op)| Landmark {
            op,
            all_logs_round: tb[idx],
            prefix_fixed_round: te[idx],
        })
        .collect();
    Ok(Collected {
        metrics_csv,
        messages_csv,
        summary: Summary {
            convergence_round: None,
            timing: timing_stats(records),
        },
        gates,
        landmarks,
    })
}

pub fn aggregate(records: &[Vec<RoundRecord>], _spec: &NodeRunSpec) -> Result<Collected, String> {
    if records.is_empty() || records.iter().all(|r| r.is_empty()) {
        return Err("no records to aggregate".to_string());
    }
    let rounds = records.iter().map(|r| r.len()).max().unwrap_or(0);
    let rounds_consistent = records.iter().all(|r| r.len() == rounds);

    let mut metrics_csv = format!("{VALUE_METRICS_CSV_HEADER}\n");
    let mut messages_csv = format!("{MESSAGES_CSV_HEADER}\n");
    let mut gates = Gates {
        rounds_consistent,
        safety_ok: true,
        ..Gates::default()
    };
    let mut candidate: Option<(u64, u64)> = None;

    for i in 0..rounds {
        let round_recs: Vec<&RoundRecord> = records.iter().filter_map(|r| r.get(i)).collect();
        let mut holders = 0u32;
        let mut undecided = 0u32;
        let mut blocked = 0u32;
        let mut useful = 0u32;
        let mut values = BTreeSet::new();
        let mut totals = MsgTotals::default();
        for rec in &round_recs {
            match &rec.post {
                PostState::Value(Some(v)) => {
                    holders += 1;
                    values.insert(*v);
                }
                PostState::Value(None) => undecided += 1,
                PostState::Log { .. } => {
                    return Err("SMR aggregation lands with the SMR engine arms".to_string());
                }
            }
            blocked += u32::from(rec.blocked);
            useful += u32::from(rec.snap_held && !rec.blocked);
            totals.add(rec);
            gates.accumulate(rec);
        }
        metrics_csv.push_str(&format!(
            "{},{holders},{undecided},{blocked},{useful},{}\n",
            i + 1,
            values.len()
        ));
        messages_csv.push_str(&totals.csv_row(i + 1));

        let unanimous = holders > 0 && values.len() == 1;
        let value = values.first().copied();
        candidate = match (unanimous, candidate) {
            (true, None) => Some((i as u64 + 1, value.expect("unanimous"))),
            (true, Some((_, v))) if value != Some(v) => {
                Some((i as u64 + 1, value.expect("unanimous")))
            }
            (true, keep) => keep,
            (false, _) => None,
        };
    }

    Ok(Collected {
        metrics_csv,
        messages_csv,
        summary: Summary {
            convergence_round: candidate.map(|(r, _)| r),
            timing: timing_stats(records),
        },
        gates,
        landmarks: Vec::new(),
    })
}

/// `summary.json` body shared by every bin that writes collector results.
pub fn summary_json(out: &Collected) -> String {
    format!(
        "{{\"convergence_round\":{},\"timing\":{{\"t_last_reply_us_max\":{},\"t_last_reply_us_p99\":{},\"t_step_done_us_max\":{},\"t_step_done_us_p99\":{},\"t_step_compute_us_max\":{},\"t_step_compute_us_p99\":{}}},\"gates\":{{\"late_rep\":{},\"late_app\":{},\"dropped_past\":{},\"encode_err\":{},\"rounds_consistent\":{},\"safety_ok\":{}}}}}",
        out.summary
            .convergence_round
            .map_or("null".into(), |r| r.to_string()),
        out.summary.timing.t_last_reply_us_max,
        out.summary.timing.t_last_reply_us_p99,
        out.summary.timing.t_step_done_us_max,
        out.summary.timing.t_step_done_us_p99,
        out.summary.timing.t_step_compute_us_max,
        out.summary.timing.t_step_compute_us_p99,
        out.gates.late_rep,
        out.gates.late_app,
        out.gates.dropped_past,
        out.gates.encode_err,
        out.gates.rounds_consistent,
        out.gates.safety_ok
    )
}
