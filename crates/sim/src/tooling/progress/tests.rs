use super::*;

fn read(path: &std::path::Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(path).expect("progress file exists"))
        .expect("valid json")
}

fn tmp(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("sim-progress-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_file(&p);
    p
}

#[test]
fn a_tick_writes_the_file_and_a_reread_shows_the_round() {
    let path = tmp("tick");
    let f = ProgressFile::with_interval(path.clone(), Duration::ZERO);
    f.tick("n10-s1", 137, 250);
    let v = read(&path);
    assert_eq!(v["cells"]["n10-s1"]["round"], 137);
    assert_eq!(v["cells"]["n10-s1"]["max_rounds"], 250);
    assert_eq!(v["cells"]["n10-s1"]["done"], false);
}

#[test]
fn ticks_inside_the_interval_do_not_rewrite() {
    let path = tmp("throttle");
    let f = ProgressFile::with_interval(path.clone(), Duration::from_secs(3600));
    f.tick("c", 1, 10);
    f.tick("c", 2, 10);
    assert_eq!(read(&path)["cells"]["c"]["round"], 1);
}

#[test]
fn done_forces_the_final_state_out_past_the_throttle() {
    let path = tmp("done");
    let f = ProgressFile::with_interval(path.clone(), Duration::from_secs(3600));
    f.tick("c", 1, 10);
    f.tick("c", 7, 10);
    f.done("c");
    let v = read(&path);
    assert_eq!(v["cells"]["c"]["round"], 7);
    assert_eq!(v["cells"]["c"]["done"], true);
}

#[test]
fn cells_report_independently() {
    let path = tmp("cells");
    let f = ProgressFile::with_interval(path.clone(), Duration::ZERO);
    f.tick("a", 3, 10);
    f.tick("b", 9, 20);
    let v = read(&path);
    assert_eq!(v["cells"]["a"]["round"], 3);
    assert_eq!(v["cells"]["b"]["round"], 9);
}
