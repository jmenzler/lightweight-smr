use protocol::certificates::{
    ClientChains, Hash, MmrForest, Notification, ServerCertState, leaf_hash,
};
use protocol::compact::{ClientCommand, Entry};
use sha2::{Digest, Sha256};

fn cmd(client: u32, sn: u64, op: u64) -> Entry {
    Entry::Cmd(ClientCommand { client, sn, op })
}

/// Reference fold: climb from the leaf using the chain, sibling side taken
/// from the position bits (interior = h(0x01‖left‖right)).
fn ref_fold(leaf: Hash, pos: u64, chain: &[Hash]) -> Hash {
    let mut h = leaf;
    for (i, sib) in chain.iter().enumerate() {
        let mut s = Sha256::new();
        s.update([0x01]);
        if (pos >> i) & 1 == 1 {
            s.update(sib);
            s.update(h);
        } else {
            s.update(h);
            s.update(sib);
        }
        h = s.finalize().into();
    }
    h
}

/// Root of the forest tree covering `pos` (trees cover consecutive ranges,
/// tallest first, §5 p. 27).
fn covering_root(forest: &MmrForest, pos: u64) -> Hash {
    let mut start = 0u64;
    for (height, root) in forest.peak_heights().into_iter().zip(forest.roots()) {
        let size = 1u64 << height;
        if pos < start + size {
            return root;
        }
        start += size;
    }
    panic!("pos {pos} beyond forest");
}

/// Heights of the set bits of m, descending — the paper's unique decomposition
/// m = 2^{i₁} + … + 2^{i_a} with i₁ > … > i_a (§5 p. 27).
fn binary_decomposition(m: u64) -> Vec<u32> {
    (0..64).rev().filter(|i| (m >> i) & 1 == 1).collect()
}

#[test]
fn server_keeps_the_last_two_commands_per_client_and_notifies_with_the_previous_chain() {
    let mut server = ServerCertState::new();
    server.append(&Entry::Nop(0));
    server.append(&cmd(7, 1, 100));
    // Only one command of client 7 committed: no x₁ yet, nothing to attach.
    assert!(server.notification(7).is_none());

    server.append(&Entry::Nop(1));
    server.append(&cmd(7, 2, 101));
    // Window is (x₁ = sn 1 at pos 1, x₂ = sn 2 at pos 3); the ack for x₂
    // carries hc(x₁) and p(x₁) (§5 p. 28).
    let note = server.notification(7).expect("x1 exists");
    assert_eq!(note.prev_pos, 1);
    assert_eq!(note.prev_sn, 1);

    server.append(&cmd(7, 3, 102));
    // x₁ rotated out: window is now (sn 2 at pos 3, sn 3 at pos 4).
    let note = server.notification(7).expect("window rotated");
    assert_eq!(note.prev_pos, 3);
    assert_eq!(note.prev_sn, 2);

    // A null command still advances the client's window (identity kept).
    server.append(&Entry::Null { client: 7, sn: 4 });
    let note = server.notification(7).expect("null rotates too");
    assert_eq!(note.prev_pos, 4);
    assert_eq!(note.prev_sn, 3);

    // Unknown client: nothing stored.
    assert!(server.notification(99).is_none());
}

#[test]
fn client_keeps_the_longer_chain_and_its_next_sn_is_k_plus_2() {
    let h = [9u8; 32];
    let note = |pos, sn, len| Notification {
        prev_chain: vec![h; len],
        prev_pos: pos,
        prev_sn: sn,
    };

    let mut client = ClientChains::new(7);
    client.record_issue(&cmd(7, 1, 100));
    client.on_commit_ack(1, None);
    client.record_issue(&cmd(7, 2, 101));
    client.on_commit_ack(2, Some(note(1, 1, 1)));
    // k = 1 chains received → sn = k + 2 (§5 p. 28).
    assert_eq!(client.next_sn(), 3);
    assert_eq!(client.chain_for(1).unwrap().1.len(), 1);

    // Re-acks may carry chains of different length for the same command:
    // the longer one wins, a shorter one never regresses (§5 p. 28).
    client.on_commit_ack(2, Some(note(1, 1, 3)));
    assert_eq!(client.chain_for(1).unwrap().1.len(), 3);
    client.on_commit_ack(2, Some(note(1, 1, 2)));
    assert_eq!(client.chain_for(1).unwrap().1.len(), 3);

    // No chain yet for the newest committed command.
    assert!(client.chain_for(2).is_none());
}

#[test]
fn every_committed_command_verifies_at_any_useful_server() {
    // Client 7's commands interleaved with foreign traffic; after every ack,
    // certificates for ALL its committed commands verify at BOTH servers
    // (Thm 6 — the sweep over m exercises both proof cases and the
    // degenerate x = x_{k+1} certificate).
    let mut s1 = ServerCertState::new();
    let mut s2 = ServerCertState::new();
    let mut client = ClientChains::new(7);
    let mut sn = 0u64;
    for i in 0..48u64 {
        let entry = match i % 3 {
            0 => {
                sn += 1;
                let e = cmd(7, sn, 700 + sn);
                client.record_issue(&e);
                e
            }
            1 => Entry::Nop(i),
            _ => cmd(3, i / 3 + 1, 300 + i),
        };
        s1.append(&entry);
        s2.append(&entry);
        if sn > 0 {
            // Resubmission ack from one server, reply-time snapshot (§5 p. 28).
            client.on_commit_ack(sn, s1.notification(7));
        }
        for want in 1..=sn {
            let cert = client
                .build_certificate(want)
                .unwrap_or_else(|| panic!("no cert for sn {want} at m {}", i + 1));
            assert!(s1.verify(7, &cert), "sn {want} at m {} on s1", i + 1);
            assert!(s2.verify(7, &cert), "sn {want} at m {} on s2", i + 1);
        }
    }
}

/// Commits `count` commands of client 7 (interleaved with foreign traffic)
/// into a server, acking after each; returns (server, client).
fn committed_run(count: u64) -> (ServerCertState, ClientChains) {
    let mut server = ServerCertState::new();
    let mut client = ClientChains::new(7);
    for sn in 1..=count {
        server.append(&Entry::Nop(1000 + sn));
        let e = cmd(7, sn, 700 + sn);
        client.record_issue(&e);
        server.append(&e);
        client.on_commit_ack(sn, server.notification(7));
    }
    (server, client)
}

#[test]
fn tampered_certificates_are_rejected() {
    let (server, client) = committed_run(6);
    let cert = client.build_certificate(2).unwrap();
    assert!(server.verify(7, &cert), "untampered baseline");
    let protocol::certificates::Certificate::Chained { entry, pos, chain } = cert else {
        panic!("sn 2 must be a chained certificate")
    };
    let Entry::Cmd(c) = entry else { panic!() };

    let wrong_pos = protocol::certificates::Certificate::Chained {
        entry: Entry::Cmd(c),
        pos: pos + 1,
        chain: chain.clone(),
    };
    assert!(!server.verify(7, &wrong_pos));

    let altered = protocol::certificates::Certificate::Chained {
        entry: Entry::Cmd(ClientCommand { op: c.op + 1, ..c }),
        pos,
        chain: chain.clone(),
    };
    assert!(!server.verify(7, &altered));

    let truncated = protocol::certificates::Certificate::Chained {
        entry: Entry::Cmd(c),
        pos,
        chain: chain[..chain.len() - 1].to_vec(),
    };
    assert!(!server.verify(7, &truncated));

    let mut corrupted = chain.clone();
    corrupted[0][0] ^= 0xff;
    let bad_link = protocol::certificates::Certificate::Chained {
        entry: Entry::Cmd(c),
        pos,
        chain: corrupted,
    };
    assert!(!server.verify(7, &bad_link));

    assert!(chain.len() >= 2, "need ≥2 links to test reordering");
    let mut swapped = chain.clone();
    swapped.swap(0, 1);
    let reordered = protocol::certificates::Certificate::Chained {
        entry: Entry::Cmd(c),
        pos,
        chain: swapped,
    };
    assert!(!server.verify(7, &reordered));
}

#[test]
fn newest_certificate_survives_one_further_commit_and_dies_on_eviction() {
    // §5 p. 29 says stale certificates "might" fail — never that one newer
    // commit fails: the last-two window still holds x_{k+1} after ONE further
    // commit. Eviction (≥ 2 further commits) makes rejection deterministic
    // for the bare-newest certificate.
    let (mut server, client) = committed_run(4);
    let stale = client.build_certificate(4).unwrap();
    assert!(matches!(
        stale,
        protocol::certificates::Certificate::Newest { .. }
    ));
    assert!(server.verify(7, &stale), "fresh");

    server.append(&cmd(7, 5, 705));
    assert!(
        server.verify(7, &stale),
        "one further commit: still in the window"
    );

    server.append(&cmd(7, 6, 706));
    assert!(!server.verify(7, &stale), "two further commits: evicted");
}

#[test]
fn losing_one_chain_forfeits_only_that_command() {
    // §5 p. 27: loss of the chain for x forfeits x's certification only.
    let mut server = ServerCertState::new();
    let mut intact = ClientChains::new(7);
    let mut lossy = ClientChains::new(7);
    for sn in 1..=5u64 {
        server.append(&Entry::Nop(1000 + sn));
        let e = cmd(7, sn, 700 + sn);
        intact.record_issue(&e);
        lossy.record_issue(&e);
        server.append(&e);
        let note = server.notification(7);
        intact.on_commit_ack(sn, note.clone());
        // The lossy client never received (or lost) the chain for sn 2.
        lossy.on_commit_ack(sn, note.filter(|n| n.prev_sn != 2));
    }
    assert!(lossy.build_certificate(2).is_none(), "sn 2 forfeit");
    for sn in [1, 3, 4, 5] {
        let cert = lossy
            .build_certificate(sn)
            .unwrap_or_else(|| panic!("sn {sn}"));
        assert!(server.verify(7, &cert), "sn {sn} unaffected");
    }
}

#[test]
fn stored_chains_always_reach_their_trees_root() {
    // v2 p. 28: hc(x) = "the sibling hashes of x and all ancestors of x in
    // its Merkle hash tree" — after EVERY append, every stored chain must
    // fold from its leaf to the root of its covering tree.
    let mut server = ServerCertState::new();
    for i in 0..40u64 {
        let entry = match i % 3 {
            0 => Entry::Nop(i),
            1 => cmd(1, i / 3 + 1, 200 + i),
            _ => cmd(2, i / 3 + 1, 300 + i),
        };
        server.append(&entry);
        for client in [1u32, 2] {
            let Some((prev, last)) = server.last_two(client) else {
                continue;
            };
            for sc in prev.iter().copied().chain([last]) {
                assert_eq!(
                    ref_fold(sc.leaf, sc.pos, &sc.chain),
                    covering_root(server.forest(), sc.pos),
                    "m = {}, client {client}, pos {}",
                    i + 1,
                    sc.pos
                );
            }
        }
    }
}

#[test]
fn roots_pin_rfc6962_domain_separated_sha256() {
    // Hand-computed vector: two leaves → root = H(0x01 ‖ H(0x00‖enc(a)) ‖ H(0x00‖enc(b))).
    let (a, b) = (Entry::Nop(0), Entry::Nop(1));
    let mut forest = MmrForest::new();
    forest.append(leaf_hash(&a));
    forest.append(leaf_hash(&b));
    let expect_leaf = |e: &Entry| -> Hash {
        let Entry::Nop(op) = e else { panic!() };
        let mut s = Sha256::new();
        s.update([0x00, 0x03]);
        s.update(op.to_le_bytes());
        s.finalize().into()
    };
    let mut s = Sha256::new();
    s.update([0x01]);
    s.update(expect_leaf(&a));
    s.update(expect_leaf(&b));
    let root: Hash = s.finalize().into();
    assert_eq!(forest.roots(), vec![root]);
}

#[test]
fn forest_shape_is_the_binary_decomposition_of_m_for_every_m() {
    let mut forest = MmrForest::new();
    for m in 1..=64u64 {
        forest.append(leaf_hash(&Entry::Nop(m)));
        assert_eq!(forest.len(), m);
        assert_eq!(forest.peak_heights(), binary_decomposition(m), "m = {m}");
        assert_eq!(forest.roots().len(), m.count_ones() as usize, "m = {m}");
    }
}

#[test]
fn an_early_chain_still_verifies_after_the_tree_grows_past_it() {
    // A client whose newest ack is sn 2 holds a frozen chain for sn 1 (no
    // later chains to stitch). After foreign commits merge the forest well
    // past that pair, Thm 6 case (b) must still confirm sn 1 via the stored
    // hc(x_{k+1}) bridge at the lca-child level — the verifier folds the
    // chain PREFIX to lc(lca), not the whole chain (Lemma 5.1 / p. 29).
    let mut server = ServerCertState::new();
    let mut client = ClientChains::new(7);
    for sn in 1..=2u64 {
        let e = cmd(7, sn, 700 + sn);
        client.record_issue(&e);
        server.append(&e);
        client.on_commit_ack(sn, server.notification(7));
    }
    for i in 0..30u64 {
        server.append(&Entry::Nop(2000 + i));
    }
    let cert = client.build_certificate(1).unwrap();
    assert!(server.verify(7, &cert), "bridge via stored newest chain");
}

#[test]
fn verification_bridges_via_the_previous_slot_when_the_server_runs_one_ahead() {
    // Wait-policy timing: sn 3 is committed server-side but its ack is still
    // in flight, so the window is (sn 2, sn 3) while the client's newest
    // chained command x_{k+1} is sn 2 — stored in the PREV slot. p. 29 only
    // needs hc(x_{k+1}) "still stored"; the bridge must work from either slot.
    // Geometry that defeats a last-slot-only bridge: sn 1 at pos 1 (chain
    // frozen at length 1 when sn 2 was acked at m = 3), sn 3 across the
    // power-of-8 boundary at pos 8 → lca-child level vs sn 3 is 3 > 1, but
    // vs sn 2 (pos 2, the prev slot) it is 1.
    let mut server = ServerCertState::new();
    let mut client = ClientChains::new(7);
    server.append(&Entry::Nop(0));
    let issue = |client: &mut ClientChains, server: &mut ServerCertState, sn: u64| {
        let e = cmd(7, sn, 700 + sn);
        client.record_issue(&e);
        server.append(&e);
    };
    issue(&mut client, &mut server, 1); // pos 1
    client.on_commit_ack(1, server.notification(7));
    issue(&mut client, &mut server, 2); // pos 2
    client.on_commit_ack(2, server.notification(7));
    for i in 0..5u64 {
        server.append(&Entry::Nop(2000 + i)); // pos 3..=7
    }
    issue(&mut client, &mut server, 3); // pos 8, ack in flight
    for i in 0..8u64 {
        server.append(&Entry::Nop(3000 + i));
    }
    let cert = client.build_certificate(1).unwrap();
    assert!(
        server.verify(7, &cert),
        "bridge via prev-slot hc(x_{{k+1}})"
    );
}

#[test]
fn duplicate_position_notifications_cannot_break_certificate_building() {
    // Notifications are external input: two carrying the same prev_pos for
    // different sns must not panic (debug underflow in lca_child_level) or
    // silently corrupt the stitched chain — the duplicate is skipped.
    let (server, mut client) = committed_run(4);
    client.on_commit_ack(
        4,
        Some(Notification {
            prev_chain: vec![[7u8; 32]; 2],
            prev_pos: client.chain_for(1).unwrap().0,
            prev_sn: 9,
        }),
    );
    client.record_issue(&cmd(7, 9, 709));
    let cert = client.build_certificate(1).unwrap();
    assert!(
        server.verify(7, &cert),
        "sn 1 unaffected by the poisoned entry"
    );
}

#[test]
fn verify_binds_the_certificate_to_the_claimed_client() {
    // Strengthening beyond the paper (whose root-comparison case is client-
    // agnostic): the entry's own client id must match the claimed client, so
    // per-client verification tallies cannot be cross-contaminated.
    let (server, client) = committed_run(4);
    let cert = client.build_certificate(2).unwrap();
    assert!(server.verify(7, &cert));
    assert!(
        !server.verify(99, &cert),
        "foreign client id must not verify"
    );
}

#[test]
fn forged_certificate_for_an_uncommitted_command_is_rejected() {
    let (server, _) = committed_run(6);
    let never_committed = cmd(7, 3, 999_999);
    for pos in 0..12u64 {
        for len in 0..5usize {
            let forged = protocol::certificates::Certificate::Chained {
                entry: never_committed.clone(),
                pos,
                chain: vec![[0u8; 32]; len],
            };
            assert!(!server.verify(7, &forged), "pos {pos} len {len}");
        }
    }
}

#[test]
fn accepted_certificates_may_carry_garbage_beyond_the_used_prefix() {
    // Deliberate prefix-fold semantics (Lemma 5.1 / p. 29): verification
    // consults only the prefix up to the confirmable node — acceptance does
    // NOT certify trailing links. Pinned so a refactor back to whole-chain
    // folding (or a downstream "accepted ⇒ all links correct" assumption)
    // fails loudly here.
    let (server, client) = committed_run(6);
    let protocol::certificates::Certificate::Chained {
        entry,
        pos,
        mut chain,
    } = client.build_certificate(2).unwrap()
    else {
        panic!()
    };
    chain.push([0xab; 32]);
    chain.push([0xcd; 32]);
    let padded = protocol::certificates::Certificate::Chained { entry, pos, chain };
    assert!(server.verify(7, &padded));
}

#[test]
fn forgotten_commands_and_their_order_remain_provable_from_hashes_alone() {
    // §5's headline property: the compact rule forgets committed commands,
    // yet certificates prove WHAT committed and WHERE in the order, from
    // O(log m) root hashes + two chains per client — nothing else.
    let (server, client) = committed_run(8);

    // Forgotten: sn 1..=6 evicted from every window slot of client 7…
    let (prev, last) = server.last_two(7).unwrap();
    for sc in prev.iter().copied().chain([last]) {
        assert!(sc.sn >= 7, "window holds only the last two (sn {})", sc.sn);
    }
    // …and the server-side certificate state is only the forest peaks
    // (≤ log₂ m + 1 roots) plus those two chains — no command bytes.
    let m = server.forest().len();
    assert!(server.forest().roots().len() as u32 <= m.ilog2() + 1);

    // Every forgotten command still proves, and the verified positions
    // reproduce the exact commit order.
    let mut positions = Vec::new();
    for sn in 1..=6u64 {
        let cert = client.build_certificate(sn).unwrap();
        assert!(server.verify(7, &cert), "forgotten sn {sn} proves");
        let protocol::certificates::Certificate::Chained { pos, .. } = &cert else {
            panic!("evicted commands are chained certs")
        };
        positions.push(*pos);
    }
    assert!(
        positions.windows(2).all(|w| w[0] < w[1]),
        "commit order: {positions:?}"
    );

    // Order is cryptographically BOUND, not asserted: swapping two
    // certificates' positions makes both reject.
    let a = client.build_certificate(2).unwrap();
    let b = client.build_certificate(4).unwrap();
    let (
        protocol::certificates::Certificate::Chained {
            entry: ea,
            pos: pa,
            chain: ca,
        },
        protocol::certificates::Certificate::Chained {
            entry: eb,
            pos: pb,
            chain: cb,
        },
    ) = (a, b)
    else {
        panic!()
    };
    let a_at_b = protocol::certificates::Certificate::Chained {
        entry: ea,
        pos: pb,
        chain: ca,
    };
    let b_at_a = protocol::certificates::Certificate::Chained {
        entry: eb,
        pos: pa,
        chain: cb,
    };
    assert!(
        !server.verify(7, &a_at_b),
        "sn 2 at sn 4's position rejects"
    );
    assert!(
        !server.verify(7, &b_at_a),
        "sn 4 at sn 2's position rejects"
    );
}
