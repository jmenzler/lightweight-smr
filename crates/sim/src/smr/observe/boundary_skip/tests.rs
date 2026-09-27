use super::BoundarySkipTally;

#[test]
fn a_run_without_skips_reports_nothing() {
    let mut tally = BoundarySkipTally::new(3);
    tally.observe([false, false, false], 1);
    tally.observe([false, false, false], 2);
    assert_eq!(tally.events(), 0);
    assert_eq!(tally.first_round(), None);
    assert_eq!(tally.last_round(), None);
    assert_eq!(tally.max_rejoin_rounds(), None);
    assert_eq!(tally.unrejoined(), 0);
}

#[test]
fn one_episode_spans_skip_to_adoption() {
    let mut tally = BoundarySkipTally::new(2);
    tally.observe([false, false], 77);
    tally.observe([true, false], 78);
    tally.observe([true, false], 79);
    tally.observe([false, false], 80);
    assert_eq!(tally.events(), 1);
    assert_eq!(tally.first_round(), Some(78));
    assert_eq!(tally.max_rejoin_rounds(), Some(2));
    assert_eq!(tally.unrejoined(), 0);
}

#[test]
fn a_node_still_stale_at_the_end_is_unrejoined_and_left_out_of_the_maximum() {
    let mut tally = BoundarySkipTally::new(2);
    tally.observe([true, true], 26);
    tally.observe([false, true], 27);
    tally.observe([false, true], 90);
    assert_eq!(tally.events(), 2);
    assert_eq!(tally.max_rejoin_rounds(), Some(1));
    assert_eq!(tally.unrejoined(), 1);
}

#[test]
fn a_second_skip_by_the_same_node_is_a_second_episode() {
    let mut tally = BoundarySkipTally::new(1);
    tally.observe([true], 26);
    tally.observe([false], 30);
    tally.observe([true], 52);
    tally.observe([false], 53);
    assert_eq!(tally.events(), 2);
    assert_eq!(tally.first_round(), Some(26));
    assert_eq!(tally.last_round(), Some(52));
    assert_eq!(tally.max_rejoin_rounds(), Some(4));
}
