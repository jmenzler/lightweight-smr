//! §5 certificate checks (Thm 6, at-ack, tamper, p. 29 staleness) over live compact runs.

use protocol::certificates::Certificate;
use protocol::compact::{ClientCommand, Entry};
use sim::certs::{CertHarness, VerifyTally};
use sim::runlog::{ledger_target, log_smr_run_to};
use sim::smr::{ClientModel, Proto, SmrScenario, SmrState};
use sim::{BlockSchedule, Config};

const SEEDS: u64 = 10;
const SEED_BASE: u64 = 980_000;
const NS: [usize; 2] = [32, 64];
const BETAS: [f64; 2] = [0.0, 0.1];
const SIGMA: f64 = 1.0;
const CLIENTS: [u32; 3] = [10, 11, 12];
const ROUNDS: usize = 400;
const CHECK_EVERY: usize = 50;

fn t_commit(n: usize) -> u64 {
    // 5·log₂ n: tail-governed under load, with headroom over the sparse wait-policy arrival rate.
    5 * (n as u64).ilog2() as u64
}

struct Actor {
    client: u32,
    next_op: u64,
    issued: u64,
}

fn main() {
    let mut failures: Vec<String> = Vec::new();
    for (arm, &beta) in BETAS.iter().enumerate() {
        for (n_idx, &n) in NS.iter().enumerate() {
            for i in 0..SEEDS {
                let seed = SEED_BASE + 1000 * arm as u64 + 100 * n_idx as u64 + i;
                let cell = format!("beta={beta} n={n} seed={seed}");
                match run_cell(beta, n, seed) {
                    Ok(line) => println!("{cell}: {line}"),
                    Err(e) => {
                        println!("{cell}: FAIL — {e}");
                        failures.push(format!("{cell}: {e}"));
                    }
                }
            }
        }
    }
    if failures.is_empty() {
        println!("E6 PASS — Thm 6 + freshness on every cell");
    } else {
        println!("E6 FAIL — {} cells: {failures:?}", failures.len());
        std::process::exit(1);
    }
}

fn run_cell(beta: f64, n: usize, seed: u64) -> Result<String, String> {
    let proto = Proto::Compact {
        t_commit_rounds: t_commit(n),
    };
    let mut state = SmrState::new(
        n,
        Config::default(),
        proto,
        SIGMA,
        seed,
        &[],
        &[],
        ClientModel::Unique,
        None,
    );
    let mut harness = CertHarness::new(n);
    let mut actors: Vec<Actor> = CLIENTS
        .iter()
        .map(|&client| Actor {
            client,
            next_op: 100 * client as u64,
            issued: 0,
        })
        .collect();
    for a in &actors {
        harness.register_client(a.client);
    }

    let mut checks = 0usize;
    let mut at_ack_ok = 0usize;
    let mut at_ack_total = 0usize;
    let mut stale: Option<(Certificate, u64)> = None;
    let mut mask = vec![false; n];

    for round in 1..=ROUNDS {
        for a in &mut actors {
            // §4 base client: next command only after the previous ack (the p. 29 wait mitigation).
            if harness.acked_sn(a.client) == a.issued {
                if a.issued > 0 {
                    let cert = harness
                        .client(a.client)
                        .build_certificate(a.issued)
                        .ok_or("no certificate at ack")?;
                    let t = harness.verify_everywhere(a.client, &cert, &mask);
                    at_ack_total += 1;
                    at_ack_ok += usize::from(t.covered > 0 && t.accepted == t.covered);
                }
                let op = a.next_op;
                a.next_op += 1;
                state
                    .inject(a.client, op, None)
                    .map_err(|e| e.to_string())?;
                a.issued += 1;
                harness.record_issue(a.client, a.issued, op);
            }
        }

        mask = state.sample_mask(state.blocked_count(beta));
        state.step_masked(&mask);
        harness.observe_round(&state);

        if !state.report().safety_ok {
            return Err(format!("split brain at round {round}"));
        }
        if !harness.roots_consistent() {
            return Err(format!("root divergence at round {round}"));
        }
        if stale.is_none() && harness.acked_sn(CLIENTS[0]) >= 1 {
            let sn = harness.acked_sn(CLIENTS[0]);
            stale = harness
                .client(CLIENTS[0])
                .build_certificate(sn)
                .map(|c| (c, sn));
        }
        if round % CHECK_EVERY == 0 {
            checks += verify_all(&harness, &actors, &mask)?;
        }
    }

    let no_block = vec![false; n];
    checks += verify_all(&harness, &actors, &no_block)?;
    tamper_checks(&harness, &no_block)?;

    if at_ack_ok != at_ack_total {
        return Err(format!(
            "at-ack verification failed: {at_ack_ok}/{at_ack_total}"
        ));
    }
    let (stale_cert, stale_sn) = stale.ok_or("no stale certificate captured")?;
    if harness.acked_sn(CLIENTS[0]) < stale_sn + 2 {
        return Err("fewer than 2 commits after the stale capture".into());
    }
    let t = harness.verify_everywhere(CLIENTS[0], &stale_cert, &no_block);
    if t.covered == 0 || t.accepted != 0 {
        return Err(format!(
            "stale newest cert: accepted {}/{}",
            t.accepted, t.covered
        ));
    }

    ledger(&state, beta, n, seed, checks, at_ack_ok, at_ack_total);
    let committed: u64 = CLIENTS.iter().map(|&c| harness.acked_sn(c)).sum();
    Ok(format!(
        "ok — {committed} commands, {checks} cert checks, at-ack {at_ack_ok}/{at_ack_total}, stale rejected {}/{} servers",
        t.covered, t.covered
    ))
}

// Thm 6: every committed sn of every actor verifies at every covering useful server.
fn verify_all(harness: &CertHarness, actors: &[Actor], mask: &[bool]) -> Result<usize, String> {
    let mut checks = 0;
    for a in actors {
        for sn in 1..=harness.acked_sn(a.client) {
            let cert = harness
                .client(a.client)
                .build_certificate(sn)
                .ok_or_else(|| format!("client {} sn {sn}: no certificate", a.client))?;
            let VerifyTally { covered, accepted } =
                harness.verify_everywhere(a.client, &cert, mask);
            if accepted != covered {
                return Err(format!(
                    "client {} sn {sn}: {accepted}/{covered} accepted",
                    a.client
                ));
            }
            checks += covered;
        }
    }
    Ok(checks)
}

// Truncation is not tampering: a shorter chain is a weaker certificate and may still verify.
fn tamper_checks(harness: &CertHarness, mask: &[bool]) -> Result<(), String> {
    // First command: evicted from every last-two window, so its chain is load-bearing.
    let client = CLIENTS[0];
    if harness.acked_sn(client) < 3 {
        return Err("not enough commits for tamper checks".into());
    }
    let sn = 1;
    let Some(Certificate::Chained { entry, pos, chain }) =
        harness.client(client).build_certificate(sn)
    else {
        return Err("expected a chained certificate".into());
    };
    let Entry::Cmd(c) = entry else {
        return Err("expected a command entry".into());
    };
    let variants = [
        (
            "wrong-pos",
            Certificate::Chained {
                entry: Entry::Cmd(c),
                pos: pos + 1,
                chain: chain.clone(),
            },
        ),
        (
            "altered-op",
            Certificate::Chained {
                entry: Entry::Cmd(ClientCommand {
                    op: c.op + 1_000_000,
                    ..c
                }),
                pos,
                chain: chain.clone(),
            },
        ),
        ("corrupted-link", {
            let mut bad = chain.clone();
            bad[0][0] ^= 0xff;
            Certificate::Chained {
                entry: Entry::Cmd(c),
                pos,
                chain: bad,
            }
        }),
    ];
    for (name, cert) in &variants {
        let t = harness.verify_everywhere(client, cert, mask);
        if t.accepted != 0 {
            return Err(format!(
                "tamper {name}: accepted {}/{}",
                t.accepted, t.covered
            ));
        }
    }
    Ok(())
}

// Ledgered as its byte-identical batch replay (actual injection rounds exported).
fn ledger(
    state: &SmrState,
    beta: f64,
    n: usize,
    seed: u64,
    checks: usize,
    ok: usize,
    total: usize,
) {
    let Some(ledger) = ledger_target(None) else {
        return;
    };
    let scenario = SmrScenario {
        n,
        seed,
        cfg: Config::default(),
        sigma: SIGMA,
        proto: Proto::Compact {
            t_commit_rounds: t_commit(n),
        },
        injections: state.injections(),
        max_rounds: ROUNDS,
        schedule: BlockSchedule::FreshPerRound { fraction: beta },
        traffic: None,
        client_model: ClientModel::Unique,
        certs: false,
        manual_blocks: Vec::new(),
        partition: None,
        halt_on_violation: false,
        merge_policy: Default::default(),
        repeated_commit: Default::default(),
    };
    let extra = serde_json::json!({
        "exp": "E6",
        "beta": beta,
        "cert_checks": checks,
        "at_ack_ok": ok,
        "at_ack_total": total,
    });
    log_smr_run_to(&ledger, &scenario, &state.report(), Some(extra))
        .expect("run ledger write failed (set SIM_RUNLOG=off to opt out)");
}
