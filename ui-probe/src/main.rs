use anyhow::{Context, Result};
use playwright_rs::{Browser, ColorScheme, EmulateMediaOptions, Page, Viewport};
use regex::Regex;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

mod capture;
mod design;
mod output;
mod rules;
mod source;

use capture::{
    activate_js, activated_js, discover_chrome, evaluate, evaluate_js, launch_browser, load,
    oracle_js, validate_js, BrowserSession, DISCOVER_JS, FOCUS_JS, STYLES_JS,
};
use design::{design_checks, infer_design, load_design_sources};
pub(crate) use design::{
    font_families, generic_family, parse_color_rgb, parse_dimension, radius_allowed, DesignSpec,
};
use output::{emit_report, OutputFormat};
use source::SourceScan;

use rules::{
    apply_ruleset, apply_unverified_rules, check_keys, expand_rules_sources, load_default_rules,
    load_rules, load_rules_with_default, rule_findings, RuleSet,
};

struct Args {
    url: Option<String>,
    source: Option<PathBuf>,
    wait_for: Option<String>,
    width: u32,
    height: u32,
    viewports: Vec<(u32, u32)>,
    timeout: Duration,
    launch_timeout: Duration,
    state: String,
    theme: Option<String>,
    pretty: bool,
    chrome: Option<PathBuf>,
    no_sandbox: bool,
    expand: bool,
    max_states: usize,
    design: Option<String>,
    sample_states: bool,
    allow: Vec<String>,
    source_all: bool,
    infer_missing: bool,
    force: bool,
    rules: Option<PathBuf>,
    baseline: Option<PathBuf>,
    write_baseline: bool,
    update_baseline: bool,
    no_baseline: bool,
    freeze_animations: bool,
    format: OutputFormat,
}

fn main() {
    match run() {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("ui-probe: {error:#}");
            std::process::exit(1);
        }
    }
}

fn parse_args(argv: Vec<String>) -> Result<Args> {
    let mut url = None;
    let mut source = None;
    let mut wait_for = None;
    let mut width = 1280;
    let mut height = 800;
    let mut viewports = vec![(1280u32, 800u32)];
    let mut timeout = Duration::from_millis(15000);
    let mut launch_timeout = Duration::from_millis(15000);
    let mut state = String::from("default");
    let mut theme = None;
    let mut pretty = false;
    let mut no_sandbox = false;
    let mut expand = false;
    let mut max_states = 8usize;
    let mut design = None;
    let mut sample_states = false;
    let mut allow: Vec<String> = Vec::new();
    let mut source_all = false;
    let mut infer_missing = false;
    let mut force = false;
    let mut rules = None;
    let mut baseline = None;
    let mut write_baseline = false;
    let mut update_baseline = false;
    let mut no_baseline = false;
    let mut freeze_animations = false;
    let mut format = OutputFormat::Json;
    let mut chrome = std::env::var("UI_PROBE_CHROME")
        .or_else(|_| std::env::var("CHROME"))
        .ok()
        .map(PathBuf::from);

    let mut argv = argv.into_iter();
    while let Some(arg) = argv.next() {
        match arg.as_str() {
            "--" => {}
            "--url" => url = Some(argv.next().context("--url needs a value")?),
            "--source" => {
                source = Some(PathBuf::from(argv.next().context("--source needs a path")?))
            }
            "--wait-for" => wait_for = Some(argv.next().context("--wait-for needs a value")?),
            "--state" => state = argv.next().context("--state needs a value")?,
            "--theme" => theme = Some(argv.next().context("--theme needs light or dark")?),
            "--viewport" => {
                let raw = argv.next().context("--viewport needs WxH or sweep")?;
                let list: Vec<(u32, u32)> = if raw == "sweep" {
                    vec![(375, 667), (768, 1024), (1280, 800)]
                } else {
                    raw.split(',')
                        .map(|entry| {
                            let (w, h) = entry
                                .trim()
                                .split_once('x')
                                .context("--viewport must be WxH")?;
                            Ok((
                                w.parse().context("bad viewport width")?,
                                h.parse().context("bad viewport height")?,
                            ))
                        })
                        .collect::<Result<Vec<_>>>()?
                };
                width = list[0].0;
                height = list[0].1;
                viewports = list;
            }
            "--timeout" => {
                let raw = argv.next().context("--timeout needs milliseconds")?;
                timeout = Duration::from_millis(raw.parse().context("bad timeout")?);
            }
            "--launch-timeout" => {
                let raw = argv.next().context("--launch-timeout needs milliseconds")?;
                launch_timeout = Duration::from_millis(raw.parse().context("bad launch timeout")?);
            }
            "--chrome" => {
                chrome = Some(PathBuf::from(argv.next().context("--chrome needs a path")?))
            }
            "--no-sandbox" => no_sandbox = true,
            "--expand" => expand = true,
            "--design" => design = Some(argv.next().context("--design needs a path or auto")?),
            "--sample-states" => sample_states = true,
            "--source-all" => source_all = true,
            "--infer-missing" => infer_missing = true,
            "--force" => force = true,
            "--rules" => rules = Some(PathBuf::from(argv.next().context("--rules needs a path")?)),
            "--baseline" => {
                baseline = Some(PathBuf::from(
                    argv.next().context("--baseline needs a path")?,
                ))
            }
            "--write-baseline" => write_baseline = true,
            "--update-baseline" => update_baseline = true,
            "--no-baseline" => no_baseline = true,
            "--freeze-animations" => freeze_animations = true,
            "--format" => {
                let value = argv.next().context("--format needs a value")?;
                format = match value.as_str() {
                    "json" => OutputFormat::Json,
                    "sarif" => OutputFormat::Sarif,
                    "markdown" => OutputFormat::Markdown,
                    "github" => OutputFormat::Github,
                    other => anyhow::bail!("unknown format: {other}"),
                };
            }
            "--allow" => {
                allow = argv
                    .next()
                    .context("--allow needs check ids")?
                    .split(',')
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
                    .collect()
            }
            "--max-states" => {
                max_states = argv
                    .next()
                    .context("--max-states needs a count")?
                    .parse()
                    .context("bad max-states")?
            }
            "--pretty" => pretty = true,
            "--help" | "-h" => {
                println!(
                    "usage: ui-probe init [DIR]\n       ui-probe audit URL [options]\n       ui-probe source [DIR] [options]\n       ui-probe pack [--rules PATH] [--json]\n       ui-probe doctor [--json]\n       ui-probe --url URL [options]\n       ui-probe --source DIR [--pretty]\n\noptions: --wait-for SELECTOR --viewport WxH --timeout MS --launch-timeout MS\n         --state NAME --theme light|dark --viewport WxH|sweep --chrome PATH\n         --no-sandbox --expand --max-states N --design PATH|auto --infer-missing\n         --force --sample-states --allow id,id --source-all --rules PATH\n         --baseline PATH --write-baseline --update-baseline --no-baseline\n         --freeze-animations --format json|sarif|markdown|github\n\nrules resolve from --rules, the nearest .opal/rules.json, then ~/.opal/rules.json;\nnamed rules overlay by id and structural rules append.\ndesign layers --design, the nearest .opal/design.json, then ~/.opal/design.json."
                );
                std::process::exit(0);
            }
            other => anyhow::bail!("unknown argument: {other}"),
        }
    }

    if url.is_none() && source.is_none() {
        anyhow::bail!("--url or --source is required");
    }

    Ok(Args {
        url,
        source,
        wait_for,
        width,
        height,
        viewports,
        timeout,
        launch_timeout,
        state,
        theme,
        pretty,
        chrome,
        no_sandbox,
        expand,
        max_states,
        design,
        sample_states,
        allow,
        source_all,
        infer_missing,
        force,
        rules,
        baseline,
        write_baseline,
        update_baseline,
        no_baseline,
        freeze_animations,
        format,
    })
}

fn run() -> Result<i32> {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let command = raw.first().cloned();
    match command.as_deref() {
        Some("init") => init_mode(&raw[1..]),
        Some("pack") => pack_mode(&raw[1..]),
        Some("doctor") => doctor_mode(&raw[1..]),
        Some("audit") => audit_mode(parse_args(audit_arguments(&raw[1..]))?),
        Some("source") => audit_mode(parse_args(source_arguments(&raw[1..]))?),
        _ => audit_mode(parse_args(raw)?),
    }
}

fn audit_arguments(raw: &[String]) -> Vec<String> {
    let mut rest = raw.to_vec();
    let target = take_target(&mut rest);
    if rest.first().map(String::as_str) == Some("--") {
        rest.remove(0);
    }
    match target {
        Some(url) => {
            let mut out = vec!["--url".to_string(), url];
            out.extend(rest);
            out
        }
        None => rest,
    }
}

fn source_arguments(raw: &[String]) -> Vec<String> {
    let mut rest = raw.to_vec();
    let target = take_target(&mut rest);
    if rest.first().map(String::as_str) == Some("--") {
        rest.remove(0);
    }
    let mut out = vec![
        "--source".to_string(),
        target.unwrap_or_else(|| ".".to_string()),
    ];
    out.extend(rest);
    out
}

fn take_target(rest: &mut Vec<String>) -> Option<String> {
    if rest.first().map(String::as_str) == Some("--") {
        rest.remove(0);
    }
    match rest.first() {
        Some(first) if !first.starts_with('-') => Some(rest.remove(0)),
        _ => None,
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn project_config_dir(start: &Path) -> Option<PathBuf> {
    let mut current = Some(start);
    while let Some(dir) = current {
        let config = dir.join(".opal");
        if config.is_dir() {
            return Some(config);
        }
        if dir.join(".git").exists() {
            break;
        }
        current = dir.parent();
    }
    None
}

fn resolve_design_sources(explicit: Option<&str>) -> Vec<(PathBuf, &'static str)> {
    let mut sources = Vec::new();
    match explicit {
        Some("auto") => return sources,
        Some(path) => sources.push((PathBuf::from(path), "explicit")),
        None => {}
    }
    if let Ok(cwd) = std::env::current_dir() {
        if let Some(config) = project_config_dir(&cwd) {
            push_source(&mut sources, config.join("design.json"), "project");
        }
    }
    if let Some(home) = home_dir() {
        push_source(&mut sources, home.join(".opal").join("design.json"), "user");
    }
    sources
}

fn resolve_rules_sources(explicit: Option<&Path>) -> Vec<(PathBuf, &'static str)> {
    let mut sources = Vec::new();
    if let Some(path) = explicit {
        sources.push((path.to_path_buf(), "explicit"));
    }
    if let Ok(cwd) = std::env::current_dir() {
        if let Some(config) = project_config_dir(&cwd) {
            push_source(&mut sources, config.join("rules.json"), "project");
        }
    }
    if let Some(home) = home_dir() {
        push_source(&mut sources, home.join(".opal").join("rules.json"), "user");
    }
    sources
}

fn push_source(sources: &mut Vec<(PathBuf, &'static str)>, path: PathBuf, origin: &'static str) {
    if !path.is_file() {
        return;
    }
    let canonical = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
    let duplicate = sources.iter().any(|(existing, _)| {
        std::fs::canonicalize(existing)
            .map(|entry| entry == canonical)
            .unwrap_or(false)
    });
    if !duplicate {
        sources.push((path, origin));
    }
}

struct Baseline {
    path: PathBuf,
    fingerprints: HashSet<String>,
}

fn resolve_baseline_path(
    explicit: Option<&Path>,
    no_baseline: bool,
    write: bool,
    update: bool,
) -> Option<PathBuf> {
    if no_baseline {
        return None;
    }
    if let Some(path) = explicit {
        return Some(path.to_path_buf());
    }
    let cwd = std::env::current_dir().ok()?;
    if let Some(config) = project_config_dir(&cwd) {
        let path = config.join("baseline.json");
        if path.is_file() || write || update {
            return Some(path);
        }
    }
    if write || update {
        return Some(cwd.join(".opal").join("baseline.json"));
    }
    None
}

fn load_baseline(path: Option<&Path>, allow_missing: bool) -> Result<Option<Baseline>> {
    let Some(path) = path else {
        return Ok(None);
    };
    if !path.is_file() {
        if allow_missing {
            return Ok(None);
        }
        anyhow::bail!("baseline file not found: {}", path.display());
    }
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("read baseline file {}", path.display()))?;
    let data: Value = serde_json::from_str(&text)
        .with_context(|| format!("parse baseline file {}", path.display()))?;
    check_keys(&data, &["version", "fingerprints"], "baseline file")?;
    if data["version"].as_u64() != Some(1) {
        anyhow::bail!("baseline: version must be 1");
    }
    let fingerprints = data["fingerprints"]
        .as_array()
        .context("baseline: fingerprints must be an array")?
        .iter()
        .filter_map(|value| value.as_str().map(str::to_string))
        .collect();
    Ok(Some(Baseline {
        path: path.to_path_buf(),
        fingerprints,
    }))
}

fn write_baseline(path: &Path, fingerprints: &HashSet<String>) -> Result<()> {
    let mut list: Vec<&String> = fingerprints.iter().collect();
    list.sort();
    let document = json!({
        "version": 1,
        "fingerprints": list
    });
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    std::fs::write(path, serde_json::to_string_pretty(&document)? + "\n")
        .with_context(|| format!("write baseline file {}", path.display()))?;
    Ok(())
}

fn hash_text(text: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn finding_fingerprint(finding: &Value) -> String {
    let check = finding["check"].as_str().unwrap_or("");
    let mut parts = Vec::new();
    for key in [
        "check", "layer", "selector", "property", "message", "file", "line",
    ] {
        if let Some(value) = finding.get(key) {
            match value {
                Value::String(text) => parts.push(text.clone()),
                Value::Number(number) => parts.push(number.to_string()),
                Value::Null => {}
                other => parts.push(other.to_string()),
            }
        }
    }
    if matches!(
        check,
        "console-error" | "console-warning" | "page-error" | "network-error"
    ) {
        parts.push(finding["evidence"].to_string());
    }
    hash_text(&parts.join("\u{1f}"))
}

fn baseline_filter(report: &mut Value, baseline: &Baseline) -> usize {
    let findings = report["findings"].as_array().cloned().unwrap_or_default();
    let mut suppressed = 0;
    let filtered: Vec<Value> = findings
        .into_iter()
        .filter(|finding| {
            let fingerprint = finding["fingerprint"].as_str().unwrap_or("");
            if baseline.fingerprints.contains(fingerprint) {
                suppressed += 1;
                false
            } else {
                true
            }
        })
        .collect();
    report["findings"] = json!(filtered);
    suppressed
}

fn refresh_summary(report: &mut Value) {
    let findings = report["findings"].as_array().cloned().unwrap_or_default();
    let mut by_check: serde_json::Map<String, Value> = serde_json::Map::new();
    for finding in &findings {
        let name = finding["check"].as_str().unwrap_or("unknown").to_string();
        let count = by_check.get(&name).and_then(Value::as_u64).unwrap_or(0);
        by_check.insert(name, json!(count + 1));
    }
    report["summary"]["findings"] = json!(findings.len());
    report["summary"]["byCheck"] = Value::Object(by_check);
}

fn apply_audit_baseline(
    report: &mut Value,
    args: &Args,
    baseline: Option<Baseline>,
    first: &Scrape,
) -> Result<()> {
    let findings = report["findings"].as_array().cloned().unwrap_or_default();
    let all: HashSet<String> = findings
        .iter()
        .filter_map(|finding| finding["fingerprint"].as_str().map(str::to_string))
        .collect();
    if args.write_baseline || args.update_baseline {
        let Some(path) = resolve_baseline_path(
            args.baseline.as_deref(),
            args.no_baseline,
            args.write_baseline,
            args.update_baseline,
        ) else {
            anyhow::bail!("baseline writing is disabled");
        };
        let mut output = if args.update_baseline {
            baseline
                .as_ref()
                .map(|entry| entry.fingerprints.clone())
                .unwrap_or_default()
        } else {
            HashSet::new()
        };
        output.extend(all);
        write_baseline(&path, &output)?;
        report["baseline"] = json!({
            "path": path.display().to_string(),
            "written": output.len()
        });
        return Ok(());
    }
    let Some(baseline) = baseline else {
        return Ok(());
    };
    let suppressed = baseline_filter(report, &baseline);
    report["baseline"] = json!({
        "path": baseline.path.display().to_string(),
        "suppressed": suppressed
    });
    refresh_summary(report);
    let findings = report["findings"].as_array().cloned().unwrap_or_default();
    report["perState"] = json!(per_state_report(&findings, args, first));
    Ok(())
}

fn apply_source_baseline(
    report: &mut Value,
    args: &Args,
    baseline: Option<Baseline>,
) -> Result<()> {
    let findings = report["findings"].as_array().cloned().unwrap_or_default();
    let all: HashSet<String> = findings
        .iter()
        .filter_map(|finding| finding["fingerprint"].as_str().map(str::to_string))
        .collect();
    if args.write_baseline || args.update_baseline {
        let Some(path) = resolve_baseline_path(
            args.baseline.as_deref(),
            args.no_baseline,
            args.write_baseline,
            args.update_baseline,
        ) else {
            anyhow::bail!("baseline writing is disabled");
        };
        let mut output = if args.update_baseline {
            baseline
                .as_ref()
                .map(|entry| entry.fingerprints.clone())
                .unwrap_or_default()
        } else {
            HashSet::new()
        };
        output.extend(all);
        write_baseline(&path, &output)?;
        report["baseline"] = json!({
            "path": path.display().to_string(),
            "written": output.len()
        });
        return Ok(());
    }
    let Some(baseline) = baseline else {
        return Ok(());
    };
    let suppressed = baseline_filter(report, &baseline);
    report["baseline"] = json!({
        "path": baseline.path.display().to_string(),
        "suppressed": suppressed
    });
    refresh_summary(report);
    Ok(())
}

fn push_check(checks: &mut Vec<Value>, ok: &mut bool, name: &str, passed: bool, detail: String) {
    if !passed {
        *ok = false;
    }
    checks.push(json!({
        "name": name,
        "ok": passed,
        "detail": detail
    }));
}

fn doctor_mode(argv: &[String]) -> Result<i32> {
    let mut json_output = false;
    let mut chrome = std::env::var("UI_PROBE_CHROME")
        .or_else(|_| std::env::var("CHROME"))
        .ok()
        .map(PathBuf::from);
    let mut rules = None;
    let mut design = None;
    let mut argv = argv.iter();
    while let Some(arg) = argv.next() {
        match arg.as_str() {
            "--json" => json_output = true,
            "--chrome" => {
                chrome = Some(PathBuf::from(argv.next().context("--chrome needs a path")?))
            }
            "--rules" => rules = Some(PathBuf::from(argv.next().context("--rules needs a path")?)),
            "--design" => design = Some(argv.next().context("--design needs a path")?.clone()),
            "--help" | "-h" => {
                println!("usage: ui-probe doctor [--json] [--chrome PATH] [--rules PATH] [--design PATH]");
                return Ok(0);
            }
            other => anyhow::bail!("unknown argument: {other}"),
        }
    }

    let mut checks = Vec::new();
    let mut ok = true;

    match discover_chrome(chrome.as_deref()) {
        Ok(path) => match Command::new(&path).arg("--version").output() {
            Ok(output) if output.status.success() => {
                let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
                push_check(
                    &mut checks,
                    &mut ok,
                    "chromium",
                    true,
                    format!("{} ({version})", path.display()),
                );
            }
            Ok(output) => push_check(
                &mut checks,
                &mut ok,
                "chromium",
                false,
                format!("{} exited {}", path.display(), output.status),
            ),
            Err(error) => push_check(
                &mut checks,
                &mut ok,
                "chromium",
                false,
                format!("{}: {error}", path.display()),
            ),
        },
        Err(error) => push_check(&mut checks, &mut ok, "chromium", false, error.to_string()),
    }

    let cwd = std::env::current_dir().unwrap_or_default();
    let project = project_config_dir(&cwd)
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "none".to_string());
    push_check(&mut checks, &mut ok, "project", true, project);

    let design_sources = resolve_design_sources(design.as_deref());
    let mut design_spec = None;
    if design_sources.is_empty() {
        push_check(&mut checks, &mut ok, "design", true, "none".to_string());
    } else {
        match load_design_sources(&design_sources) {
            Ok((spec, notes)) => {
                design_spec = Some(spec);
                let paths: Vec<String> = design_sources
                    .iter()
                    .map(|(path, _)| path.display().to_string())
                    .collect();
                push_check(
                    &mut checks,
                    &mut ok,
                    "design",
                    true,
                    format!("{} ({} notes)", paths.join(", "), notes.len()),
                );
            }
            Err(error) => push_check(&mut checks, &mut ok, "design", false, error.to_string()),
        }
    }

    let rule_sources = resolve_rules_sources(rules.as_deref());
    let rule_sources = expand_rules_sources(&rule_sources)?;
    if rule_sources.is_empty() {
        push_check(&mut checks, &mut ok, "rules", true, "none".to_string());
    } else {
        let paths: Vec<PathBuf> = rule_sources.iter().map(|(path, _)| path.clone()).collect();
        match load_rules(&paths, design_spec.as_ref()) {
            Ok((set, notes)) => {
                let labels: Vec<String> = rule_sources
                    .iter()
                    .map(|(path, _)| path.display().to_string())
                    .collect();
                push_check(
                    &mut checks,
                    &mut ok,
                    "rules",
                    true,
                    format!(
                        "{} ({} structural, {} notes, {} coverage gaps)",
                        labels.join(", "),
                        set.structural_len(),
                        notes.len(),
                        set.coverage().len()
                    ),
                );
            }
            Err(error) => push_check(&mut checks, &mut ok, "rules", false, error.to_string()),
        }
    }

    let baseline_path = resolve_baseline_path(None, false, false, false);
    match load_baseline(baseline_path.as_deref(), true) {
        Ok(Some(baseline)) => push_check(
            &mut checks,
            &mut ok,
            "baseline",
            true,
            format!(
                "{} ({} fingerprints)",
                baseline.path.display(),
                baseline.fingerprints.len()
            ),
        ),
        Ok(None) => push_check(&mut checks, &mut ok, "baseline", true, "none".to_string()),
        Err(error) => push_check(&mut checks, &mut ok, "baseline", false, error.to_string()),
    }

    let report = json!({
        "tool": "ui-probe",
        "version": env!("CARGO_PKG_VERSION"),
        "schemaVersion": 1,
        "ok": ok,
        "checks": checks
    });
    if json_output {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        for check in report["checks"].as_array().cloned().unwrap_or_default() {
            println!(
                "{} {}: {}",
                if check["ok"].as_bool().unwrap_or(false) {
                    "ok"
                } else {
                    "FAIL"
                },
                check["name"].as_str().unwrap_or("check"),
                check["detail"].as_str().unwrap_or("")
            );
        }
    }
    Ok(if ok { 0 } else { 1 })
}

fn init_mode(argv: &[String]) -> Result<i32> {
    let mut target = PathBuf::from(".");
    for arg in argv {
        if arg == "--help" || arg == "-h" {
            println!("usage: ui-probe init [DIR]");
            return Ok(0);
        }
        if arg.starts_with('-') {
            anyhow::bail!("unknown argument: {arg}");
        }
        target = PathBuf::from(arg);
    }
    let config = target.join(".opal");
    std::fs::create_dir_all(&config).with_context(|| format!("create {}", config.display()))?;
    let design = config.join("design.json");
    if design.exists() {
        println!("kept {}", design.display());
    } else {
        std::fs::write(&design, "{}\n").with_context(|| format!("write {}", design.display()))?;
        println!("created {}", design.display());
    }
    let readme = config.join("README.md");
    if readme.exists() {
        println!("kept {}", readme.display());
    } else {
        std::fs::write(&readme, INIT_README)
            .with_context(|| format!("write {}", readme.display()))?;
        println!("created {}", readme.display());
    }
    println!(
        "next: fill {} then run ui-probe audit <url>",
        design.display()
    );
    Ok(0)
}

const INIT_README: &str = "\
# opal

- `design.json` holds project design parameters. Every key is optional.
- `rules.json` is optional. Named rules overlay by id; declarative version 2 rules append, with verbs in, notIn, equals, notEquals, present, and absent.
- Rules resolve from `--rules`, the nearest `.opal/rules.json`, then `~/.opal/rules.json`.
- Design layers from `--design`, the nearest `.opal/design.json`, then `~/.opal/design.json`.
- `baseline.json` stores accepted finding fingerprints when you run `--write-baseline`.

Run:

    ui-probe audit http://localhost:3000 --theme both --viewport sweep
    ui-probe source .
    ui-probe pack
";

fn pack_mode(argv: &[String]) -> Result<i32> {
    let mut explicit = None;
    let mut json_output = false;
    let mut argv = argv.iter();
    while let Some(arg) = argv.next() {
        match arg.as_str() {
            "--rules" => {
                explicit = Some(PathBuf::from(argv.next().context("--rules needs a path")?))
            }
            "--json" => json_output = true,
            "--help" | "-h" => {
                println!("usage: ui-probe pack [--rules PATH] [--json]");
                return Ok(0);
            }
            other => anyhow::bail!("unknown argument: {other}"),
        }
    }
    let sources = resolve_rules_sources(explicit.as_deref());
    let sources = expand_rules_sources(&sources)?;
    if sources.is_empty() {
        anyhow::bail!("no rules pack found; pass --rules or add .opal/rules.json");
    }
    for (source, _) in &sources {
        if !source.is_file() {
            anyhow::bail!("rules file not found: {}", source.display());
        }
    }
    if json_output {
        let entries: Vec<Value> = sources
            .iter()
            .map(|(path, origin)| json!({ "path": path.display().to_string(), "origin": origin }))
            .collect();
        println!("{}", serde_json::to_string(&json!({ "sources": entries }))?);
    } else {
        for (source, _) in &sources {
            println!("{}", source.display());
        }
    }
    Ok(0)
}

fn audit_mode(mut args: Args) -> Result<i32> {
    if let Some(root) = &args.source {
        return source_mode(root, &args);
    }

    let design_sources = resolve_design_sources(args.design.as_deref());
    let design_origins: Vec<String> = if args.design.as_deref() == Some("auto") {
        vec!["auto".to_string()]
    } else {
        design_sources
            .iter()
            .map(|(_, origin)| (*origin).to_string())
            .collect()
    };
    if let Some((path, _)) = design_sources.first() {
        args.design = Some(path.display().to_string());
    }
    let loaded_design = if design_sources.is_empty() {
        None
    } else {
        Some(load_design_sources(&design_sources)?)
    };
    let design_spec = loaded_design.as_ref().map(|(spec, _)| spec);
    let design_notes = loaded_design
        .as_ref()
        .map(|(_, notes)| notes.clone())
        .unwrap_or_default();
    let design_source_labels: Vec<String> = design_sources
        .iter()
        .map(|(path, _)| path.display().to_string())
        .collect();

    let baseline_path = resolve_baseline_path(
        args.baseline.as_deref(),
        args.no_baseline,
        args.write_baseline,
        args.update_baseline,
    );
    let baseline = load_baseline(
        baseline_path.as_deref(),
        args.write_baseline || args.update_baseline,
    )?;

    let rule_notes;
    let rule_sources = resolve_rules_sources(args.rules.as_deref());
    let rule_sources = expand_rules_sources(&rule_sources)?;
    let mut rule_source_labels: Vec<String> = rule_sources
        .iter()
        .map(|(path, _)| path.display().to_string())
        .collect();
    let rule_origins: Vec<String> = rule_sources
        .iter()
        .map(|(_, origin)| (*origin).to_string())
        .collect();
    let rule_paths: Vec<PathBuf> = rule_sources.iter().map(|(path, _)| path.clone()).collect();
    let ruleset = if rule_paths.is_empty() {
        let (rules, notes) = load_default_rules(design_spec)?;
        rule_notes = notes;
        rule_source_labels.push("default-rules.json".to_string());
        Some(rules)
    } else {
        let (rules, notes) = load_rules_with_default(&rule_paths, design_spec)?;
        rule_notes = notes;
        Some(rules)
    };

    let session = launch_browser(&args)?;
    let both_themes = args.theme.as_deref() == Some("both");
    let sweep = args.viewports.len() > 1;
    let mut runs: Vec<Run> = Vec::new();

    for viewport in &args.viewports {
        let label = format!("{}x{}", viewport.0, viewport.1);
        if both_themes {
            for theme in ["light", "dark"] {
                runs.push(Run {
                    theme: Some(theme.to_string()),
                    viewport: label.clone(),
                    scrape: scrape_once(
                        &session,
                        &args,
                        ScrapeContext {
                            theme: Some(theme),
                            viewport: *viewport,
                            ruleset: ruleset.as_ref(),
                            design: design_spec,
                            design_notes: &design_notes,
                            rule_notes: &rule_notes,
                        },
                    )?,
                });
            }
        } else {
            let theme = args.theme.clone().filter(|value| value != "both");
            runs.push(Run {
                theme: theme.clone(),
                viewport: label,
                scrape: scrape_once(
                    &session,
                    &args,
                    ScrapeContext {
                        theme: theme.as_deref(),
                        viewport: *viewport,
                        ruleset: ruleset.as_ref(),
                        design: design_spec,
                        design_notes: &design_notes,
                        rule_notes: &rule_notes,
                    },
                )?,
            });
        }
    }

    let mut report = build_report(
        &args,
        &runs,
        both_themes,
        sweep,
        &ReportSources {
            rules: &rule_source_labels,
            rule_origins: &rule_origins,
            design: &design_source_labels,
            design_origins: &design_origins,
        },
    );
    if let Some(path) = write_design_if_needed(&args, runs[0].scrape.inferred.as_ref())? {
        report["designFile"] = json!(path);
    }
    apply_audit_baseline(&mut report, &args, baseline, &runs[0].scrape)?;
    drop(session);

    emit_report(&report, args.format, args.pretty)?;

    if args.write_baseline || args.update_baseline {
        return Ok(0);
    }

    let has_error = report["findings"]
        .as_array()
        .map(|list| {
            list.iter().any(|finding| {
                matches!(
                    finding["severity"].as_str(),
                    Some("P0" | "P1" | "P2" | "error")
                )
            })
        })
        .unwrap_or(false);
    Ok(if has_error { 2 } else { 0 })
}

struct Run {
    theme: Option<String>,
    viewport: String,
    scrape: Scrape,
}

struct Scrape {
    baseline: Value,
    findings: Vec<Value>,
    sheets: Value,
    keyframes: Vec<Value>,
    motion: Value,
    focus_states: Vec<Value>,
    inferred: Option<Value>,
    unverified: Vec<Value>,
    expanded: Vec<Value>,
    skipped: Vec<Value>,
    disabled_checks: Vec<String>,
}

#[derive(Default)]
struct PageDiagnostics {
    console: Vec<Value>,
    page_errors: Vec<Value>,
    network: Vec<Value>,
}

fn runtime_findings(diagnostics: &PageDiagnostics) -> Vec<Value> {
    let mut findings = Vec::new();
    let mut seen = HashSet::new();
    let mut push = |check: &str, severity: &str, message: String, evidence: Value| {
        let key = format!("{check}|{message}|{evidence}");
        if seen.insert(key) {
            findings.push(json!({
                "check": check,
                "layer": "runtime",
                "severity": severity,
                "message": message,
                "evidence": evidence
            }));
        }
    };
    for entry in &diagnostics.console {
        let level = entry["level"].as_str().unwrap_or("warning");
        let text = entry["text"]
            .as_str()
            .unwrap_or("")
            .chars()
            .take(240)
            .collect::<String>();
        if level == "error" {
            push(
                "console-error",
                "P2",
                format!("console error: {text}"),
                entry.clone(),
            );
        } else {
            push(
                "console-warning",
                "P3",
                format!("console warning: {text}"),
                entry.clone(),
            );
        }
    }
    for entry in &diagnostics.page_errors {
        let text = entry["text"]
            .as_str()
            .unwrap_or("")
            .chars()
            .take(240)
            .collect::<String>();
        push(
            "page-error",
            "P2",
            format!("page error: {text}"),
            entry.clone(),
        );
    }
    for entry in &diagnostics.network {
        let url = entry["url"].as_str().unwrap_or("");
        let text = entry["text"].as_str().unwrap_or("");
        if entry["kind"].as_str() == Some("failed") {
            push(
                "network-error",
                "P2",
                format!("network failed: {url} ({text})"),
                entry.clone(),
            );
        } else {
            let status = entry["status"].as_u64().unwrap_or(0);
            let severity = if status >= 500 { "P2" } else { "P3" };
            push(
                "network-error",
                severity,
                format!("network {status}: {url}"),
                entry.clone(),
            );
        }
    }
    findings.truncate(100);
    findings
}

struct ScrapeContext<'a> {
    theme: Option<&'a str>,
    viewport: (u32, u32),
    ruleset: Option<&'a RuleSet>,
    design: Option<&'a DesignSpec>,
    design_notes: &'a [Value],
    rule_notes: &'a [Value],
}

fn scrape_once(
    session: &BrowserSession,
    args: &Args,
    context: ScrapeContext<'_>,
) -> Result<Scrape> {
    session
        .runtime()
        .block_on(scrape_once_async(session.browser(), args, context))
}

async fn scrape_once_async(
    browser: &Browser,
    args: &Args,
    context: ScrapeContext<'_>,
) -> Result<Scrape> {
    let theme = context.theme;
    let viewport = context.viewport;
    let ruleset = context.ruleset;
    let design = context.design;
    let design_notes = context.design_notes;
    let rule_notes = context.rule_notes;
    let tab = browser.new_page().await.context("open tab")?;
    tab.set_default_timeout(args.timeout.as_millis() as f64)
        .await;
    tab.set_default_navigation_timeout(args.timeout.as_millis() as f64)
        .await;
    let diagnostics = Arc::new(Mutex::new(PageDiagnostics::default()));
    let log_events = Arc::clone(&diagnostics);
    tab.on_console(move |message| {
        let log_events = Arc::clone(&log_events);
        let level = message.type_().to_string();
        let text = message.text().to_string();
        let location = message.location().clone();
        async move {
            if level == "error" || level == "warning" {
                log_events.lock().unwrap().console.push(json!({
                    "level": level,
                    "text": text,
                    "source": "console",
                    "url": location.url,
                    "line": location.line_number
                }));
            }
            Ok(())
        }
    })
    .await
    .context("enable browser console")?;
    let log_events = Arc::clone(&diagnostics);
    tab.on_pageerror(move |text| {
        let log_events = Arc::clone(&log_events);
        async move {
            log_events.lock().unwrap().page_errors.push(json!({
                "text": text,
                "url": "",
                "line": 0
            }));
            Ok(())
        }
    })
    .await
    .context("enable browser runtime")?;
    let log_events = Arc::clone(&diagnostics);
    tab.on_request_failed(move |request| {
        let log_events = Arc::clone(&log_events);
        let url = request.url().to_string();
        let text = request.failure().unwrap_or_default();
        async move {
            if url.starts_with("data:")
                || url.ends_with("/favicon.ico")
                || text.contains("ERR_ABORTED")
            {
                return Ok(());
            }
            log_events.lock().unwrap().network.push(json!({
                "kind": "failed",
                "status": 0,
                "url": url,
                "text": text
            }));
            Ok(())
        }
    })
    .await
    .context("enable browser network")?;
    let response_events = Arc::clone(&diagnostics);
    tab.on_response(move |response| {
        let response_events = Arc::clone(&response_events);
        let status = response.status();
        let url = response.url().to_string();
        let text = response.status_text().to_string();
        async move {
            if status >= 400 && !url.starts_with("data:") && !url.ends_with("/favicon.ico") {
                response_events.lock().unwrap().network.push(json!({
                    "kind": "status",
                    "status": status,
                    "url": url,
                    "text": text
                }));
            }
            Ok(())
        }
    })
    .await
    .context("enable browser responses")?;
    tab.set_viewport_size(Viewport {
        width: viewport.0,
        height: viewport.1,
    })
    .await
    .context("set device metrics")?;

    if let Some(theme) = theme {
        let color_scheme = if theme == "dark" {
            ColorScheme::Dark
        } else {
            ColorScheme::Light
        };
        tab.emulate_media(Some(
            EmulateMediaOptions::builder()
                .color_scheme(color_scheme)
                .build(),
        ))
        .await
        .context("set emulated media")?;
    }

    let needs_first_paint = ruleset.map(RuleSet::needs_first_paint).unwrap_or(false);
    if needs_first_paint {
        if let Some(rules) = ruleset {
            tab.add_init_script(&capture::first_paint_script(
                rules.selectors(),
                rules.properties(),
            ))
            .await
            .context("install first-paint capture")?;
        }
    }

    load(&tab, args).await?;
    if let Some(rules) = ruleset {
        validate_selectors(&tab, rules).await?;
    }
    let lifecycle_evidence = if needs_first_paint {
        evaluate_js(
            &tab,
            "(() => { const raw = window.__opal_first_paint; return raw || null; })()",
        )
        .await
        .ok()
        .filter(Value::is_object)
    } else {
        None
    };
    let lifecycle_note = if needs_first_paint
        && lifecycle_evidence
            .as_ref()
            .and_then(|value| value["baseline"]["elements"].as_array())
            .map(|elements| elements.len() <= 2)
            .unwrap_or(true)
    {
        Some(json!({
            "check": "lifecycle",
            "reason": "first-paint evidence unavailable",
            "lifecycle": "first-paint"
        }))
    } else {
        None
    };
    let baseline = evaluate(&tab).await?;
    let has_content = baseline["elements"]
        .as_array()
        .map(|list| {
            list.iter().any(|element| {
                !matches!(
                    element["tag"].as_str(),
                    Some(
                        "html"
                            | "head"
                            | "body"
                            | "title"
                            | "meta"
                            | "link"
                            | "style"
                            | "script"
                            | "noscript"
                            | "base"
                    )
                )
            })
        })
        .unwrap_or(false);
    if !has_content {
        anyhow::bail!("empty capture: the page loaded but no elements were sampled");
    }
    let sheets = evaluate_js(&tab, STYLES_JS).await.unwrap_or_else(|error| {
        json!({
            "sheets": [],
            "unreadable": [],
            "error": format!("{error:#}")
        })
    });
    let sheet_text = sheets_text(&sheets);

    let mut findings = Vec::new();
    if let Some(rules) = ruleset {
        let oracle = load_oracle(&tab, rules).await?;
        findings.extend(with_state(
            apply_ruleset(
                rule_findings(
                    rules,
                    &oracle,
                    &baseline,
                    design,
                    &sheet_text,
                    lifecycle_evidence.as_ref(),
                ),
                ruleset,
            ),
            &args.state,
        ));
    }
    let baseline_keys: HashSet<String> = findings.iter().map(finding_key).collect();
    let mut expanded: Vec<Value> = Vec::new();
    let mut skipped: Vec<Value> = Vec::new();

    if args.expand {
        let discovered = evaluate_js(&tab, DISCOVER_JS).await?;
        for candidate in discovered.as_array().cloned().unwrap_or_default() {
            let name = candidate["name"].as_str().unwrap_or("").trim().to_string();
            let index = candidate["index"].as_u64().unwrap_or(0) as usize;
            let kind = candidate["kind"].clone();

            if is_destructive(&name) {
                skipped.push(json!({ "name": name, "reason": "destructive" }));
                continue;
            }
            if expanded.len() >= args.max_states {
                skipped.push(json!({ "name": name, "reason": "max-states" }));
                continue;
            }

            load(&tab, args).await?;
            let fresh = evaluate_js(&tab, DISCOVER_JS).await?;
            let available = fresh.as_array().map(|list| list.len()).unwrap_or(0);
            if index >= available {
                skipped.push(json!({ "name": name, "reason": "not-found" }));
                continue;
            }
            if !evaluate_js(&tab, &activate_js(index))
                .await?
                .as_bool()
                .unwrap_or(false)
            {
                skipped.push(json!({ "name": name, "reason": "not-clickable" }));
                continue;
            }

            let mut activated = false;
            for _ in 0..20 {
                if evaluate_js(&tab, &activated_js(index))
                    .await?
                    .as_bool()
                    .unwrap_or(false)
                {
                    activated = true;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            if !activated {
                skipped.push(json!({
                    "name": if name.is_empty() { "unnamed".to_string() } else { name.clone() },
                    "reason": "postcondition-not-met",
                    "selector": candidate["selector"]
                }));
                continue;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;

            let state_data = evaluate(&tab).await?;
            let generic_delta: Vec<Value> = Vec::new();
            let mut state_rules: Vec<Value> = Vec::new();
            if let Some(rules) = ruleset {
                let oracle = load_oracle(&tab, rules).await?;
                state_rules = apply_ruleset(
                    rule_findings(
                        rules,
                        &oracle,
                        &state_data,
                        design,
                        &sheet_text,
                        lifecycle_evidence.as_ref(),
                    ),
                    ruleset,
                );
            }
            let state_name = slug(&name, index);
            let state_count = generic_delta.len()
                + state_rules
                    .iter()
                    .filter(|finding| !baseline_keys.contains(&finding_key(finding)))
                    .count();
            findings.extend(with_state(generic_delta, &state_name));
            findings.extend(with_state(state_rules, &state_name));
            let postcondition = match kind.as_str() {
                Some("details") => "details.open",
                Some("tab") => "aria-selected=true",
                _ => "aria-expanded=true or controlled region visible",
            };
            expanded.push(json!({
                "name": state_name,
                "kind": kind,
                "reached": format!("click {}", candidate["selector"].as_str().unwrap_or("?")),
                "postcondition": postcondition,
                "findings": state_count
            }));
        }
    }

    let mut extra = stylesheet_checks(&baseline, &sheet_text);
    {
        let diagnostics = diagnostics.lock().unwrap();
        extra.extend(runtime_findings(&diagnostics));
    }

    let (motion, motion_findings) = motion_evidence(&baseline, &sheet_text, false);
    extra.extend(motion_findings);
    let mut design_unverified: Vec<Value> = Vec::new();
    if let Some(spec) = design {
        let (design_findings, notes) = design_checks(&baseline, spec, &sheet_text);
        extra.extend(design_findings);
        design_unverified.extend(notes);
    }

    let focus_states = if args.sample_states {
        let sampled = evaluate_js(&tab, FOCUS_JS).await?;
        extra.extend(focus_state_findings(&sampled));
        sampled.as_array().cloned().unwrap_or_default()
    } else {
        Vec::new()
    };

    let explicit_design = design.is_some();
    let infer =
        args.design.is_none() || args.design.as_deref() == Some("auto") || args.infer_missing;
    let inferred = if infer {
        let (report, inferred_findings) = infer_design(&baseline);
        if !explicit_design {
            extra.extend(inferred_findings);
        }
        Some(report)
    } else {
        None
    };

    if let Some(rules) = ruleset {
        let declared = rules.declared_names();
        extra.retain(|finding| !declared.contains(finding["check"].as_str().unwrap_or("")));
    }
    findings.extend(with_state(apply_ruleset(extra, ruleset), &args.state));

    let mut unverified_entries: Vec<Value> = Vec::new();
    for sheet in sheets["unreadable"].as_array().cloned().unwrap_or_default() {
        unverified_entries.push(json!({
            "check": "stylesheets",
            "reason": "cross-origin sheet unreadable",
            "sheet": sheet
        }));
    }
    if let Some(error) = sheets["error"].as_str() {
        unverified_entries.push(json!({
            "check": "stylesheets",
            "reason": error
        }));
    }
    unverified_entries.extend(design_notes.to_vec());
    unverified_entries.extend(rule_notes.to_vec());
    if let Some(note) = lifecycle_note {
        unverified_entries.push(note);
    }
    if let Some(rules) = ruleset {
        unverified_entries.extend(rules.coverage().to_vec());
    }
    unverified_entries.extend(design_unverified);
    unverified_entries.extend(colour_unverified(&baseline));
    let unverified_entries = apply_unverified_rules(unverified_entries, ruleset);
    let disabled_checks = ruleset.map(RuleSet::disabled_checks).unwrap_or_default();

    Ok(Scrape {
        baseline,
        findings,
        sheets,
        keyframes: keyframes(&sheet_text),
        motion,
        focus_states,
        inferred,
        unverified: unverified_entries,
        expanded,
        skipped,
        disabled_checks,
    })
}

async fn validate_selectors(tab: &Page, rules: &RuleSet) -> Result<()> {
    let errors = evaluate_js(tab, &validate_js(rules.selectors())).await?;
    if let Some(list) = errors.as_array() {
        if let Some(error) = list.first() {
            anyhow::bail!(
                "rules: invalid selector {}: {}",
                error["selector"].as_str().unwrap_or("?"),
                error["error"].as_str().unwrap_or("invalid selector")
            );
        }
    }
    Ok(())
}

#[derive(Default)]
struct SelectorOracle {
    matches: Vec<usize>,
    values: std::collections::HashMap<String, std::collections::HashMap<usize, String>>,
}

#[derive(Default)]
struct Oracle {
    selectors: std::collections::HashMap<String, SelectorOracle>,
}

async fn load_oracle(tab: &Page, rules: &RuleSet) -> Result<Oracle> {
    let data = evaluate_js(tab, &oracle_js(rules.selectors(), rules.properties())).await?;
    oracle_from_value(&data)
}

fn oracle_from_value(data: &Value) -> Result<Oracle> {
    let mut oracle = Oracle::default();
    if let Some(object) = data["selectors"].as_object() {
        for (selector, entry) in object {
            if let Some(error) = entry["error"].as_str() {
                anyhow::bail!("rules: invalid selector {selector}: {error}");
            }
            let mut value = SelectorOracle::default();
            for index in entry["matches"].as_array().cloned().unwrap_or_default() {
                if let Some(index) = index.as_u64() {
                    value.matches.push(index as usize);
                }
            }
            if let Some(properties) = entry["values"].as_object() {
                for (property, values) in properties {
                    let mut map = std::collections::HashMap::new();
                    if let Some(values) = values.as_object() {
                        for (index, raw) in values {
                            if let (Ok(index), Some(raw)) = (index.parse::<usize>(), raw.as_str()) {
                                map.insert(index, raw.to_string());
                            }
                        }
                    }
                    value.values.insert(property.clone(), map);
                }
            }
            oracle.selectors.insert(selector.clone(), value);
        }
    }
    Ok(oracle)
}

impl Oracle {
    fn matches(&self, selector: &str) -> &[usize] {
        self.selectors
            .get(selector)
            .map(|entry| entry.matches.as_slice())
            .unwrap_or(&[])
    }

    fn value(&self, selector: &str, property: &str, index: usize) -> Option<&str> {
        self.selectors
            .get(selector)?
            .values
            .get(property)?
            .get(&index)
            .map(|value| value.as_str())
    }
}

fn colour_unverified(baseline: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    for element in baseline["elements"].as_array().cloned().unwrap_or_default() {
        if !element["visible"].as_bool().unwrap_or(false) {
            continue;
        }
        for failure in element["colourFailures"]
            .as_array()
            .cloned()
            .unwrap_or_default()
        {
            out.push(json!({
                "check": "colour",
                "reason": "colour conversion failed",
                "selector": element["selector"],
                "property": failure["property"],
                "value": failure["value"]
            }));
        }
    }
    out
}

fn layer_for(check: &str) -> &'static str {
    if check.starts_with("sheet-") || check == "bounce-easing" {
        "sheet"
    } else if check.starts_with("copy-")
        || check.starts_with("numbered-label")
        || check == "em-dash-overuse"
        || check == "marketing-buzzword"
    {
        "content"
    } else {
        "layout"
    }
}

fn severity_rank(finding: &Value) -> u8 {
    match finding["severity"].as_str().unwrap_or("P3") {
        "P0" => 0,
        "P1" => 1,
        "P2" => 2,
        _ => 3,
    }
}

fn sort_findings(findings: &mut [Value]) {
    findings.sort_by(|left, right| {
        severity_rank(left)
            .cmp(&severity_rank(right))
            .then_with(|| {
                left["check"]
                    .as_str()
                    .unwrap_or("")
                    .cmp(right["check"].as_str().unwrap_or(""))
            })
            .then_with(|| {
                left["state"]
                    .as_str()
                    .unwrap_or("")
                    .cmp(right["state"].as_str().unwrap_or(""))
            })
            .then_with(|| {
                left["selector"]
                    .as_str()
                    .unwrap_or("")
                    .cmp(right["selector"].as_str().unwrap_or(""))
            })
            .then_with(|| {
                left["message"]
                    .as_str()
                    .unwrap_or("")
                    .cmp(right["message"].as_str().unwrap_or(""))
            })
            .then_with(|| left.to_string().cmp(&right.to_string()))
    });
}

fn sort_value_entries(entries: &mut [Value]) {
    entries.sort_by(|left, right| {
        right["count"]
            .as_u64()
            .cmp(&left["count"].as_u64())
            .then_with(|| left.to_string().cmp(&right.to_string()))
    });
}

fn normalize_findings(findings: &mut [Value], generic: bool) {
    for finding in findings.iter_mut() {
        let check = finding["check"].as_str().unwrap_or("").to_string();
        if finding["layer"].is_null() {
            finding["layer"] = json!(layer_for(&check));
        }
        let drift = check.starts_with("design-")
            || check.starts_with("inferred-")
            || check.starts_with("surface-")
            || check == "motion-duration";
        if drift {
            finding["inferred"] = json!(check.starts_with("inferred-"));
        }
        if finding["lifecycle"].is_null() {
            finding["lifecycle"] = json!("settled");
        }
        if generic && finding["severity"].is_null() {
            finding["severity"] = json!("advisory");
        }
        finding["fingerprint"] = json!(finding_fingerprint(finding));
    }
    sort_findings(findings);
}

struct ReportSources<'a> {
    rules: &'a [String],
    rule_origins: &'a [String],
    design: &'a [String],
    design_origins: &'a [String],
}

fn build_report(
    args: &Args,
    runs: &[Run],
    both_themes: bool,
    sweep: bool,
    sources: &ReportSources,
) -> Value {
    let first = &runs[0].scrape;
    let findings = if runs.len() > 1 {
        merge_findings(runs, both_themes, sweep)
    } else {
        merge_states(first.findings.clone())
    };
    let findings: Vec<Value> = findings
        .into_iter()
        .filter(|finding| {
            !args
                .allow
                .iter()
                .any(|allowed| finding["check"].as_str() == Some(allowed))
        })
        .collect();
    let mut findings = findings;
    normalize_findings(&mut findings, true);
    let mut by_check: serde_json::Map<String, Value> = serde_json::Map::new();
    for finding in &findings {
        let name = finding["check"].as_str().unwrap_or("unknown").to_string();
        let count = by_check.get(&name).and_then(Value::as_u64).unwrap_or(0);
        by_check.insert(name, json!(count + 1));
    }
    let elements = first.baseline["elements"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter(|element| element["visible"].as_bool().unwrap_or(false))
                .count()
        })
        .unwrap_or(0);
    let captured = first.baseline["elements"]
        .as_array()
        .map(|list| list.len())
        .unwrap_or(0);
    let mut unverified_list = unverified(&first.baseline);
    unverified_list.extend(first.unverified.clone());
    let mut unverified_list = dedupe_values(unverified_list);
    unverified_list.retain(|entry| {
        entry["check"]
            .as_str()
            .map(|check| {
                !first
                    .disabled_checks
                    .iter()
                    .any(|disabled| disabled == check)
            })
            .unwrap_or(true)
    });
    let coverage_gaps: Vec<Value> = unverified_list
        .iter()
        .filter(|entry| entry["coverageGap"].as_bool() == Some(true))
        .cloned()
        .collect();

    let mut themes: Vec<String> = Vec::new();
    for theme in runs.iter().filter_map(|run| run.theme.clone()) {
        if !themes.contains(&theme) {
            themes.push(theme);
        }
    }
    let mut viewports: Vec<String> = Vec::new();
    for viewport in runs.iter().map(|run| run.viewport.clone()) {
        if !viewports.contains(&viewport) {
            viewports.push(viewport);
        }
    }

    json!({
        "tool": "ui-probe",
        "version": env!("CARGO_PKG_VERSION"),
        "schemaVersion": 1,
        "sources": {
            "rules": sources.rules,
            "rulesOrigin": sources.rule_origins,
            "design": sources.design,
            "designOrigin": sources.design_origins
        },
        "baseline": Value::Null,
        "state": args.state,
        "theme": args.theme,
        "themes": if both_themes { json!(themes) } else { Value::Null },
        "viewports": if sweep { json!(viewports) } else { Value::Null },
        "states": {
            "baseline": args.state,
            "expanded": first.expanded,
            "skipped": first.skipped
        },
        "perState": per_state_report(&findings, args, first),
        "parameters": {
            "declared": if args.design.as_deref().map(|value| value != "auto").unwrap_or(false) { json!(["design"]) } else { json!([]) },
            "inferred": if first.inferred.is_some() { json!(["design"]) } else { json!([]) },
            "absent": if args.design.is_none() { json!(["design"]) } else { json!([]) }
        },
        "reviewer": [],
        "url": first.baseline["url"],
        "title": first.baseline["title"],
        "lang": first.baseline["lang"],
        "viewport": first.baseline["viewport"],
        "scheme": first.baseline["scheme"],
        "reducedMotion": first.baseline["reducedMotion"],
        "summary": {
            "findings": findings.len(),
            "elements": elements,
            "captured": captured,
            "byCheck": by_check
        },
        "findings": findings,
        "candidates": first.baseline["candidates"],
        "focusOrder": first.baseline["focusables"],
        "stylesheets": {
            "sheets": first.sheets["sheets"].as_array().map(|list| list.len()).unwrap_or(0),
            "rules": first.sheets["sheets"].as_array().map(|list| list.iter().map(|sheet| sheet["rules"].as_array().map(|rules| rules.len()).unwrap_or(0)).sum::<usize>()).unwrap_or(0),
            "unreadable": first.sheets["unreadable"]
        },
        "keyframes": first.keyframes,
        "motion": first.motion,
        "focusStates": first.focus_states,
        "inferredDesign": first.inferred,
        "coverageGaps": coverage_gaps,
        "unverified": unverified_list
    })
}

fn write_design_if_needed(args: &Args, inferred: Option<&Value>) -> Result<Option<String>> {
    let Some(inferred) = inferred else {
        return Ok(None);
    };
    let target = if args.design.as_deref() == Some("auto") {
        PathBuf::from(".ui-witness/design.json")
    } else if args.infer_missing {
        args.design
            .as_deref()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(".ui-witness/design.json"))
    } else {
        return Ok(None);
    };
    let inferred = inferred_design_document(inferred);
    if target.exists() {
        if args.infer_missing {
            let text = std::fs::read_to_string(&target)
                .with_context(|| format!("read design file {}", target.display()))?;
            let mut existing: Value = serde_json::from_str(&text)
                .with_context(|| format!("parse design file {}", target.display()))?;
            if !existing.is_object() {
                anyhow::bail!("design file {} is not a JSON object", target.display());
            }
            merge_missing(&mut existing, &inferred);
            std::fs::write(&target, serde_json::to_string_pretty(&existing)?)
                .with_context(|| format!("write design file {}", target.display()))?;
        } else if args.force {
            std::fs::write(&target, serde_json::to_string_pretty(&inferred)?)
                .with_context(|| format!("write design file {}", target.display()))?;
        } else {
            anyhow::bail!(
                "design file already exists: {} (use --force or --infer-missing)",
                target.display()
            );
        }
    } else {
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create {}", parent.display()))?;
        }
        std::fs::write(&target, serde_json::to_string_pretty(&inferred)?)
            .with_context(|| format!("write design file {}", target.display()))?;
    }
    Ok(Some(target.display().to_string()))
}

fn inferred_design_document(inferred: &Value) -> Value {
    let mut document = serde_json::Map::new();
    let mut colors = serde_json::Map::new();
    for (index, entry) in inferred["palette"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .take(12)
        .enumerate()
    {
        let Some((r, g, b)) = entry["color"].as_str().and_then(parse_color_rgb) else {
            continue;
        };
        let count = entry["count"].as_u64().unwrap_or(0);
        colors.insert(
            format!("color-{}", index + 1),
            json!({
                "$value": format!("#{r:02x}{g:02x}{b:02x}"),
                "$type": "color",
                "$description": format!("Inferred colour, {count} observations")
            }),
        );
    }
    if !colors.is_empty() {
        document.insert("color".to_string(), Value::Object(colors));
    }

    let mut fonts = serde_json::Map::new();
    for (index, entry) in inferred["fonts"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .take(6)
        .enumerate()
    {
        let Some(family) = entry["family"].as_str() else {
            continue;
        };
        let count = entry["count"].as_u64().unwrap_or(0);
        fonts.insert(
            format!("font-{}", index + 1),
            json!({
                "$value": [family],
                "$type": "fontFamily",
                "$description": format!("Inferred font family, {count} observations")
            }),
        );
    }
    if !fonts.is_empty() {
        document.insert("font".to_string(), Value::Object(fonts));
    }

    let mut spacing = serde_json::Map::new();
    for (index, entry) in inferred["spacing"]["values"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .take(12)
        .enumerate()
    {
        let Some(value) = entry["value"].as_f64() else {
            continue;
        };
        let count = entry["count"].as_u64().unwrap_or(0);
        spacing.insert(
            format!("space-{}", index + 1),
            json!({
                "$value": format!("{value}px"),
                "$type": "dimension",
                "$description": format!("Inferred spacing, {count} observations")
            }),
        );
    }
    if !spacing.is_empty() {
        document.insert("spacing".to_string(), Value::Object(spacing));
    }

    let mut radii = serde_json::Map::new();
    for (index, entry) in inferred["radii"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .take(8)
        .enumerate()
    {
        let Some(value) = entry["value"].as_str() else {
            continue;
        };
        let count = entry["count"].as_u64().unwrap_or(0);
        radii.insert(
            format!("radius-{}", index + 1),
            json!({
                "$value": value,
                "$type": "dimension",
                "$description": format!("Inferred radius, {count} observations")
            }),
        );
    }
    if !radii.is_empty() {
        document.insert("radius".to_string(), Value::Object(radii));
    }

    let mut typography = serde_json::Map::new();
    for (index, entry) in inferred["typography"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .take(8)
        .enumerate()
    {
        let count = entry["count"].as_u64().unwrap_or(0);
        typography.insert(
            format!("type-{}", index + 1),
            json!({
                "$value": {
                    "fontFamily": entry["fontFamily"],
                    "fontSize": format!("{}px", entry["fontSize"].as_f64().unwrap_or(16.0)),
                    "fontWeight": entry["fontWeight"],
                    "lineHeight": entry["lineHeight"],
                    "letterSpacing": entry["letterSpacing"]
                },
                "$type": "typography",
                "$description": format!("Inferred typography, {count} observations")
            }),
        );
    }
    if !typography.is_empty() {
        document.insert("typography".to_string(), Value::Object(typography));
    }

    Value::Object(document)
}

fn merge_missing(existing: &mut Value, inferred: &Value) {
    let Some(existing_object) = existing.as_object_mut() else {
        *existing = inferred.clone();
        return;
    };
    let Some(inferred_object) = inferred.as_object() else {
        return;
    };
    for (group, tokens) in inferred_object {
        let flat_key = match group.as_str() {
            "color" => Some("colors"),
            "font" => Some("fontFamilies"),
            "spacing" => Some("spacingPx"),
            "radius" => Some("radiiPx"),
            "typography" => Some("typography"),
            _ => None,
        };
        if flat_key
            .map(|key| existing_object.contains_key(key))
            .unwrap_or(false)
        {
            continue;
        }
        if let Some(existing_group) = existing_object
            .get_mut(group)
            .and_then(Value::as_object_mut)
        {
            if let Some(tokens) = tokens.as_object() {
                for (key, token) in tokens {
                    if !existing_group.contains_key(key) {
                        existing_group.insert(key.clone(), token.clone());
                    }
                }
            }
        } else if !existing_object.contains_key(group) {
            existing_object.insert(group.clone(), tokens.clone());
        }
    }
}

fn per_state_report(findings: &[Value], args: &Args, first: &Scrape) -> Vec<Value> {
    let mut states = vec![json!({
        "name": args.state,
        "reached": "initial load",
        "postcondition": "page loaded",
        "findings": findings
            .iter()
            .filter(|finding| {
                finding["states"]
                    .as_array()
                    .map(|states| states.iter().any(|value| value.as_str() == Some(args.state.as_str())))
                    .unwrap_or(false)
            })
            .cloned()
            .collect::<Vec<_>>()
    })];
    for state in &first.expanded {
        let name = state["name"].as_str().unwrap_or("");
        states.push(json!({
            "name": name,
            "reached": state["reached"],
            "postcondition": state["postcondition"],
            "findings": findings
                .iter()
                .filter(|finding| {
                    finding["states"]
                        .as_array()
                        .map(|states| states.iter().any(|value| value.as_str() == Some(name)))
                        .unwrap_or(false)
                })
                .cloned()
                .collect::<Vec<_>>()
        }));
    }
    states
}

fn merge_states(findings: Vec<Value>) -> Vec<Value> {
    let mut merged: Vec<Value> = Vec::new();
    let mut index: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for finding in findings {
        let key = format!(
            "{}|{}|{}|{}",
            finding["check"].as_str().unwrap_or(""),
            finding["selector"].as_str().unwrap_or(""),
            finding["message"].as_str().unwrap_or(""),
            finding["property"].as_str().unwrap_or("")
        );
        if let Some(position) = index.get(&key).copied() {
            if let (Some(state), Some(states)) = (
                finding["state"].as_str(),
                merged[position]["states"].as_array_mut(),
            ) {
                if !states.iter().any(|value| value.as_str() == Some(state)) {
                    states.push(json!(state));
                }
            }
        } else {
            let mut entry = finding;
            let state = entry["state"].clone();
            entry["states"] = json!([state]);
            index.insert(key, merged.len());
            merged.push(entry);
        }
    }
    merged
}

fn merge_findings(runs: &[Run], both_themes: bool, sweep: bool) -> Vec<Value> {
    let mut merged: Vec<Value> = Vec::new();
    let mut index: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for run in runs {
        for finding in &run.scrape.findings {
            let key = format!(
                "{}|{}|{}|{}",
                finding["check"].as_str().unwrap_or(""),
                finding["selector"].as_str().unwrap_or(""),
                finding["message"].as_str().unwrap_or(""),
                finding["property"].as_str().unwrap_or("")
            );
            let position = index.get(&key).copied();
            if let Some(position) = position {
                let entry = &mut merged[position];
                if both_themes {
                    if let Some(theme) = &run.theme {
                        let themes = entry["themes"].as_array_mut().unwrap();
                        if !themes.iter().any(|value| value.as_str() == Some(theme)) {
                            themes.push(json!(theme));
                        }
                    }
                }
                if sweep {
                    let viewports = entry["viewports"].as_array_mut().unwrap();
                    if !viewports
                        .iter()
                        .any(|value| value.as_str() == Some(&run.viewport))
                    {
                        viewports.push(json!(run.viewport));
                    }
                }
                if let (Some(state), Some(states)) =
                    (finding["state"].as_str(), entry["states"].as_array_mut())
                {
                    if !states.iter().any(|value| value.as_str() == Some(state)) {
                        states.push(json!(state));
                    }
                }
            } else {
                let mut entry = finding.clone();
                entry["states"] = json!([finding["state"]]);
                if both_themes {
                    entry["themes"] = json!([run.theme]);
                }
                if sweep {
                    entry["viewports"] = json!([run.viewport]);
                }
                index.insert(key, merged.len());
                merged.push(entry);
            }
        }
    }
    merged
}

fn dedupe_values(values: Vec<Value>) -> Vec<Value> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(value.to_string()))
        .collect()
}

fn sheets_text(sheets: &Value) -> String {
    let mut text = String::new();
    for sheet in sheets["sheets"].as_array().cloned().unwrap_or_default() {
        for rule in sheet["rules"].as_array().cloned().unwrap_or_default() {
            if let Some(value) = rule.as_str() {
                text.push_str(value);
                text.push('\n');
            } else if let Some(value) = rule["text"].as_str() {
                let condition = rule["condition"].as_str();
                if let Some(condition) = condition {
                    text.push_str("@media ");
                    text.push_str(condition);
                    text.push_str(" { ");
                    text.push_str(value);
                    text.push_str(" }\n");
                } else {
                    text.push_str(value);
                    text.push('\n');
                }
            }
            if text.len() > 4_000_000 {
                return text;
            }
        }
    }
    text
}

fn stylesheet_checks(baseline: &Value, text: &str) -> Vec<Value> {
    let mut findings = Vec::new();
    let lower = text.to_lowercase();
    let counts = &baseline["counts"];
    let focusables = counts["focusables"].as_u64().unwrap_or(0);
    let interactive = counts["interactive"].as_u64().unwrap_or(0);
    let form_controls = counts["formControls"].as_u64().unwrap_or(0);
    let text_inputs = counts["textInputs"].as_u64().unwrap_or(0);

    if focusables > 0 && !has_visible_focus_rule(text) {
        findings.push(json!({
            "check": "sheet-focus",
            "severity": "P2",
            "selector": ":root",
            "message": "focusable elements exist but no focus rule declares a visible indicator",
            "evidence": { "focusables": focusables }
        }));
    }
    if interactive > 0 && !lower.contains(":hover") {
        findings.push(json!({
            "check": "sheet-hover",
            "severity": "P3",
            "selector": ":root",
            "message": "interactive elements exist but no hover rule",
            "evidence": { "interactive": interactive }
        }));
    }
    if form_controls > 0 && !(lower.contains(":disabled") || lower.contains("[disabled]")) {
        findings.push(json!({
            "check": "sheet-disabled",
            "severity": "P2",
            "selector": ":root",
            "message": "form controls exist but no disabled rule",
            "evidence": { "formControls": form_controls }
        }));
    }
    if !lower.contains("::selection") {
        findings.push(json!({
            "check": "sheet-selection",
            "severity": "P3",
            "selector": "::selection",
            "message": "no selection style is declared",
            "evidence": {}
        }));
    }
    if text_inputs > 0 && !lower.contains("::placeholder") {
        findings.push(json!({
            "check": "sheet-placeholder",
            "severity": "P3",
            "selector": "::placeholder",
            "message": "text inputs exist but no placeholder style is declared",
            "evidence": { "textInputs": text_inputs }
        }));
    }
    if !lower.contains("-webkit-tap-highlight-color") {
        findings.push(json!({
            "check": "sheet-tap-highlight",
            "severity": "P3",
            "selector": ":root",
            "message": "no -webkit-tap-highlight-color reset is declared",
            "evidence": {}
        }));
    }
    if motion_declared(baseline, text) && !lower.contains("prefers-reduced-motion") {
        findings.push(json!({
            "check": "sheet-reduced-motion",
            "severity": "P2",
            "selector": ":root",
            "message": "motion is declared but no prefers-reduced-motion block exists",
            "evidence": {}
        }));
    }
    findings
}

fn has_visible_focus_rule(text: &str) -> bool {
    let rule = Regex::new(r"(?s)([^{}]*:focus(?:-visible)?[^{}]*)\{([^{}]*)\}").unwrap();
    let declaration = Regex::new(r"(?i)([a-z-]+)\s*:\s*([^;]+)").unwrap();
    for captures in rule.captures_iter(text) {
        let body = &captures[2];
        for property in declaration.captures_iter(body) {
            let name = property[1].to_lowercase();
            if !(name.starts_with("outline")
                || name.starts_with("box-shadow")
                || name.starts_with("border")
                || name.starts_with("background"))
            {
                continue;
            }
            let value = property[2].trim().to_lowercase();
            if value.starts_with("none")
                || value.starts_with("0")
                || value.starts_with("transparent")
                || value.starts_with("inherit")
                || value.starts_with("initial")
                || value.starts_with("unset")
            {
                continue;
            }
            return true;
        }
    }
    false
}

fn split_top_level(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut depth = 0i32;
    for character in raw.chars() {
        match character {
            '(' => {
                depth += 1;
                current.push(character);
            }
            ')' => {
                depth -= 1;
                current.push(character);
            }
            ',' if depth == 0 => {
                let value = current.trim();
                if !value.is_empty() {
                    out.push(value.to_string());
                }
                current.clear();
            }
            _ => current.push(character),
        }
    }
    let value = current.trim();
    if !value.is_empty() {
        out.push(value.to_string());
    }
    out
}

fn motion_declared(baseline: &Value, text: &str) -> bool {
    let lower = text.to_lowercase();
    if lower.contains("@keyframes") || lower.contains("animation:") || lower.contains("transition:")
    {
        return true;
    }
    baseline["elements"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter(|element| element["visible"].as_bool().unwrap_or(false))
        .any(|element| {
            element["animationName"].as_str().unwrap_or("none") != "none"
                || duration_values(element["transitionDuration"].as_str().unwrap_or("0s"))
                    .iter()
                    .any(|value| *value > 0.0)
                || duration_values(element["animationDuration"].as_str().unwrap_or("0s"))
                    .iter()
                    .any(|value| *value > 0.0)
        })
}

pub(crate) fn visible_elements(data: &Value) -> Vec<Value> {
    data["elements"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|element| element["visible"].as_bool().unwrap_or(false))
        .collect()
}

fn duration_values(raw: &str) -> Vec<f64> {
    raw.split(',')
        .filter_map(|value| {
            let value = value.trim();
            if let Some(ms) = value.strip_suffix("ms") {
                ms.parse::<f64>().ok().map(|number| number / 1000.0)
            } else {
                value
                    .strip_suffix('s')
                    .and_then(|seconds| seconds.parse::<f64>().ok())
            }
        })
        .collect()
}

fn motion_evidence(baseline: &Value, text: &str, emit_findings: bool) -> (Value, Vec<Value>) {
    let mut durations: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    let mut timings: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    let mut findings = Vec::new();

    for element in visible_elements(baseline) {
        let selector = element["selector"].as_str().unwrap_or("?");
        for raw in [
            element["transitionDuration"].as_str().unwrap_or(""),
            element["animationDuration"].as_str().unwrap_or(""),
        ] {
            for value in split_top_level(raw) {
                if duration_values(&value).iter().any(|seconds| *seconds > 0.0) {
                    *durations.entry(value).or_insert(0) += 1;
                }
            }
        }
        for (property, raw) in [
            (
                "transitionTimingFunction",
                element["transitionTimingFunction"].as_str().unwrap_or(""),
            ),
            (
                "animationTimingFunction",
                element["animationTimingFunction"].as_str().unwrap_or(""),
            ),
        ] {
            for value in split_top_level(raw) {
                *timings.entry(value.clone()).or_insert(0) += 1;
                if emit_findings && is_overshooting(&value) {
                    findings.push(json!({
                        "check": "bounce-easing",
                        "severity": "P3",
                        "selector": selector,
                        "message": format!("{property} uses an overshooting timing function"),
                        "evidence": { "property": property.strip_prefix('@').unwrap_or(property), "value": value }
                    }));
                }
            }
        }
    }

    let bounce = Regex::new(r"(?i)\b(bounce|elastic)\b").unwrap();
    if emit_findings && bounce.is_match(text) {
        findings.push(json!({
            "check": "bounce-easing",
            "severity": "P3",
            "selector": ":root",
            "message": "stylesheet declares a bounce or elastic timing keyword",
            "evidence": { "sheet": true }
        }));
    }
    let curve = Regex::new(r"cubic-bezier\(([^)]*)\)").unwrap();
    if emit_findings {
        for captures in curve.captures_iter(text) {
            let value = format!("cubic-bezier({})", &captures[1]);
            if is_overshooting(&value) {
                findings.push(json!({
                    "check": "bounce-easing",
                    "severity": "P3",
                    "selector": ":root",
                    "message": "stylesheet declares an overshooting cubic-bezier",
                    "evidence": { "value": value, "sheet": true }
                }));
            }
        }
    }

    let inventory = json!({
        "durations": counts_to_values(durations),
        "timingFunctions": counts_to_values(timings)
    });
    (inventory, findings)
}

fn counts_to_values(counts: std::collections::HashMap<String, u64>) -> Vec<Value> {
    let mut entries: Vec<(String, u64)> = counts.into_iter().collect();
    entries.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(&right.0)));
    entries
        .into_iter()
        .map(|(value, count)| json!({ "value": value, "count": count }))
        .collect()
}

fn is_overshooting(value: &str) -> bool {
    let lower = value.to_lowercase();
    if lower.contains("bounce") || lower.contains("elastic") {
        return true;
    }
    let curve = Regex::new(r"cubic-bezier\(([^)]*)\)").unwrap();
    let Some(captures) = curve.captures(&lower) else {
        return false;
    };
    let numbers: Vec<f64> = captures[1]
        .split(',')
        .filter_map(|part| part.trim().parse::<f64>().ok())
        .collect();
    numbers.len() == 4
        && (numbers[1] < 0.0 || numbers[1] > 1.0 || numbers[3] < 0.0 || numbers[3] > 1.0)
}

fn keyframes(text: &str) -> Vec<Value> {
    let property = Regex::new(r"([a-z-]+)\s*:").unwrap();
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(position) = rest.find("@keyframes") {
        let after = rest[position + "@keyframes".len()..].trim_start();
        let name_end = after
            .find(|character: char| character.is_whitespace() || character == '{')
            .unwrap_or(after.len());
        let name = after[..name_end].trim().to_string();
        let Some(brace) = after.find('{') else {
            rest = &after[name_end..];
            continue;
        };
        let mut depth = 0i32;
        let mut end = None;
        for (offset, character) in after[brace..].char_indices() {
            match character {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(brace + offset);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(end) = end else {
            rest = &after[brace..];
            continue;
        };
        let body = &after[brace + 1..end];
        let mut properties: Vec<String> = Vec::new();
        for captures in property.captures_iter(body) {
            let name = captures[1].to_string();
            if !properties.contains(&name) {
                properties.push(name);
            }
        }
        out.push(json!({ "name": name, "properties": properties }));
        rest = &after[end + 1..];
    }
    out
}

fn with_state(findings: Vec<Value>, state: &str) -> Vec<Value> {
    findings
        .into_iter()
        .map(|mut finding| {
            if let Some(object) = finding.as_object_mut() {
                object.insert("state".to_string(), json!(state));
            }
            finding
        })
        .collect()
}

fn finding_key(finding: &Value) -> String {
    format!(
        "{}|{}",
        finding["check"].as_str().unwrap_or(""),
        finding["selector"].as_str().unwrap_or("")
    )
}

fn is_destructive(name: &str) -> bool {
    let name = name.to_lowercase();
    [
        "delete",
        "remove",
        "pay",
        "buy",
        "send",
        "submit",
        "logout",
        "sign out",
        "unsubscribe",
        "publish",
    ]
    .iter()
    .any(|word| name.contains(word))
}

fn slug(name: &str, _index: usize) -> String {
    let mut out = String::new();
    for character in name.chars() {
        if character.is_ascii_alphanumeric() {
            out.push(character.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "unnamed".to_string()
    } else {
        out
    }
}

fn unverified(data: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    if let Some(elements) = data["elements"].as_array() {
        let unknown: Vec<&Value> = elements
            .iter()
            .filter(|e| {
                e["visible"].as_bool().unwrap_or(false) && e["background"]["unknown"].is_string()
            })
            .collect();
        let images = unknown
            .iter()
            .filter(|e| e["background"]["unknown"].as_str() == Some("background-image"))
            .count();
        if images > 0 {
            out.push(json!({
                "check": "contrast",
                "reason": "background image or gradient behind text",
                "elements": images
            }));
        }
        let conversions = unknown
            .iter()
            .filter(|e| e["background"]["unknown"].as_str() == Some("colour-conversion-failed"))
            .count();
        if conversions > 0 {
            out.push(json!({
                "check": "colour",
                "reason": "background colour conversion failed behind text",
                "elements": conversions
            }));
        }
    }
    if let Some(frames) = data["frames"].as_u64() {
        if frames > 0 {
            out.push(json!({
                "check": "frames",
                "reason": "iframe content is not sampled",
                "frames": frames
            }));
        }
    }
    if let Some(closed) = data["closedShadowSuspects"].as_u64() {
        if closed > 0 {
            out.push(json!({
                "check": "shadow-dom",
                "reason": "closed shadow root content is not sampled",
                "elements": closed
            }));
        }
    }
    out
}

fn focus_state_findings(states: &Value) -> Vec<Value> {
    let mut findings = Vec::new();
    for state in states.as_array().cloned().unwrap_or_default() {
        if state["changed"].as_bool().unwrap_or(true) {
            continue;
        }
        findings.push(json!({
            "check": "focus-invisible",
            "severity": "P2",
            "selector": state["selector"],
            "message": "focusing the element does not visibly change outline, shadow, or border",
            "evidence": {
                "name": state["name"],
                "before": state["before"],
                "after": state["after"]
            }
        }));
    }
    findings
}

fn source_mode(root: &Path, args: &Args) -> Result<i32> {
    if !root.is_dir() {
        anyhow::bail!("source directory does not exist: {}", root.display());
    }
    let baseline_path = resolve_baseline_path(
        args.baseline.as_deref(),
        args.no_baseline,
        args.write_baseline,
        args.update_baseline,
    );
    let baseline = load_baseline(
        baseline_path.as_deref(),
        args.write_baseline || args.update_baseline,
    )?;

    let SourceScan {
        findings,
        suppressed,
        files: scanned,
    } = source::scan(root, args.source_all)?;

    let mut by_check: serde_json::Map<String, Value> = serde_json::Map::new();
    for finding in &findings {
        let name = finding["check"].as_str().unwrap_or("unknown").to_string();
        let count = by_check.get(&name).and_then(Value::as_u64).unwrap_or(0);
        by_check.insert(name, json!(count + 1));
    }

    let mut report = json!({
        "tool": "ui-probe",
        "version": env!("CARGO_PKG_VERSION"),
        "schemaVersion": 1,
        "mode": "source",
        "root": root,
        "baseline": Value::Null,
        "summary": {
            "findings": findings.len(),
            "suppressed": suppressed,
            "files": scanned,
            "byCheck": by_check
        },
        "findings": findings
    });
    apply_source_baseline(&mut report, args, baseline)?;

    emit_report(&report, args.format, args.pretty)?;

    if args.write_baseline || args.update_baseline {
        return Ok(0);
    }

    let has_error = report["findings"]
        .as_array()
        .map(|list| {
            list.iter().any(|finding| {
                matches!(
                    finding["severity"].as_str(),
                    Some("P0" | "P1" | "P2" | "error")
                )
            })
        })
        .unwrap_or(false);
    Ok(if has_error { 2 } else { 0 })
}

fn contrast_ratio(element: &Value) -> Option<f64> {
    let (fr, fg, fb, fa) = parse_css_color(element["color"].as_str()?)?;
    let background = element["background"].as_object()?;
    let br = background.get("r")?.as_f64()?;
    let bg = background.get("g")?.as_f64()?;
    let bb = background.get("b")?.as_f64()?;

    let fg_composited = (
        fr * fa + br * (1.0 - fa),
        fg * fa + bg * (1.0 - fa),
        fb * fa + bb * (1.0 - fa),
    );
    let foreground = luminance(fg_composited.0, fg_composited.1, fg_composited.2);
    let backdrop = luminance(br, bg, bb);
    let (lighter, darker) = if foreground > backdrop {
        (foreground, backdrop)
    } else {
        (backdrop, foreground)
    };
    Some((lighter + 0.05) / (darker + 0.05))
}

pub(crate) fn parse_css_color(css: &str) -> Option<(f64, f64, f64, f64)> {
    let css = css.trim();
    let inner = css
        .strip_prefix("rgba(")
        .or_else(|| css.strip_prefix("rgb("))?
        .strip_suffix(')')?;
    let parts: Vec<f64> = inner
        .split([',', '/', ' '])
        .filter(|part| !part.trim().is_empty())
        .filter_map(|part| part.trim().parse::<f64>().ok())
        .collect();
    if parts.len() < 3 {
        return None;
    }
    Some((parts[0], parts[1], parts[2], *parts.get(3).unwrap_or(&1.0)))
}

pub(crate) fn luminance(r: f64, g: f64, b: f64) -> f64 {
    0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
}

fn channel(value: f64) -> f64 {
    let value = value / 255.0;
    if value <= 0.03928 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}
