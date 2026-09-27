use sim::harness::{concurrent_injections, fresh_extended, injection};
use sim::smr::Proto;
use sim::{BlockSchedule, Config};

#[test]
fn builders_fill_the_shared_runner_shape() {
    let inj = injection(3, 1, 7);
    assert_eq!((inj.round, inj.client, inj.op, inj.target), (3, 1, 7, None));

    let batch = concurrent_injections(3, 8, 100);
    assert_eq!(batch.len(), 8);
    assert!(batch.iter().enumerate().all(|(i, x)| {
        x.round == 3 && x.client == 1 + i as u32 && x.op == 100 + i as u64 && x.target.is_none()
    }));

    let sc = fresh_extended(250, 42, 1.0, 0.1, vec![injection(3, 1, 7)], 150);
    assert_eq!(
        (sc.n, sc.seed, sc.sigma, sc.max_rounds),
        (250, 42, 1.0, 150)
    );
    assert_eq!(sc.cfg, Config::default());
    assert!(matches!(sc.proto, Proto::Extended));
    assert!(matches!(sc.schedule, BlockSchedule::FreshPerRound { fraction } if fraction == 0.1));
    assert!(sc.traffic.is_none());
}
