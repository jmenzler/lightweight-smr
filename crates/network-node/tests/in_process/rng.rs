//! Per-role RNG derivation (DP-8) + with-replacement peer sampling (F12).

use network_node::rng::{derive_rng, draw_targets};
use rand::Rng;

#[test]
fn derivation_is_deterministic_and_role_id_separated() {
    let a: Vec<u64> = {
        let mut r = derive_rng(42, "node", 7);
        (0..8).map(|_| r.random()).collect()
    };
    let b: Vec<u64> = {
        let mut r = derive_rng(42, "node", 7);
        (0..8).map(|_| r.random()).collect()
    };
    assert_eq!(a, b, "same (seed, role, id) → identical stream on any host");

    let other_id: Vec<u64> = {
        let mut r = derive_rng(42, "node", 8);
        (0..8).map(|_| r.random()).collect()
    };
    let other_role: Vec<u64> = {
        let mut r = derive_rng(42, "blocking", 7);
        (0..8).map(|_| r.random()).collect()
    };
    let other_seed: Vec<u64> = {
        let mut r = derive_rng(43, "node", 7);
        (0..8).map(|_| r.random()).collect()
    };
    assert_ne!(a, other_id);
    assert_ne!(a, other_role);
    assert_ne!(a, other_seed);
}

#[test]
fn role_id_concatenation_cannot_collide() {
    // ("ab", 1) vs ("a", ...) style ambiguity: framing in the hash input must
    // separate role from id.
    let mut a = derive_rng(1, "node1", 2);
    let mut b = derive_rng(1, "node", 12);
    let sa: Vec<u64> = (0..4).map(|_| a.random()).collect();
    let sb: Vec<u64> = (0..4).map(|_| b.random()).collect();
    assert_ne!(sa, sb);
}

#[test]
fn targets_are_uniform_with_replacement_including_self() {
    let n = 16;
    let k = 6;
    let mut rng = derive_rng(7, "node", 3);
    let mut counts = vec![0usize; n];
    let mut dupe_rounds = 0;
    let rounds = 20_000;
    for _ in 0..rounds {
        let t = draw_targets(&mut rng, n, k);
        assert_eq!(t.len(), k, "exactly k draws");
        let mut seen = vec![false; n];
        let mut dupe = false;
        for &id in &t {
            let id = id as usize;
            assert!(id < n);
            if seen[id] {
                dupe = true;
            }
            seen[id] = true;
            counts[id] += 1;
        }
        dupe_rounds += usize::from(dupe);
    }
    let total = (rounds * k) as f64;
    for (id, c) in counts.iter().enumerate() {
        let p = *c as f64 / total;
        assert!(
            (p - 1.0 / n as f64).abs() < 0.005,
            "target {id} frequency {p:.4} off uniform 1/{n} — self id included like any other"
        );
    }
    // With replacement, k=6 of n=16: dupes must occur in a large sample.
    assert!(
        dupe_rounds > rounds / 10,
        "with-replacement duplicates: {dupe_rounds}"
    );
}
