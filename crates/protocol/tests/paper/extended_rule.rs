use protocol::{Command, Config, LogNode};
use rand::SeedableRng;
use rand_chacha::ChaCha12Rng;
use rstest::rstest;

/// Replies reach the rule as shared handles; the tests keep the plain
/// vectors so their assertions still read the content directly.
fn shared(logs: &[Vec<Command>]) -> Vec<std::sync::Arc<Vec<Command>>> {
    logs.iter().cloned().map(std::sync::Arc::new).collect()
}

fn rng() -> ChaCha12Rng {
    ChaCha12Rng::seed_from_u64(42)
}

fn node() -> LogNode {
    LogNode::new(0, Config::default())
}

#[test]
fn starts_with_seed_command_log() {
    assert_eq!(node().log(), Some(&[0][..]));
}

#[rstest]
#[case::six_three(6, 3, 2)]
#[case::seven_five(7, 5, 4)]
fn undecided_on_fewer_than_ell_replies(#[case] k: usize, #[case] ell: usize, #[case] got: usize) {
    let mut n = LogNode::new(0, Config::new(k, ell).unwrap());
    let replies = vec![vec![0, 1]; got];
    n.step(&shared(&replies), &[], &mut rng());
    assert_eq!(n.log(), None, "below ℓ replies the log must be ⊥");
}

#[test]
fn undecided_node_does_not_reply() {
    let mut n = node();
    n.step(&[], &[], &mut rng());
    assert_eq!(n.log(), None);
}

#[test]
fn adopts_lexicographic_median_log_and_appends_missing() {
    let mut n = node();
    // lex order: [0,1,2] < [0,1,3] < [0,2,1] → median [0,1,3]; missing: {2}
    let replies = vec![vec![0, 1, 2], vec![0, 1, 3], vec![0, 2, 1]];
    n.step(&shared(&replies), &[], &mut rng());
    assert_eq!(n.log(), Some(&[0, 1, 3, 2][..]));
}

#[test]
fn append_requests_join_the_log_tail() {
    let mut n = node();
    n.step(
        &[
            std::sync::Arc::new(vec![0]),
            std::sync::Arc::new(vec![0]),
            std::sync::Arc::new(vec![0]),
        ],
        &[5],
        &mut rng(),
    );
    assert_eq!(n.log(), Some(&[0, 5][..]));
}

#[test]
fn append_request_already_in_median_is_not_duplicated() {
    let mut n = node();
    n.step(
        &[
            std::sync::Arc::new(vec![0, 5]),
            std::sync::Arc::new(vec![0, 5]),
            std::sync::Arc::new(vec![0, 5]),
        ],
        &[5],
        &mut rng(),
    );
    assert_eq!(n.log(), Some(&[0, 5][..]));
}

#[test]
fn merged_log_never_repeats_a_command() {
    // K* = sequences without repetitions; L̄ must dedup across sources
    let mut n = node();
    let replies = vec![vec![0, 7], vec![0, 8], vec![0, 9]];
    n.step(&shared(&replies), &[7, 8, 9], &mut rng());
    let log = n.log().unwrap();
    let mut seen = std::collections::HashSet::new();
    assert!(
        log.iter().all(|c| seen.insert(*c)),
        "repeated command in {log:?}"
    );
}

#[test]
fn adopted_commands_all_come_from_replies_or_appends() {
    // validity: the rule never invents commands
    for seed in 0..50u64 {
        let mut n = node();
        let replies = vec![vec![0, 1], vec![0, 2], vec![0, 3], vec![0, 1, 4]];
        let appends = [9];
        let mut r = ChaCha12Rng::seed_from_u64(seed);
        n.step(&shared(&replies), &appends, &mut r);
        let known: std::collections::HashSet<Command> =
            replies.iter().flatten().copied().chain(appends).collect();
        for c in n.log().unwrap() {
            assert!(known.contains(c), "invented command {c}");
        }
    }
}

#[test]
fn median_log_stays_a_prefix_of_the_result() {
    // L_i := L'_i ∘ L̄ — the median log is never reordered, only extended
    for seed in 0..50u64 {
        let mut n = node();
        let replies = vec![vec![0, 1, 2], vec![0, 1, 3], vec![0, 2, 1]];
        let mut r = ChaCha12Rng::seed_from_u64(seed);
        n.step(&shared(&replies), &[], &mut r);
        assert_eq!(&n.log().unwrap()[..3], &[0, 1, 3]);
    }
}

#[rstest]
#[case::fresh_command(5, true)]
#[case::already_logged(0, false)]
fn wants_amplify_only_new_commands(#[case] x: Command, #[case] expected: bool) {
    assert_eq!(node().wants_amplify(x), expected);
}

#[test]
fn undecided_node_amplifies_unseen_commands() {
    let mut n = node();
    n.step(&[], &[], &mut rng());
    assert!(
        n.wants_amplify(5),
        "Algorithm 3 requires an unblocked ⊥ node to send append requests"
    );
}

#[test]
fn undecided_node_recovers_on_enough_replies() {
    let mut n = node();
    n.step(&[], &[], &mut rng());
    n.step(
        &[
            std::sync::Arc::new(vec![0, 4]),
            std::sync::Arc::new(vec![0, 4]),
            std::sync::Arc::new(vec![0, 4]),
        ],
        &[],
        &mut rng(),
    );
    assert_eq!(n.log(), Some(&[0, 4][..]));
}
