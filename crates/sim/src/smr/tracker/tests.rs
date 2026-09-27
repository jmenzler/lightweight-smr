use super::super::AUTO_CLIENT_BASE;
use super::*;

fn inj(round: usize, client: u32, op: u64) -> Injection {
    Injection {
        round,
        client,
        op,
        target: None,
    }
}

fn reachable(injections: &[Injection]) -> bool {
    nulls_reachable(&trackers_for(injections).0)
}

#[test]
fn the_inert_prefix_stops_at_the_first_live_tracker() {
    let ts = trackers_for(&[inj(1, 1, 10), inj(1, 2, 20), inj(1, 3, 30), inj(1, 4, 40)]).0;
    // Inert by op: 10 and 20 lead, 30 is live, 40 is inert but behind it.
    let inert = |t: &CommandTracker| t.op != 30;
    assert_eq!(
        advance_inert_prefix(&ts, 0, inert),
        2,
        "a live tracker stops the advance"
    );
}

#[test]
fn the_inert_prefix_only_moves_forward() {
    let ts = trackers_for(&[inj(1, 1, 10), inj(1, 2, 20), inj(1, 3, 30)]).0;
    assert_eq!(advance_inert_prefix(&ts, 2, |_| false), 2);
    assert_eq!(advance_inert_prefix(&ts, 2, |_| true), 3);
    assert_eq!(
        advance_inert_prefix(&ts, 3, |_| true),
        3,
        "cannot pass the end"
    );
}

#[test]
fn distinct_clients_cannot_reach_a_null() {
    assert!(!reachable(&[inj(1, 1, 10), inj(1, 2, 20), inj(5, 3, 30)]));
}

#[test]
fn repeat_commands_from_one_client_cannot_reach_a_null() {
    assert!(!reachable(&[inj(1, 1, 10), inj(2, 1, 20), inj(3, 1, 30)]));
}

#[test]
fn a_manual_client_in_the_auto_namespace_can_reach_a_null() {
    // A later arrival takes this client with sn = 1 and a different op.
    assert!(reachable(&[inj(1, AUTO_CLIENT_BASE, 10)]));
}

#[test]
fn pool_clients_trip_the_namespace_check_conservatively() {
    assert!(reachable(&[
        inj(1, AUTO_CLIENT_BASE, 10),
        inj(9, AUTO_CLIENT_BASE, 20),
    ]));
}
