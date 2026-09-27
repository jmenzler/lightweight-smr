//! Compact safety-abort diagnostic. Args: `[t_commit] [seed] [settle|v] [n] [blocked_count] [rate]`

use protocol::compact::Entry;
use sim::Config;
use sim::smr::{ClientModel, Proto, SmrState, TrafficPhase, prefixes_consistent};

fn fmt(e: &Entry) -> String {
    match e {
        Entry::Cmd(c) => {
            let auto = if c.op >= 1 << 40 { " auto" } else { "" };
            format!("Cmd(client={}, sn={}, op={}{auto})", c.client, c.sn, c.op)
        }
        Entry::Null { client, sn } => format!("Null(client={client}, sn={sn})"),
        Entry::Nop(x) => format!("Nop({x})"),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let t_commit: u64 = args.get(1).map(|s| s.parse().unwrap()).unwrap_or(21);
    let seed: u64 = args.get(2).map(|s| s.parse().unwrap()).unwrap_or(950_000);
    let verbose = args.get(3).is_some();
    let n: usize = args.get(4).map(|s| s.parse().unwrap()).unwrap_or(64);
    let blocked: usize = args.get(5).map(|s| s.parse().unwrap()).unwrap_or(0);
    let rate: usize = args.get(6).map(|s| s.parse().unwrap()).unwrap_or(4);
    let pmf = sim::smr::point_mass_pmf(rate);
    let traffic = vec![
        TrafficPhase {
            from_round: 1,
            arrivals_pmf: pmf,
        },
        TrafficPhase {
            from_round: 2001,
            arrivals_pmf: vec![1.0],
        },
    ];
    let mut state = SmrState::new(
        n,
        Config::default(),
        Proto::Compact {
            t_commit_rounds: t_commit,
        },
        1.0,
        seed,
        &[],
        &traffic,
        ClientModel::Unique,
        None,
    );
    let settle_mode = verbose && args[3] == "settle";
    let mut max_unsettled_age: u64 = 0;
    let mut max_age_round = 0usize;
    let mut rounds_ge: [u32; 3] = [0, 0, 0];
    let mask = vec![false; n];
    for round in 1..=2500usize {
        state.draw_arrivals();
        let status = if blocked > 0 {
            state.step_sampled(blocked)
        } else {
            state.step_masked(&mask)
        };
        let ex = state.executed_seqs();
        if settle_mode {
            let logs = state.compact_logs();
            let e_min = ex.iter().map(Vec::len).min().unwrap();
            let vlen = |i: usize| ex[i].len() + logs[i].as_ref().map_or(0, Vec::len);
            let entry_at = |i: usize, p: usize| -> &Entry {
                if p < ex[i].len() {
                    &ex[i][p]
                } else {
                    &logs[i].as_ref().unwrap()[p - ex[i].len()].entry
                }
            };
            let min_vlen = (0..n).map(vlen).min().unwrap();
            let mut lcp = min_vlen;
            'scan: for p in e_min..min_vlen {
                let first = entry_at(0, p);
                for i in 1..n {
                    if entry_at(i, p) != first {
                        lcp = p;
                        break 'scan;
                    }
                }
            }
            let mut round_max: u64 = 0;
            for (i, log) in logs.iter().enumerate() {
                let Some(log) = log else { continue };
                for (j, t) in log.iter().enumerate() {
                    if ex[i].len() + j >= lcp {
                        round_max = round_max.max((round as u64).saturating_sub(t.round));
                    }
                }
            }
            if round_max > max_unsettled_age {
                max_unsettled_age = round_max;
                max_age_round = round;
            }
            for (slot, thr) in [15u64, 21, 25].iter().enumerate() {
                if round_max >= *thr {
                    rounds_ge[slot] += 1;
                }
            }
        }
        if !prefixes_consistent(&ex) {
            println!("T={t_commit} seed={seed}: FIRST VIOLATION at round {round}");
            if !verbose {
                return;
            }
            let longest = (0..n).max_by_key(|&i| ex[i].len()).unwrap();
            let bad: Vec<usize> = (0..n)
                .filter(|&i| ex[i][..] != ex[longest][..ex[i].len()])
                .collect();
            println!(
                "longest node {longest} len {}, disagreeing nodes ({}): {:?}",
                ex[longest].len(),
                bad.len(),
                &bad[..bad.len().min(10)]
            );
            for &i in bad.iter().take(3) {
                let pos = ex[i]
                    .iter()
                    .zip(ex[longest].iter())
                    .position(|(a, b)| a != b)
                    .unwrap();
                println!("node {i} len {} first mismatch at pos {pos}:", ex[i].len());
                let lo = pos.saturating_sub(3);
                let hi = (pos + 4).min(ex[i].len());
                for (p, (a, b)) in ex[i]
                    .iter()
                    .zip(ex[longest].iter())
                    .enumerate()
                    .take(hi)
                    .skip(lo)
                {
                    let marker = if p == pos { " <-- " } else { "     " };
                    println!(
                        "  pos {p}:{marker}node{i}={} | node{longest}={}",
                        fmt(a),
                        fmt(b)
                    );
                }
            }
            return;
        }
        if status.dead.is_some() {
            println!("dead at {round}");
            return;
        }
    }
    println!("T={t_commit} seed={seed}: no violation in 2500 rounds");
    if settle_mode {
        println!(
            "  max unsettled age {max_unsettled_age} (round {max_age_round}); \
             rounds with age>=15: {}, >=21: {}, >=25: {}",
            rounds_ge[0], rounds_ge[1], rounds_ge[2]
        );
    }
}
