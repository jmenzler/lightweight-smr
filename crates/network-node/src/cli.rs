//! Argv lookup and fatal-exit helpers shared by the bins.

/// Value following `name` in argv (`--flag value` form only).
pub fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

/// Print `{program}: {msg}` to stderr and exit(2).
pub fn die(program: &str, msg: &str) -> ! {
    eprintln!("{program}: {msg}");
    std::process::exit(2);
}
