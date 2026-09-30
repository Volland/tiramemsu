//! A small Gherkin reader for the openCypher TCK feature files.
//!
//! Adapted from oxilite's `tests/tck.rs` (MIT OR Apache-2.0, same author as Tiramemsu):
//! scenarios and scenario outlines, docstrings and tables, examples expanded per row.
#![allow(dead_code)]

use std::path::Path;

// ----- Gherkin -----

#[derive(Debug, Clone)]
pub enum StepArg {
    None,
    Doc(String),
    Table(Vec<Vec<String>>),
}

#[derive(Debug, Clone)]
pub struct GStep {
    pub text: String,
    pub arg: StepArg,
}

#[derive(Debug, Clone)]
pub struct Scenario {
    pub feature: String,
    pub name: String,
    pub steps: Vec<GStep>,
}

fn parse_table_row(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    // Cells are separated by unescaped pipes.
    let mut cells = Vec::new();
    let mut cur = String::new();
    let mut chars = t.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'|') => {
                cur.push('|');
                chars.next();
            }
            '|' => {
                cells.push(cur.trim().to_string());
                cur.clear();
            }
            c => cur.push(c),
        }
    }
    cells.push(cur.trim().to_string());
    cells
}

pub fn parse_feature(root: &Path, path: &Path) -> Vec<Scenario> {
    let text = std::fs::read_to_string(path).unwrap();
    let feature = path
        .strip_prefix(root.join("features"))
        .unwrap_or(path)
        .display()
        .to_string();
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let mut background: Vec<GStep> = Vec::new();
    while i < lines.len() {
        let l = lines[i].trim();
        let (is_outline, name) = if let Some(n) = l.strip_prefix("Scenario Outline:") {
            (true, n.trim().to_string())
        } else if let Some(n) = l.strip_prefix("Scenario:") {
            (false, n.trim().to_string())
        } else if l.starts_with("Background:") {
            i += 1;
            background = parse_steps(&lines, &mut i);
            continue;
        } else {
            i += 1;
            continue;
        };
        i += 1;
        let mut steps = background.clone();
        steps.extend(parse_steps(&lines, &mut i));
        if !is_outline {
            out.push(Scenario {
                feature: feature.clone(),
                name,
                steps,
            });
            continue;
        }
        // Examples tables.
        while i < lines.len() {
            let l = lines[i].trim();
            if l.starts_with("Examples:") {
                i += 1;
                let mut rows = Vec::new();
                while i < lines.len() && lines[i].trim().starts_with('|') {
                    rows.push(parse_table_row(lines[i]));
                    i += 1;
                }
                if rows.is_empty() {
                    continue;
                }
                let header = rows.remove(0);
                for (k, row) in rows.iter().enumerate() {
                    let subst = |s: &str| {
                        let mut s = s.to_string();
                        for (h, v) in header.iter().zip(row) {
                            s = s.replace(&format!("<{h}>"), v);
                        }
                        s
                    };
                    let steps = steps
                        .iter()
                        .map(|st| GStep {
                            text: subst(&st.text),
                            arg: match &st.arg {
                                StepArg::None => StepArg::None,
                                StepArg::Doc(d) => StepArg::Doc(subst(d)),
                                StepArg::Table(t) => StepArg::Table(
                                    t.iter()
                                        .map(|r| r.iter().map(|c| subst(c)).collect())
                                        .collect(),
                                ),
                            },
                        })
                        .collect();
                    out.push(Scenario {
                        feature: feature.clone(),
                        name: format!("{name} #{}", k + 1),
                        steps,
                    });
                }
            } else if l.starts_with("Scenario") {
                break;
            } else {
                i += 1;
            }
        }
    }
    out
}

fn parse_steps(lines: &[&str], i: &mut usize) -> Vec<GStep> {
    let mut steps = Vec::new();
    while *i < lines.len() {
        let l = lines[*i].trim();
        let kw = ["Given ", "When ", "Then ", "And ", "But "]
            .iter()
            .find(|k| l.starts_with(**k));
        let Some(kw) = kw else {
            if l.starts_with("Scenario") || l.starts_with("Examples:") || l.starts_with('@') {
                break;
            }
            *i += 1;
            continue;
        };
        let text = l[kw.len()..].trim_end_matches(':').trim().to_string();
        *i += 1;
        let mut arg = StepArg::None;
        if *i < lines.len() && lines[*i].trim().starts_with("\"\"\"") {
            let indent = lines[*i].len() - lines[*i].trim_start().len();
            *i += 1;
            let mut doc = Vec::new();
            while *i < lines.len() && !lines[*i].trim().starts_with("\"\"\"") {
                let line = lines[*i];
                doc.push(if line.len() >= indent {
                    &line[indent..]
                } else {
                    line.trim_start()
                });
                *i += 1;
            }
            *i += 1;
            arg = StepArg::Doc(doc.join("\n"));
        } else if *i < lines.len() && lines[*i].trim().starts_with('|') {
            let mut rows = Vec::new();
            while *i < lines.len() && lines[*i].trim().starts_with('|') {
                rows.push(parse_table_row(lines[*i]));
                *i += 1;
            }
            arg = StepArg::Table(rows);
        }
        steps.push(GStep { text, arg });
    }
    steps
}
