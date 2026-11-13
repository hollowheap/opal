use crate::finding_fingerprint;
use anyhow::{Context, Result};
use regex::Regex;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

const SOURCE_RULES: &[(&str, &str, &str, &str)] = &[
    (
        "raw-color",
        "P2",
        "raw color literal",
        r"#[0-9a-fA-F]{3,8}\b|\brgba?\(|\bhsla?\(",
    ),
    (
        "outline-none",
        "P2",
        "focus outline suppressed",
        r"outline:\s*none|outline-none",
    ),
    (
        "transition-all",
        "P3",
        "transition on every property",
        r"transition-all|transition:\s*all",
    ),
    (
        "will-change",
        "P3",
        "manual compositor hint",
        r"will-change",
    ),
    (
        "gradient-text",
        "P3",
        "gradient text",
        r"bg-clip-text|text-transparent",
    ),
    (
        "glass",
        "P3",
        "decorative blur",
        r"backdrop-blur|backdrop-filter",
    ),
    ("viewport-width", "P3", "100vw sizing", r"100vw"),
    (
        "arbitrary-px",
        "P3",
        "arbitrary pixel value",
        r"\[[0-9]+px\]",
    ),
    (
        "clickable-div",
        "P2",
        "click handler on a non-control",
        r"<(div|span)[^>]*(onClick|@click)",
    ),
    (
        "motion",
        "P3",
        "motion declaration",
        r"@keyframes|animation:|animate-",
    ),
];

fn parse_ignore_ids(raw: &str) -> HashSet<String> {
    let cleaned = raw.trim_start_matches(|c: char| c == ':' || c.is_whitespace());
    cleaned
        .split(|c: char| c == ',' || c.is_whitespace())
        .map(|value| {
            value
                .trim_matches(|c: char| c == '*' || c == '/' || c == '>' || c == '-')
                .to_ascii_lowercase()
        })
        .filter(|value| {
            !value.is_empty()
                && !value.starts_with("--")
                && value
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '*')
        })
        .collect()
}

fn source_suppressions(content: &str) -> (HashMap<usize, HashSet<String>>, HashSet<String>) {
    let mut lines: HashMap<usize, HashSet<String>> = HashMap::new();
    let mut file = HashSet::new();
    for (index, line) in content.lines().enumerate() {
        let lower = line.to_ascii_lowercase();
        let Some(position) = lower.find("opal-ignore") else {
            continue;
        };
        let rest = &line[position + "opal-ignore".len()..];
        let rest_lower = rest.to_ascii_lowercase();
        let (scope, rest) = if rest_lower.starts_with("-file") {
            ("file", &rest["-file".len()..])
        } else if rest_lower.starts_with("-next-line") {
            ("next", &rest["-next-line".len()..])
        } else {
            ("line", rest)
        };
        let ids = parse_ignore_ids(rest);
        if ids.is_empty() {
            continue;
        }
        match scope {
            "file" => file.extend(ids),
            "next" => lines.entry(index + 2).or_default().extend(ids),
            _ => lines.entry(index + 1).or_default().extend(ids),
        }
    }
    (lines, file)
}

fn source_files(root: &Path, all: bool) -> Vec<PathBuf> {
    if all {
        let mut files = Vec::new();
        walk(root, &mut files, 0);
        return files;
    }
    if let Some(output) = git_output(
        root,
        &[
            "diff",
            "--name-only",
            "--diff-filter=ACMRTUXB",
            "HEAD",
            "--",
            ".",
        ],
    ) {
        let mut files: Vec<PathBuf> = output
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| root.join(line))
            .collect();
        if let Some(untracked) = git_output(root, &["ls-files", "--others", "--exclude-standard"]) {
            files.extend(
                untracked
                    .lines()
                    .filter(|line| !line.trim().is_empty())
                    .map(|line| root.join(line)),
            );
        }
        files.sort();
        files.dedup();
        return files;
    }

    let mut files = Vec::new();
    walk(root, &mut files, 0);
    files
}

fn git_output(root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 8 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            if matches!(
                name.as_str(),
                "node_modules" | ".git" | "dist" | "build" | "target" | ".next" | "vendor"
            ) {
                continue;
            }
            walk(&path, out, depth + 1);
        } else if path.is_file() {
            out.push(path);
        }
    }
}

fn is_ui_file(path: &Path) -> bool {
    let text = path.to_string_lossy();
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    matches!(
        extension,
        "tsx" | "jsx" | "vue" | "svelte" | "astro" | "html" | "css" | "scss" | "sass" | "less"
    ) || (matches!(extension, "ts" | "js")
        && [
            "/components/",
            "/pages/",
            "/views/",
            "/ui/",
            "/styles/",
            "/app/",
        ]
        .iter()
        .any(|dir| text.contains(dir)))
}

pub(crate) struct SourceScan {
    pub findings: Vec<Value>,
    pub suppressed: usize,
    pub files: usize,
}

pub(crate) fn scan(root: &Path, all: bool) -> Result<SourceScan> {
    let rules: Vec<(&str, &str, &str, Regex)> = SOURCE_RULES
        .iter()
        .map(|(id, severity, message, pattern)| {
            Ok((
                *id,
                *severity,
                *message,
                Regex::new(pattern).context("compile source rule")?,
            ))
        })
        .collect::<Result<_>>()?;
    let mut findings = Vec::new();
    let mut suppressed = 0usize;
    let mut scanned = 0usize;
    for path in source_files(root, all)
        .iter()
        .filter(|path| is_ui_file(path))
    {
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        scanned += 1;
        let (line_ignores, file_ignores) = source_suppressions(&content);
        let relative = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        let token_file = name.starts_with("tokens.")
            || name.starts_with("theme.")
            || name.contains(".tokens.")
            || relative.starts_with("tokens/")
            || relative.contains("/tokens/");
        for (line_number, line) in content.lines().enumerate() {
            for (id, severity, message, regex) in &rules {
                if token_file && *id == "raw-color" {
                    continue;
                }
                if !regex.is_match(line) {
                    continue;
                }
                let ignored = file_ignores.contains(*id)
                    || file_ignores.contains("*")
                    || line_ignores
                        .get(&(line_number + 1))
                        .map(|set| set.contains(*id) || set.contains("*"))
                        .unwrap_or(false);
                if ignored {
                    suppressed += 1;
                    continue;
                }
                let mut finding = json!({
                    "check": id,
                    "layer": "source",
                    "severity": severity,
                    "file": relative,
                    "line": line_number + 1,
                    "message": message,
                    "evidence": {
                        "text": line.trim().chars().take(160).collect::<String>()
                    }
                });
                finding["fingerprint"] = json!(finding_fingerprint(&finding));
                findings.push(finding);
            }
        }
    }
    findings.sort_by_key(
        |finding| match finding["severity"].as_str().unwrap_or("P3") {
            "P0" => 0,
            "P1" => 1,
            "P2" => 2,
            _ => 3,
        },
    );
    Ok(SourceScan {
        findings,
        suppressed,
        files: scanned,
    })
}
