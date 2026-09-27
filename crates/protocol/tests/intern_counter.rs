//! The shared-mint state comparator, counted. A representation change can make
//! `content_eq` answer false where it used to answer true — the population then
//! keeps n private copies while every byte-identity gate stays green and the
//! memory rows merely go flat. The counter is what turns that into a reading.
//!
//! ONE test in this file on purpose: the counters are process-wide, and a
//! second `#[test]` in the same binary would race this one's deltas.

use protocol::Config;
use protocol::compact::{ClientCommand, Entry, SharedState, Timed};
use protocol::log::Log;
use protocol::recovery::{Checkpoint, RState, RecoveryNode, RecoveryReply, state_intern_counts};
use std::sync::Arc;

fn cmd(client: u32, op: u64) -> Entry {
    Entry::Cmd(ClientCommand { client, sn: 1, op })
}

fn genesis() -> Checkpoint {
    Checkpoint {
        s: SharedState::default().into(),
        p: None,
        w: 0,
        certs: None,
    }
}

/// Two nodes that committed the same entry independently, one boundary in.
fn committed_pair() -> (RecoveryNode, RecoveryNode) {
    let mk = || {
        let mut n = RecoveryNode::new(Config::default(), 1, false);
        let log = vec![Timed {
            entry: cmd(4, 4),
            round: 0,
        }];
        let replies: Vec<RecoveryReply> = (0..3)
            .map(|_| RecoveryReply {
                l_j: Some(Log::from_entries(log.clone())),
                c_j: genesis().into(),
                r_j: RState::NoReset,
            })
            .collect();
        n.step_chosen(&replies, &[], Some(&[0, 1, 2]));
        n
    };
    let (mut rep, mut member) = (mk(), mk());
    rep.end_window(1, 10);
    member.end_window_shared(1, 10, &rep.checkpoint_shared());
    (rep, member)
}

#[test]
fn the_shared_mint_counts_the_installs_it_attempts_and_the_ones_it_makes() {
    let (mut rep, mut member) = committed_pair();
    let (hit0, tried0) = state_intern_counts();
    assert_eq!(
        (hit0, tried0),
        (1, 1),
        "the pair's own first boundary is one attempt, and it installs"
    );

    // A hit: the member executed the same P onto a content-equal base.
    rep.end_window(2, 20);
    member.end_window_shared(2, 20, &rep.checkpoint_shared());
    assert!(
        Arc::ptr_eq(member.state_arc(), &rep.checkpoint().s),
        "the fixture must reach the install"
    );
    assert_eq!(
        state_intern_counts(),
        (hit0 + 1, tried0 + 1),
        "an install counts as both an attempt and a hit"
    );

    // A miss: the member's forget line ran ahead, so the comparator answers
    // false on an equal-length state with a different offset.
    member.forget_committed_prefix(1);
    rep.end_window(3, 30);
    member.end_window_shared(3, 30, &rep.checkpoint_shared());
    assert!(
        !Arc::ptr_eq(member.state_arc(), &rep.checkpoint().s),
        "the fixture must reach the miss"
    );
    assert_eq!(
        state_intern_counts(),
        (hit0 + 1, tried0 + 2),
        "a miss counts as an attempt and not as a hit"
    );
}
