//! The box anchors and choice tags in `src/` must match the tables in the crate README;
//! each file's anchors must appear in the README's order, which is the order of the box.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const KINDS: [(&str, &str); 3] = [
    ("DET", "DETERMINIZED"),
    ("EXT", "EXTENSION"),
    ("DEV", "DEVIATION"),
];

type Step = (String, String);
type Choice = (String, String, String);

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("src/ is readable") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn source_tags() -> (Vec<Step>, BTreeSet<Choice>) {
    let src = crate_dir().join("src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    let mut steps = Vec::new();
    let mut choices = BTreeSet::new();
    for path in files {
        let file = path
            .strip_prefix(&src)
            .expect("under src/")
            .to_string_lossy()
            .into_owned();
        for line in fs::read_to_string(&path)
            .expect("source is readable")
            .lines()
        {
            let line = line.trim_start();
            let Some(text) = line.strip_prefix("// ") else {
                continue;
            };
            if text.starts_with("Alg ") {
                steps.push((file.clone(), text.to_string()));
            }
            for (prefix, kind) in KINDS {
                if let Some(rest) = text.strip_prefix(kind).and_then(|r| r.strip_prefix(": [")) {
                    let id = rest.split(']').next().expect("closing bracket");
                    assert!(id.starts_with(prefix), "{file}: {kind} tag carries id {id}");
                    choices.insert((file.clone(), id.to_string(), kind.to_string()));
                }
            }
        }
    }
    (steps, choices)
}

fn cells(line: &str) -> Vec<&str> {
    line.split('|').map(str::trim).collect()
}

fn unquote(cell: &str) -> &str {
    cell.trim_matches('`')
}

fn readme_tags() -> (Vec<Step>, BTreeSet<Choice>) {
    let readme = fs::read_to_string(crate_dir().join("README.md")).expect("README.md");
    let mut steps = Vec::new();
    let mut choices = BTreeSet::new();
    for line in readme.lines() {
        let c = cells(line);
        if c.len() < 3 {
            continue;
        }
        if c[1].starts_with("`Alg ") {
            steps.push((unquote(c[2]).to_string(), unquote(c[1]).to_string()));
        }
        if KINDS
            .iter()
            .any(|(p, _)| c[1].starts_with(&format!("{p}-")))
        {
            for file in c[5].split(',') {
                choices.insert((
                    unquote(file.trim()).to_string(),
                    c[1].to_string(),
                    c[2].to_string(),
                ));
            }
        }
    }
    (steps, choices)
}

/// The anchors of each file, in order of appearance.
fn by_file(steps: &[Step]) -> BTreeMap<String, Vec<String>> {
    let mut files: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (file, anchor) in steps {
        files.entry(file.clone()).or_default().push(anchor.clone());
    }
    files
}

#[test]
fn step_tags() {
    let (source_steps, source_choices) = source_tags();
    let (readme_steps, readme_choices) = readme_tags();
    let unique: BTreeSet<Step> = source_steps.iter().cloned().collect();
    assert_eq!(
        unique.len(),
        source_steps.len(),
        "an anchor appears twice in one file"
    );
    assert_eq!(
        by_file(&source_steps),
        by_file(&readme_steps),
        "README box-to-code table vs anchors in src/, per file and in order"
    );
    assert_eq!(
        source_choices, readme_choices,
        "README choice table vs DETERMINIZED/EXTENSION/DEVIATION tags"
    );
}
