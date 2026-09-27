//! NodeRunSpec to a self-contained Shadow run dir.

use network_node::cli::arg;
use network_node::deploy::{peers_txt, shadow_yaml_with};
use network_node::spec::NodeRunSpec;

fn die(msg: &str) -> ! {
    network_node::cli::die("gen_shadow", msg)
}

fn main() {
    let spec_path = arg("--spec").unwrap_or_else(|| die("--spec required"));
    let out = arg("--out").unwrap_or_else(|| die("--out required"));
    let spec_json =
        std::fs::read_to_string(&spec_path).unwrap_or_else(|e| die(&format!("{spec_path}: {e}")));
    let spec = NodeRunSpec::from_json(&spec_json).unwrap_or_else(|e| die(&format!("spec: {e}")));

    let out = std::path::Path::new(&out);
    if out.exists()
        && out
            .read_dir()
            .map(|mut d| d.next().is_some())
            .unwrap_or(true)
    {
        die(&format!(
            "output dir {} exists and is not empty — run dirs are immutable evidence",
            out.display()
        ));
    }
    std::fs::create_dir_all(out).unwrap_or_else(|e| die(&format!("mkdir: {e}")));
    // Shadow runs each process with the per-host data dir as cwd, so paths must be absolute
    let bin_prefix = arg("--bin-dir").unwrap_or_else(|| "./target/release".into());
    let bin_prefix = std::fs::canonicalize(&bin_prefix)
        .unwrap_or_else(|e| die(&format!("--bin-dir {bin_prefix}: {e}")));
    let run_abs = std::fs::canonicalize(out).unwrap_or_else(|e| die(&format!("out: {e}")));
    let yaml = shadow_yaml_with(
        &spec,
        &format!("{}/", bin_prefix.display()),
        &format!("{}/", run_abs.display()),
    )
    .unwrap_or_else(|e| die(&e));
    std::fs::write(out.join("shadow.yaml"), yaml).unwrap_or_else(|e| die(&format!("write: {e}")));
    std::fs::write(out.join("peers.txt"), peers_txt(&spec))
        .unwrap_or_else(|e| die(&format!("write: {e}")));
    std::fs::write(out.join("spec.json"), &spec_json)
        .unwrap_or_else(|e| die(&format!("write: {e}")));
    println!("{}", out.display());
}
