//! `gen_schedule`'s pure generator: golden JSON for a fixed seed, wire
//! round-trip through `sim::spec::InjectionSpec`, and the per-client rank
//! sanity check via `sim::smr::injection_sns`.

use network_node::schedule::generate;
use sim::smr::{Injection, injection_sns};
use sim::spec::InjectionSpec;
use std::collections::HashMap;

#[test]
fn golden_seed_42_n3_two_clients() {
    let schedule = generate(42, 10, &[0.5, 0.5], 2, 2, 3).expect("generate");
    let json = serde_json::to_string(&schedule).expect("serialize");
    assert_eq!(
        json,
        r#"[{"round":1,"client":1,"op":1,"target":0},{"round":4,"client":2,"op":2,"target":0},{"round":5,"client":1,"op":3,"target":2},{"round":6,"client":2,"op":4,"target":0}]"#
    );
}

#[test]
fn output_deserializes_as_injection_spec_vec() {
    let schedule = generate(7, 30, &[0.2, 0.3, 0.5], 3, 4, 5).expect("generate");
    let json = serde_json::to_string(&schedule).expect("serialize");
    let parsed: Vec<InjectionSpec> = serde_json::from_str(&json).expect("wire-compatible");
    assert_eq!(parsed.len(), schedule.len());
    for (a, b) in schedule.iter().zip(&parsed) {
        assert_eq!(a.round, b.round);
        assert_eq!(a.client, b.client);
        assert_eq!(a.op, b.op);
        assert_eq!(a.target, b.target);
        assert!(b.target.is_some(), "gen_schedule always pins a target");
    }
}

#[test]
fn every_client_gets_ranks_one_through_per_client() {
    let clients = 4u32;
    let per_client = 5u32;
    let schedule = generate(99, 200, &[0.4, 0.4, 0.2], clients, per_client, 6).expect("generate");
    assert_eq!(schedule.len(), (clients * per_client) as usize);

    let injections: Vec<Injection> = schedule
        .iter()
        .map(|i| Injection {
            round: i.round,
            client: i.client,
            op: i.op,
            target: i.target,
        })
        .collect();
    let sns = injection_sns(&injections);

    let mut by_client: HashMap<u32, Vec<(usize, u64)>> = HashMap::new();
    for (inj, &sn) in injections.iter().zip(&sns) {
        by_client
            .entry(inj.client)
            .or_default()
            .push((inj.round, sn));
    }
    assert_eq!(by_client.len(), clients as usize);
    for (client, mut ranks) in by_client {
        ranks.sort_by_key(|&(round, _)| round);
        let sns: Vec<u64> = ranks.iter().map(|&(_, sn)| sn).collect();
        assert_eq!(
            sns,
            (1..=per_client as u64).collect::<Vec<_>>(),
            "client {client} ranks"
        );
    }
}

#[test]
fn rejects_a_horizon_too_short_to_fit_every_command() {
    // pmf is point-mass-0: no round ever draws an arrival.
    assert!(
        generate(1, 5, &[1.0], 2, 3, 4).is_err(),
        "zero arrivals forever can never place 6 commands"
    );
}

#[test]
fn rejects_targets_and_ops_that_would_reach_the_auto_arrival_namespace() {
    assert!(
        generate(1, 10, &[0.0, 1.0], sim::smr::AUTO_CLIENT_BASE, 1, 4).is_err(),
        "clients must stay below the auto-arrival client base"
    );
    assert!(
        generate(1, 10, &[0.0, 1.0], 1, 0, 0).is_err(),
        "n must be at least 1 for a target draw"
    );
}
