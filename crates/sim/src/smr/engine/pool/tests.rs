use super::*;
use rand::SeedableRng;

fn rng() -> ChaCha12Rng {
    ChaCha12Rng::seed_from_u64(7)
}

#[test]
fn sequence_numbers_increase_per_client_from_one() {
    let mut pool = ClientPool::new(2);
    let mut rng = rng();
    let first = pool.take(&mut rng, 1);
    let second = pool.take(&mut rng, 1);
    assert_eq!(first.1, 1);
    assert_eq!(second.1, 1, "each client starts at sn 1");
    pool.free(first.0);
    let reused = pool.take(&mut rng, 2);
    assert_eq!(reused.0, first.0, "only the freed client is drawable");
    assert_eq!(reused.1, 2, "the reused client's next sn");
}

#[test]
fn clients_live_in_the_reserved_auto_namespace() {
    let mut pool = ClientPool::new(1);
    assert_eq!(pool.take(&mut rng(), 1).0, AUTO_CLIENT_BASE);
}

#[test]
fn a_busy_client_is_never_drawn_again() {
    let mut pool = ClientPool::new(4);
    let mut rng = rng();
    let taken: Vec<u32> = (0..4).map(|_| pool.take(&mut rng, 1).0).collect();
    let mut sorted = taken.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), 4, "four draws, four distinct clients");
}

#[test]
#[should_panic(expected = "client pool exhausted")]
fn an_exhausted_pool_panics() {
    let mut pool = ClientPool::new(1);
    let mut rng = rng();
    pool.take(&mut rng, 1);
    pool.take(&mut rng, 2);
}

#[test]
fn the_idle_list_stays_sorted_whatever_the_free_order() {
    let mut pool = ClientPool::new(3);
    let mut rng = rng();
    let a = pool.take(&mut rng, 1).0;
    let b = pool.take(&mut rng, 1).0;
    let c = pool.take(&mut rng, 1).0;
    pool.free(c);
    pool.free(a);
    pool.free(b);
    assert_eq!(
        pool.idle,
        vec![0, 1, 2],
        "the draw indexes this list — its order must be the idle set, \
             not the order clients happened to be released in"
    );
}

#[test]
fn peak_in_flight_records_the_high_water_mark() {
    let mut pool = ClientPool::new(3);
    let mut rng = rng();
    let a = pool.take(&mut rng, 1).0;
    pool.take(&mut rng, 1);
    assert_eq!(pool.peak_in_flight(), 2);
    pool.free(a);
    assert_eq!(pool.peak_in_flight(), 2, "the peak never falls back");
}

#[test]
fn freeing_a_manual_client_is_ignored() {
    let mut pool = ClientPool::new(1);
    pool.free(17);
    assert_eq!(pool.idle, vec![0], "manual ids never enter the pool");
}

#[test]
#[should_panic(expected = "never issued")]
fn freeing_an_auto_client_the_pool_never_issued_panics() {
    ClientPool::new(1).free(AUTO_CLIENT_BASE + 9);
}

#[test]
#[should_panic(expected = "released twice")]
fn releasing_an_idle_client_panics() {
    let mut pool = ClientPool::new(2);
    let client = pool.take(&mut rng(), 1).0;
    pool.free(client);
    pool.free(client);
}
