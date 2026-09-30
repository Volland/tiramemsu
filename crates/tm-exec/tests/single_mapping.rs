//! Enforces the single view→SQL mapping: the time columns appear in string
//! literals only in `scan.rs` and `virtual_pred.rs`.

use std::fs;
use std::path::Path;

fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for e in fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// The string literals of a source file (test modules and comments excluded).
fn literals(src: &str) -> Vec<String> {
    let src = src.split("#[cfg(test)]").next().unwrap_or(src);
    let mut out = Vec::new();
    for line in src.lines() {
        let t = line.trim_start();
        if t.starts_with("//") {
            continue;
        }
        let mut rest = line;
        while let Some(i) = rest.find('"') {
            let after = &rest[i + 1..];
            let Some(j) = after.find('"') else { break };
            out.push(after[..j].to_string());
            rest = &after[j + 1..];
        }
    }
    out
}

#[test]
fn time_columns_only_in_scan_and_virtual_pred() {
    let mut files = Vec::new();
    walk(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    assert!(files.len() > 10);
    let mut bad = Vec::new();
    for f in files {
        let name = f.file_name().unwrap().to_string_lossy().to_string();
        if name == "scan.rs" || name == "virtual_pred.rs" {
            continue;
        }
        for l in literals(&fs::read_to_string(&f).unwrap()) {
            for col in ["t_ret", "t_add", "v_from", "v_to"] {
                if l.contains(col) {
                    bad.push(format!("{}: {l}", f.display()));
                }
            }
        }
    }
    assert!(bad.is_empty(), "time predicates outside scan.rs: {bad:#?}");
}
