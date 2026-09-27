//! Networked node: RoundEngine behind a thin Tokio shell (current_thread runtime is a Shadow requirement).

use network_node::cli::arg;
use network_node::engine::RoundEngine;
use network_node::net::read_frame;
use network_node::spec::{NodeRunSpec, SyncMode};
use network_node::wire::{Msg, decode, encode};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

fn die(msg: &str) -> ! {
    network_node::cli::die("node", msg)
}

fn main() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    rt.block_on(run());
}

async fn run() {
    let spec_path = arg("--spec").unwrap_or_else(|| die("--spec required"));
    let peers_path = arg("--peers").unwrap_or_else(|| die("--peers required"));
    let node_id: u32 = arg("--node-id")
        .unwrap_or_else(|| die("--node-id required"))
        .parse()
        .unwrap_or_else(|_| die("--node-id must be an integer"));

    let spec_json = std::fs::read_to_string(&spec_path)
        .unwrap_or_else(|e| die(&format!("read {spec_path}: {e}")));
    let spec = NodeRunSpec::from_json(&spec_json).unwrap_or_else(|e| die(&format!("spec: {e}")));
    let peers: Vec<String> = std::fs::read_to_string(&peers_path)
        .unwrap_or_else(|e| die(&format!("read {peers_path}: {e}")))
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    if peers.len() != spec.n() {
        die(&format!(
            "peers.txt has {} entries, spec n = {}",
            peers.len(),
            spec.n()
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    for p in &peers {
        if !seen.insert(p.clone()) {
            die(&format!("duplicate peer entry {p}"));
        }
    }

    let mut engine =
        RoundEngine::new(&spec, node_id).unwrap_or_else(|e| die(&format!("engine: {e}")));
    let max_rounds = spec.max_rounds() as u64;
    let max_frame = spec.max_frame_bytes;
    let round_ms = spec.round_ms;

    let own = &peers[node_id as usize];
    let port = own
        .rsplit(':')
        .next()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or_else(|| die(&format!("no port in peer entry {own}")));
    let listener = TcpListener::bind(("0.0.0.0", port))
        .await
        .unwrap_or_else(|e| die(&format!("bind :{port}: {e}")));

    let pad = spec.payload_bytes;
    let (in_tx, mut in_rx) = mpsc::unbounded_channel::<(Msg, mpsc::UnboundedSender<Msg>)>();
    let accept_tx = in_tx.clone();
    tokio::spawn(async move {
        let mut accept_failures = 0u32;
        loop {
            let (mut stream, _) = match listener.accept().await {
                Ok(conn) => {
                    accept_failures = 0;
                    conn
                }
                Err(e) => {
                    accept_failures += 1;
                    eprintln!("node: accept failed ({accept_failures} consecutive): {e}");
                    if accept_failures >= 100 {
                        eprintln!("node: accept loop wedged — dying");
                        std::process::exit(2);
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    continue;
                }
            };
            let tx = accept_tx.clone();
            tokio::spawn(async move {
                let Ok(frame) = read_frame(&mut stream, max_frame).await else {
                    return;
                };
                let msg = match decode(&frame, max_frame) {
                    Ok(msg) => msg,
                    Err(e) => {
                        eprintln!("node: inbound frame decoded FAILED (wire bug?): {e}");
                        return;
                    }
                };
                let (reply_tx, mut reply_rx) = mpsc::unbounded_channel::<Msg>();
                if tx.send((msg, reply_tx)).is_err() {
                    return;
                }
                while let Some(reply) = reply_rx.recv().await {
                    let Ok(frame) = encode(&reply, max_frame, pad) else {
                        continue;
                    };
                    let _ = stream.write_all(&frame).await;
                }
            });
        }
    });

    let mut coord = match spec.sync {
        SyncMode::Barrier => {
            let addr = arg("--coordinator")
                .unwrap_or_else(|| die("--coordinator required in barrier mode"));
            Some(
                TcpStream::connect(addr.as_str())
                    .await
                    .unwrap_or_else(|e| die(&format!("coordinator {addr}: {e}"))),
            )
        }
        SyncMode::Timer => None,
    };

    // Shadow starts every host at sim-time 0, so process start aligns the round grid there
    let t0 = match arg("--start-at-ms") {
        Some(ms) => {
            let target = ms.parse::<u64>().unwrap_or_else(|_| die("--start-at-ms"));
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_millis() as u64;
            tokio::time::Instant::now() + Duration::from_millis(target.saturating_sub(now))
        }
        None => tokio::time::Instant::now() + Duration::from_millis(spec.start_offset_ms),
    };

    for round in 1..=max_rounds {
        let mut t_last_reply = 0u64;
        let round_started = tokio::time::Instant::now();
        // deviation: see node/networked-node-architecture.md (F13)
        let deadline = match spec.sync {
            SyncMode::Timer => t0 + Duration::from_millis(round_ms * round),
            SyncMode::Barrier => round_started + Duration::from_millis(round_ms),
        };

        for (target, msg) in engine.on_round_start() {
            let addr = peers[target as usize].clone();
            let Ok(frame) = encode(&msg, max_frame, pad) else {
                continue;
            };
            let tx = in_tx.clone();
            tokio::spawn(async move {
                let Ok(mut stream) = TcpStream::connect(addr.as_str()).await else {
                    return;
                };
                if stream.write_all(&frame).await.is_err() {
                    return;
                }
                let Ok(reply_frame) = read_frame(&mut stream, max_frame).await else {
                    return;
                };
                if let Ok(reply) = decode(&reply_frame, max_frame)
                    .inspect_err(|e| eprintln!("node: reply decode FAILED (wire bug?): {e}"))
                {
                    let (ignore_tx, _ignore_rx) = mpsc::unbounded_channel::<Msg>();
                    let _ = tx.send((reply, ignore_tx));
                }
            });
        }

        loop {
            let now = tokio::time::Instant::now();
            if now >= deadline {
                break;
            }
            match tokio::time::timeout_at(deadline, in_rx.recv()).await {
                Ok(Some((msg, reply_tx))) => {
                    if matches!(msg, Msg::PullReply { .. }) {
                        t_last_reply = round_started.elapsed().as_micros() as u64;
                    }
                    for (to, reply) in engine.on_message(msg) {
                        match reply {
                            Msg::PullReply { .. } | Msg::ClientAck { .. } => {
                                let _ = reply_tx.send(reply);
                            }
                            _ => {
                                let addr = peers[to as usize].clone();
                                let Ok(frame) = encode(&reply, max_frame, pad) else {
                                    continue;
                                };
                                tokio::spawn(async move {
                                    let Ok(mut stream) = TcpStream::connect(addr.as_str()).await
                                    else {
                                        return;
                                    };
                                    let _ = stream.write_all(&frame).await;
                                });
                            }
                        }
                    }
                }
                Ok(None) => break,
                Err(_) => break,
            }
        }

        let compute_started = tokio::time::Instant::now();
        let mut record = engine.on_round_end();
        record.t_step_compute_us = compute_started.elapsed().as_micros() as u64;
        record.t_last_reply_us = t_last_reply;
        record.t_step_done_us = round_started.elapsed().as_micros() as u64;
        println!("{}", record.to_jsonl());

        if let Some(stream) = coord.as_mut() {
            let done = encode(
                &Msg::RoundDone {
                    round,
                    from: node_id,
                },
                max_frame,
                0,
            )
            .expect("control frame");
            if stream.write_all(&done).await.is_err() {
                die("coordinator write failed");
            }
            match read_frame(stream, max_frame)
                .await
                .map(|f| decode(&f, max_frame))
            {
                Ok(Ok(Msg::RoundGo { round: go })) if go == round => {}
                other => die(&format!("barrier desync at round {round}: {other:?}")),
            }
        }
    }
}
