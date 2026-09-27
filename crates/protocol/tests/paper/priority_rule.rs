use protocol::{Config, MedianNode, PriorityNode, largest};
use rand::SeedableRng;
use rand_chacha::ChaCha12Rng;
use rstest::rstest;

fn rng() -> ChaCha12Rng {
    ChaCha12Rng::seed_from_u64(42)
}

#[test]
fn largest_picks_the_maximum() {
    assert_eq!(largest(&[3, 9, 1]), 9);
}

#[test]
fn adopts_largest_of_the_ell_chosen_replies() {
    let mut n = PriorityNode::new(5, Config::default());
    // exactly ℓ replies: the chosen subset is all of them
    n.step(&[3, 9, 1], &mut rng());
    assert_eq!(n.x_i(), Some(9));
}

#[rstest]
#[case::six_three(6, 3, 2)]
#[case::seven_five(7, 5, 4)]
fn undecided_on_fewer_than_ell_replies(#[case] k: usize, #[case] ell: usize, #[case] got: usize) {
    let mut n = PriorityNode::new(5, Config::new(k, ell).unwrap());
    let replies = vec![9; got];
    n.step(&replies, &mut rng());
    assert_eq!(n.x_i(), None);
}

#[test]
fn adopted_value_is_always_one_of_the_replies() {
    let replies = [11, 5, 42, 7, 23, 5];
    for seed in 0..50u64 {
        let mut n = PriorityNode::new(1, Config::default());
        n.step(&replies, &mut ChaCha12Rng::seed_from_u64(seed));
        match n.x_i() {
            Some(v) => assert!(replies.contains(&v), "invented value {v}"),
            None => panic!("enough replies, must not go undecided"),
        }
    }
}

#[test]
fn priority_rule_is_the_f_largest_instance() {
    // Definition 3.5 = the (k,ℓ,f)-rule with f = largest
    let replies = [11, 5, 42, 7, 23, 5];
    for seed in 0..50u64 {
        let mut a = PriorityNode::new(1, Config::default());
        let mut b = MedianNode::new(1, Config::default());
        a.step(&replies, &mut ChaCha12Rng::seed_from_u64(seed));
        b.step_with(&replies, &mut ChaCha12Rng::seed_from_u64(seed), largest);
        assert_eq!(a.x_i(), b.x_i());
    }
}

#[test]
fn undecided_node_is_silent_and_recovers() {
    let mut n = PriorityNode::new(5, Config::default());
    n.step(&[], &mut rng());
    assert_eq!(n.answer(), None);
    n.step(&[4, 4, 8], &mut rng());
    assert_eq!(n.x_i(), Some(8));
}
