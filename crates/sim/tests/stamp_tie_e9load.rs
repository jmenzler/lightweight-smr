//! Stamp-tie differential on recovery-under-load (E9-load n=4096).
//!
//! The policy is set on the spec's `merge_policy` from `STAMP_TIE_POLICY`
//! (`include-round`|`commands-only`) and `STAMP_TIE_UNION` (`first`|`last`).
//!
//!   SIM_RUNLOG=off cargo test -p sim --release --test stamp_tie_e9load \
//!     e9load_n4096_seed_range -- --ignored --nocapture

use protocol::merge::{StampTie, UnionStamp};
use sim::smr::{MergePolicy, SmrScenario, SmrTerminal, run_smr_grid};
use sim::spec::SmrScenarioSpec;

const FIXTURE: &str = include_str!("fixtures/e9load-n4096-seed3205022.json");
const DEFAULT_SEED_START: u64 = 3_205_000;
const DEFAULT_SEED_COUNT: u64 = 20;

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    payload
        .downcast_ref::<&'static str>()
        .copied()
        .map(str::to_owned)
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "<non-string panic>".into())
}

fn policy() -> (String, String) {
    (
        std::env::var("STAMP_TIE_POLICY").unwrap_or_else(|_| "include-round".into()),
        std::env::var("STAMP_TIE_UNION").unwrap_or_else(|_| "first".into()),
    )
}

fn merge_policy(stamp: &str, union: &str) -> MergePolicy {
    MergePolicy {
        stamp_tie: match stamp {
            "include-round" => StampTie::IncludeRound,
            "commands-only" => StampTie::CommandsOnly,
            s => panic!("STAMP_TIE_POLICY={s:?}; expected include-round|commands-only"),
        },
        union_stamp: match union {
            "first" => UnionStamp::FirstSighting,
            "last" => UnionStamp::LastSighting,
            s => panic!("STAMP_TIE_UNION={s:?}; expected first|last"),
        },
    }
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .map(|value| {
            value
                .parse()
                .unwrap_or_else(|_| panic!("{name}={value:?}; expected u64"))
        })
        .unwrap_or(default)
}

fn seed_range() -> std::ops::Range<u64> {
    let start = env_u64("STAMP_TIE_SEED_START", DEFAULT_SEED_START);
    let count = env_u64("STAMP_TIE_SEED_COUNT", DEFAULT_SEED_COUNT);
    assert!(count > 0, "STAMP_TIE_SEED_COUNT must be positive");
    start..start.checked_add(count).expect("seed range overflows u64")
}

fn emit(stamp: &str, union: &str, seed: u64, class: &str, detail: &str) {
    let detail = detail.replace(['\t', '\n'], " ");
    println!("STAMP_TIE\t{stamp}\t{union}\t{seed}\t{class}\t{detail}");
}

fn classify(scenario: &SmrScenario) -> (String, String) {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_smr_grid(scenario))) {
        Ok(report) => {
            let completed_horizon = matches!(
                report.terminal,
                SmrTerminal::Ran { rounds } if rounds == scenario.max_rounds
            );
            let class = if report.safety_ok && completed_horizon {
                "CLEAN"
            } else {
                "SOFT-FAIL"
            };
            (
                class.into(),
                format!(
                    "safety_ok={}\tterminal={:?}\trounds={}",
                    report.safety_ok,
                    report.terminal,
                    report.metrics.len()
                ),
            )
        }
        Err(payload) => {
            let msg = panic_message(payload);
            let class = if msg.contains("executed sequence regressed") {
                "ABORT-MONO"
            } else if msg.contains("P must be a prefix of L") {
                "ABORT-PREFIX"
            } else {
                "PANIC"
            };
            (class.into(), msg)
        }
    }
}

fn spec_at(seed: u64, policy: MergePolicy) -> SmrScenario {
    let mut spec: SmrScenarioSpec = serde_json::from_str(FIXTURE).expect("fixture parses");
    spec.seed = seed;
    spec.merge_policy = policy;
    spec.try_into().expect("fixture validates")
}

#[test]
#[ignore]
fn e9load_n4096_seed3205022() {
    let (stamp, union) = policy();
    let seed: u64 = std::env::var("STAMP_TIE_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3_205_022);
    let (class, detail) = classify(&spec_at(seed, merge_policy(&stamp, &union)));
    emit(&stamp, &union, seed, &class, &detail);
    assert_ne!(class, "PANIC", "{detail}");
}

#[test]
#[ignore]
fn e9load_n4096_seed_range() {
    let (stamp, union) = policy();
    for seed in seed_range() {
        let (class, detail) = classify(&spec_at(seed, merge_policy(&stamp, &union)));
        emit(&stamp, &union, seed, &class, &detail);
        assert_ne!(class, "PANIC", "seed {seed}: {detail}");
    }
}
