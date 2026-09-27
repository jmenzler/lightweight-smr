use sim::{BlockSchedule, Init, Scenario, run_traced};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out = args.get(1).map(String::as_str).unwrap_or("trace.json");
    let scenario = Scenario {
        cfg: sim::Config::default(),
        n: args.get(2).and_then(|s| s.parse().ok()).unwrap_or(80),
        seed: args.get(3).and_then(|s| s.parse().ok()).unwrap_or(3),
        init: Init::UniformRandom { k: 4 },
        max_rounds: 500,
        schedule: BlockSchedule::FreshPerRound {
            fraction: args.get(4).and_then(|s| s.parse().ok()).unwrap_or(0.1),
        },
        partition: None,
    };
    let (outcome, trace) = run_traced(&scenario);
    std::fs::write(out, serde_json::to_string(&trace).unwrap()).unwrap();
    eprintln!("{outcome:?} -> {out} ({} rounds)", trace.rounds.len());
}
