use protocol::{Config, MedianNode, Value, median};
use rand::SeedableRng;
use rand_chacha::ChaCha12Rng;
use rstest::rstest;

fn rng() -> ChaCha12Rng {
    ChaCha12Rng::seed_from_u64(42)
}

// --- Config preconditions (Algorithm 1 box: k, ℓ > 1, k ≥ ℓ, ℓ odd) ---

#[rstest]
#[case::k_too_small(1, 3)]
#[case::ell_too_small(6, 1)]
#[case::ell_zero(6, 0)]
#[case::ell_exceeds_k(3, 6)]
#[case::ell_even(6, 4)]
fn config_rejects_invalid_parameters(#[case] k: usize, #[case] ell: usize) {
    assert!(Config::new(k, ell).is_err());
}

#[rstest]
#[case(6, 3)]
#[case(3, 3)]
#[case(7, 5)]
fn config_accepts_valid_parameters(#[case] k: usize, #[case] ell: usize) {
    let cfg = Config::new(k, ell).unwrap();
    assert_eq!((cfg.k, cfg.ell), (k, ell));
}

#[test]
fn config_default_is_paper_instantiation() {
    let cfg = Config::default();
    assert_eq!((cfg.k, cfg.ell), (6, 3));
    assert!(Config::new(cfg.k, cfg.ell).is_ok());
}

// --- Algorithm 2: the (k,ℓ,f)-rule ---

#[rstest]
#[case::min(0)]
#[case::max(1)]
fn step_with_adopts_output_of_f(#[case] which: usize) {
    let fs: [fn(&[Value]) -> Value; 2] =
        [|s| *s.iter().min().unwrap(), |s| *s.iter().max().unwrap()];
    let expected = [1, 9];
    let mut node = MedianNode::new(7, Config::default());
    // exactly ℓ replies: the chosen subset is all of them
    node.step_with(&[3, 1, 9], &mut rng(), fs[which]);
    assert_eq!(node.x_i(), Some(expected[which]));
}

#[test]
fn step_with_passes_exactly_ell_values_to_f() {
    let mut node = MedianNode::new(7, Config::default());
    node.step_with(&[5, 6, 7, 8, 9, 10], &mut rng(), |s| {
        assert_eq!(s.len(), 3, "f must receive exactly ℓ values");
        s[0]
    });
}

#[test]
fn step_with_preserves_duplicates_as_multiset() {
    // paper box p.14: f(S) with S the MULTISET of values sent by the chosen replies
    let mut node = MedianNode::new(7, Config::default());
    node.step_with(&[4, 4, 4], &mut rng(), |s| {
        assert_eq!(s, [4, 4, 4], "duplicates must be preserved");
        s[0]
    });
    assert_eq!(node.x_i(), Some(4));
}

#[test]
fn step_with_goes_undecided_below_ell_regardless_of_f() {
    let mut node = MedianNode::new(7, Config::default());
    node.step_with(&[3, 9], &mut rng(), |_| {
        panic!("f must not be called below ℓ replies")
    });
    assert_eq!(node.x_i(), None);
}

#[test]
fn median_rule_is_the_f_median_instance() {
    let replies = [11, 5, 42, 7, 23, 5];
    for seed in 0..50u64 {
        let mut a = MedianNode::new(1, Config::default());
        let mut b = MedianNode::new(1, Config::default());
        a.step(&replies, &mut ChaCha12Rng::seed_from_u64(seed));
        b.step_with(&replies, &mut ChaCha12Rng::seed_from_u64(seed), median);
        assert_eq!(a.x_i(), b.x_i());
    }
}

#[test]
fn undecided_on_fewer_than_ell_replies() {
    let mut node = MedianNode::new(7, Config::default());
    node.step(&[3, 9], &mut rng());
    assert_eq!(node.x_i(), None);
}

#[test]
fn adopts_median_of_three_replies() {
    let mut node = MedianNode::new(7, Config::default());
    node.step(&[3, 1, 9], &mut rng());
    // exactly ell replies: the only 3-subset is all of them, median = 3
    assert_eq!(node.x_i(), Some(3));
}

#[test]
fn adopted_value_is_always_one_of_the_replies() {
    // validity: the median rule never invents a value
    let replies = [11, 5, 42, 7, 23, 5];
    for seed in 0..100u64 {
        let mut node = MedianNode::new(1, Config::default());
        let mut r = ChaCha12Rng::seed_from_u64(seed);
        node.step(&replies, &mut r);
        match node.x_i() {
            Some(v) => assert!(replies.contains(&v), "invented value {v}"),
            None => panic!("enough replies, must not go undecided"),
        }
    }
}

#[test]
fn new_undecided_node_starts_undecided_and_silent() {
    let node = MedianNode::new_undecided(Config::default());
    assert_eq!(node.x_i(), None);
    assert_eq!(node.answer(), None);
}

#[test]
fn undecided_node_does_not_reply() {
    let mut node = MedianNode::new(7, Config::default());
    node.step(&[], &mut rng()); // no replies -> undecided
    assert_eq!(node.answer(), None);
}

#[test]
fn deciding_node_replies_with_its_value() {
    let node = MedianNode::new(7, Config::default());
    assert_eq!(node.answer(), Some(7));
}

#[test]
fn undecided_node_recovers_on_enough_replies() {
    let mut node = MedianNode::new(7, Config::default());
    node.step(&[], &mut rng()); // -> undecided
    node.step(&[4, 4, 8], &mut rng()); // median of {4,4,8} = 4
    assert_eq!(node.x_i(), Some(4));
}
