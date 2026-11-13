use anyhow::Result;
use serde_json::{json, Value};
use std::collections::HashSet;

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum OutputFormat {
    Json,
    Sarif,
    Markdown,
    Github,
}

pub(crate) fn severity_level(finding: &Value) -> &'static str {
    match finding["severity"].as_str().unwrap_or("P3") {
        "P0" | "P1" | "error" => "error",
        "P2" | "warning" => "warning",
        _ => "note",
    }
}

pub(crate) fn sarif_report(report: &Value) -> Value {
    let findings = report["findings"].as_array().cloned().unwrap_or_default();
    let mut rules = Vec::new();
    let mut seen = HashSet::new();
    for finding in &findings {
        let id = finding["check"].as_str().unwrap_or("unknown");
        if !seen.insert(id.to_string()) {
            continue;
        }
        let message = finding["message"].as_str().unwrap_or(id);
        rules.push(json!({
            "id": id,
            "name": id,
            "shortDescription": { "text": message },
            "defaultConfiguration": { "level": severity_level(finding) },
            "properties": { "layer": finding["layer"] }
        }));
    }
    let results: Vec<Value> = findings
        .iter()
        .map(|finding| {
            let mut result = json!({
                "ruleId": finding["check"],
                "level": severity_level(finding),
                "message": { "text": finding["message"] },
                "partialFingerprints": {
                    "uiProbeFingerprint": finding["fingerprint"]
                },
                "properties": {
                    "state": finding["state"],
                    "states": finding["states"],
                    "themes": finding["themes"],
                    "viewports": finding["viewports"],
                    "ruleSource": finding["ruleSource"],
                    "selector": finding["selector"]
                }
            });
            if let Some(file) = finding["file"].as_str() {
                result["locations"] = json!([{
                    "physicalLocation": {
                        "artifactLocation": { "uri": file },
                        "region": { "startLine": finding["line"].as_u64().unwrap_or(1) }
                    }
                }]);
            } else if let Some(selector) = finding["selector"].as_str() {
                result["locations"] = json!([{
                    "logicalLocations": [{ "name": selector, "kind": "element" }]
                }]);
            }
            result
        })
        .collect();
    json!({
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "version": "2.1.0",
        "runs": [{
            "tool": {
                "driver": {
                    "name": "ui-probe",
                    "version": report["version"],
                    "informationUri": "https://github.com/hollowheap/opal",
                    "rules": rules
                }
            },
            "results": results,
            "properties": {
                "schemaVersion": report["schemaVersion"],
                "sources": report["sources"],
                "baseline": report["baseline"]
            }
        }]
    })
}

pub(crate) fn escape_markdown(value: &str) -> String {
    value.replace('|', "\\|").replace('\n', " ")
}

pub(crate) fn markdown_report(report: &Value) -> String {
    let mut out = String::new();
    let title = report["title"]
        .as_str()
        .or_else(|| report["root"].as_str())
        .unwrap_or("ui-probe");
    out.push_str(&format!("# ui-probe: {title}\n\n"));
    if let Some(url) = report["url"].as_str() {
        out.push_str(&format!("- URL: {url}\n"));
    }
    let findings = report["findings"].as_array().cloned().unwrap_or_default();
    out.push_str(&format!("- Findings: {}\n", findings.len()));
    if let Some(suppressed) = report["baseline"]["suppressed"].as_u64() {
        out.push_str(&format!("- Baseline suppressed: {suppressed}\n"));
    }
    if let Some(unverified) = report["unverified"].as_array() {
        out.push_str(&format!("- Unverified: {}\n", unverified.len()));
    }
    out.push('\n');
    if findings.is_empty() {
        out.push_str("No findings.\n");
        return out;
    }
    out.push_str("| Severity | Check | Location | Message |\n");
    out.push_str("|---|---|---|---|\n");
    for finding in &findings {
        let severity = finding["severity"].as_str().unwrap_or("advisory");
        let check = finding["check"].as_str().unwrap_or("unknown");
        let location = finding["file"]
            .as_str()
            .map(|file| format!("{}:{}", file, finding["line"].as_u64().unwrap_or(1)))
            .or_else(|| finding["selector"].as_str().map(str::to_string))
            .unwrap_or_else(|| "-".to_string());
        let message = escape_markdown(finding["message"].as_str().unwrap_or(""));
        out.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            severity,
            check,
            escape_markdown(&location),
            message
        ));
    }
    out
}

pub(crate) fn escape_github_message(value: &str) -> String {
    value
        .replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

pub(crate) fn escape_github_property(value: &str) -> String {
    escape_github_message(value)
        .replace(':', "%3A")
        .replace(',', "%2C")
}

pub(crate) fn github_report(report: &Value) -> String {
    let mut out = String::new();
    for finding in report["findings"].as_array().cloned().unwrap_or_default() {
        let level = severity_level(&finding);
        let check = finding["check"].as_str().unwrap_or("ui-probe");
        let message = escape_github_message(finding["message"].as_str().unwrap_or(""));
        if let Some(file) = finding["file"].as_str() {
            out.push_str(&format!(
                "::{} file={},line={},title={}::{}\n",
                level,
                escape_github_property(file),
                finding["line"].as_u64().unwrap_or(1),
                escape_github_property(check),
                message
            ));
        } else {
            let selector = finding["selector"].as_str().unwrap_or("");
            out.push_str(&format!(
                "::{} title={}::{} {}\n",
                level,
                escape_github_property(check),
                message,
                escape_github_message(selector)
            ));
        }
    }
    out
}

pub(crate) fn emit_report(report: &Value, format: OutputFormat, pretty: bool) -> Result<()> {
    match format {
        OutputFormat::Json => {
            if pretty {
                println!("{}", serde_json::to_string_pretty(report)?);
            } else {
                println!("{}", serde_json::to_string(report)?);
            }
        }
        OutputFormat::Sarif => {
            let sarif = sarif_report(report);
            if pretty {
                println!("{}", serde_json::to_string_pretty(&sarif)?);
            } else {
                println!("{}", serde_json::to_string(&sarif)?);
            }
        }
        OutputFormat::Markdown => print!("{}", markdown_report(report)),
        OutputFormat::Github => print!("{}", github_report(report)),
    }
    Ok(())
}
