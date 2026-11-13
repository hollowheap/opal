use crate::{
    font_families, generic_family, is_overshooting, oracle_from_value, parse_color_rgb,
    parse_css_color, parse_dimension, radius_allowed, DesignSpec, Oracle,
};
use anyhow::{Context, Result};
use regex::Regex;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub(crate) struct RuleSet {
    structural: Vec<StructuralRule>,
    checks: Vec<CheckRule>,
    selectors: Vec<String>,
    properties: Vec<String>,
    coverage: Vec<Value>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Logic {
    All,
    Any,
    One,
}

pub(crate) struct StructuralRule {
    name: String,
    source: String,
    ancestors: Vec<Constraint>,
    descendants: Vec<Constraint>,
    selectors: Vec<String>,
    operator: Logic,
    match_operator: Logic,
    exceptions: Vec<String>,
    properties: Vec<(String, PropertyRule)>,
    when: Option<Value>,
    message: Option<String>,
    params: Value,
    cardinality: Option<Cardinality>,
    scope: String,
    lifecycle: Option<String>,
    legacy: bool,
    severity: String,
    layer: String,
}

pub(crate) struct Constraint {
    any: bool,
    selector: String,
}

pub(crate) struct Cardinality {
    minimum: Option<u64>,
    maximum: Option<u64>,
    region: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) enum Verb {
    In(Value),
    NotIn(Value),
    Equals(Value),
    NotEquals(Value),
    Present,
    Absent,
}

#[derive(Clone, Debug)]
pub(crate) struct PropertyTest {
    verb: Verb,
}

#[derive(Clone, Debug)]
pub(crate) enum PropertyRule {
    Deny(Vec<String>),
    AllowFrom(String),
    Tests(Vec<PropertyTest>),
    Components(HashMap<String, Vec<PropertyTest>>),
}

pub(crate) struct CheckRule {
    id: String,
    output: String,
    source: String,
    enabled: bool,
    severity: Option<String>,
    layer: Option<String>,
}

impl RuleSet {
    pub(crate) fn selectors(&self) -> &[String] {
        &self.selectors
    }

    pub(crate) fn properties(&self) -> &[String] {
        &self.properties
    }

    pub(crate) fn coverage(&self) -> &[Value] {
        &self.coverage
    }

    pub(crate) fn structural_len(&self) -> usize {
        self.structural.len()
    }

    pub(crate) fn declared_names(&self) -> HashSet<&str> {
        self.structural
            .iter()
            .map(|rule| rule.name.as_str())
            .collect()
    }

    pub(crate) fn needs_first_paint(&self) -> bool {
        self.structural
            .iter()
            .any(|rule| rule.lifecycle.as_deref().is_some_and(is_first_paint))
    }

    pub(crate) fn disabled_checks(&self) -> Vec<String> {
        self.checks
            .iter()
            .filter(|rule| !rule.enabled)
            .map(|rule| rule.id.clone())
            .collect()
    }
}

pub(crate) const CSS_PROPERTIES: &[&str] = &[
    "align-items",
    "animation-duration",
    "animation-name",
    "animation-timing-function",
    "background",
    "background-clip",
    "background-color",
    "background-image",
    "border",
    "border-bottom-color",
    "border-bottom-left-radius",
    "border-bottom-right-radius",
    "border-bottom-style",
    "border-bottom-width",
    "border-color",
    "border-left-color",
    "border-left-style",
    "border-left-width",
    "border-radius",
    "border-right-color",
    "border-right-style",
    "border-right-width",
    "border-style",
    "border-top-color",
    "border-top-left-radius",
    "border-top-right-radius",
    "border-top-style",
    "border-top-width",
    "border-width",
    "box-shadow",
    "color",
    "column-gap",
    "cursor",
    "display",
    "font-family",
    "font-size",
    "font-style",
    "font-weight",
    "gap",
    "height",
    "justify-content",
    "letter-spacing",
    "line-height",
    "margin",
    "margin-bottom",
    "margin-left",
    "margin-right",
    "margin-top",
    "max-height",
    "max-width",
    "min-height",
    "min-width",
    "opacity",
    "outline",
    "outline-color",
    "outline-offset",
    "outline-style",
    "outline-width",
    "overflow",
    "overflow-x",
    "overflow-y",
    "padding",
    "padding-bottom",
    "padding-left",
    "padding-right",
    "padding-top",
    "row-gap",
    "text-align",
    "text-decoration",
    "text-overflow",
    "text-transform",
    "transition-duration",
    "transition-timing-function",
    "visibility",
    "white-space",
    "width",
    "z-index",
];

pub(crate) fn check_keys(value: &Value, allowed: &[&str], context: &str) -> Result<()> {
    if let Some(object) = value.as_object() {
        for key in object.keys() {
            if !allowed.contains(&key.as_str()) {
                anyhow::bail!("rules: unknown key {key} in {context}");
            }
        }
    }
    Ok(())
}

pub(crate) fn resolve_param(design: Option<&DesignSpec>, path: &str) -> Option<Value> {
    let design = design?;
    let mut current = &design.params;
    for part in path.split('.') {
        current = current.get(part)?;
    }
    if let Some(value) = current.get("$value") {
        current = value;
    }
    Some(current.clone())
}

pub(crate) fn dangling_param(value: &Value) -> Option<String> {
    match value {
        Value::String(raw) => {
            let path = raw.strip_prefix("param:")?;
            if path.trim().is_empty() {
                Some(raw.clone())
            } else {
                None
            }
        }
        Value::Array(values) => values.iter().find_map(dangling_param),
        Value::Object(object) => object.values().find_map(dangling_param),
        _ => None,
    }
}

pub(crate) fn resolve_params(
    value: Value,
    design: Option<&DesignSpec>,
    rule: &str,
    notes: &mut Vec<Value>,
) -> Value {
    match value {
        Value::String(raw) => {
            if let Some(path) = raw.strip_prefix("param:") {
                let path = path.trim();
                if path.is_empty() {
                    return Value::String(raw);
                }
                match resolve_param(design, path) {
                    Some(value) => value,
                    None => {
                        notes.push(json!({
                            "check": rule,
                            "reason": format!("missing param:{path}"),
                            "rule": rule
                        }));
                        Value::String(raw)
                    }
                }
            } else {
                Value::String(raw)
            }
        }
        Value::Array(values) => Value::Array(
            values
                .into_iter()
                .map(|value| resolve_params(value, design, rule, notes))
                .collect(),
        ),
        Value::Object(object) => Value::Object(
            object
                .into_iter()
                .map(|(key, value)| (key, resolve_params(value, design, rule, notes)))
                .collect(),
        ),
        other => other,
    }
}

pub(crate) fn read_rules(path: &Path) -> Result<Value> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("read rules file {}", path.display()))?;
    let data: Value = serde_json::from_str(&text)
        .with_context(|| format!("parse rules file {}", path.display()))?;
    check_keys(
        &data,
        &[
            "$schema",
            "version",
            "pack",
            "structural",
            "rules",
            "extends",
            "aliases",
            "alias",
            "kinds",
            "kindAliases",
            "classAliases",
            "ruleAliases",
            "tokenSets",
            "catalogue",
            "catalogs",
            "description",
        ],
        "rules file",
    )?;
    let version = data["version"]
        .as_u64()
        .context("rules: version must be a number")?;
    if version != 1 && version != 2 {
        anyhow::bail!("rules: unsupported version {version}; expected 1 or 2");
    }
    Ok(data)
}

pub(crate) fn extends_paths(path: &Path, data: &Value) -> Result<Vec<PathBuf>> {
    let mut raw = Vec::new();
    match &data["extends"] {
        Value::Null => {}
        Value::String(value) => raw.push(value.clone()),
        Value::Array(values) => {
            for value in values {
                raw.push(
                    value
                        .as_str()
                        .context("rules: extends must contain strings")?
                        .to_string(),
                );
            }
        }
        _ => anyhow::bail!("rules: extends must be a string or an array"),
    }
    Ok(raw
        .into_iter()
        .map(|value| {
            let candidate = PathBuf::from(&value);
            if candidate.is_absolute() {
                candidate
            } else {
                path.parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join(candidate)
            }
        })
        .collect())
}

pub(crate) fn expand_rules_source(
    path: &Path,
    origin: &'static str,
    out: &mut Vec<(PathBuf, &'static str)>,
    seen: &mut HashSet<PathBuf>,
    stack: &mut Vec<PathBuf>,
) -> Result<()> {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if stack.contains(&canonical) {
        anyhow::bail!("rules: extends cycle at {}", path.display());
    }
    if seen.contains(&canonical) {
        return Ok(());
    }
    let data = read_rules(path)?;
    let bases = extends_paths(path, &data)?;
    seen.insert(canonical.clone());
    out.push((path.to_path_buf(), origin));
    stack.push(canonical);
    for base in bases {
        expand_rules_source(&base, "extended", out, seen, stack)?;
    }
    stack.pop();
    Ok(())
}

pub(crate) fn expand_rules_sources(
    sources: &[(PathBuf, &'static str)],
) -> Result<Vec<(PathBuf, &'static str)>> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut stack = Vec::new();
    for (path, origin) in sources {
        expand_rules_source(path, origin, &mut out, &mut seen, &mut stack)?;
    }
    Ok(out)
}

pub(crate) fn merge_rules(sources: &[PathBuf]) -> Result<Option<Value>> {
    if sources.is_empty() {
        return Ok(None);
    }
    let mut pack = None;
    let mut version = 1u64;
    let mut structural = Vec::new();
    let mut named = Vec::new();
    let mut aliases = serde_json::Map::new();
    let mut rule_aliases = serde_json::Map::new();
    let mut seen_ids: HashSet<String> = HashSet::new();
    for source in sources {
        let data = read_rules(source)?;
        let label = source.display().to_string();
        version = version.max(data["version"].as_u64().unwrap_or(1));
        if pack.is_none() {
            pack = data["pack"].as_str().map(str::to_string);
        }
        for key in ["aliases", "alias", "kinds", "kindAliases", "classAliases"] {
            if let Some(object) = data[key].as_object() {
                for (name, value) in object {
                    aliases.insert(name.clone(), value.clone());
                }
            }
        }
        for key in ["ruleAliases"] {
            if let Some(object) = data[key].as_object() {
                for (name, value) in object {
                    rule_aliases.insert(name.clone(), value.clone());
                }
            }
        }
        for mut rule in data["structural"].as_array().cloned().unwrap_or_default() {
            if let Some(object) = rule.as_object_mut() {
                object.insert("_source".to_string(), json!(label));
            }
            structural.push(rule);
        }
        for mut rule in data["rules"].as_array().cloned().unwrap_or_default() {
            let identity = rule["id"]
                .as_str()
                .or_else(|| rule["name"].as_str())
                .unwrap_or("")
                .to_string();
            let declarative = rule.get("selector").is_some()
                || rule.get("selectors").is_some()
                || rule.get("properties").is_some()
                || rule.get("tests").is_some()
                || rule.get("attributes").is_some()
                || rule.get("when").is_some()
                || rule.get("condition").is_some()
                || rule.get("expr").is_some();
            let duplicate =
                !declarative && !identity.is_empty() && !seen_ids.insert(identity.clone());
            if duplicate {
                continue;
            }
            if let Some(object) = rule.as_object_mut() {
                object.insert("_source".to_string(), json!(label));
            }
            named.push(rule);
        }
    }
    let mut merged = json!({
        "version": version,
        "pack": pack.unwrap_or_else(|| "custom".to_string()),
        "structural": structural,
        "rules": named
    });
    if !aliases.is_empty() {
        merged["aliases"] = Value::Object(aliases);
    }
    if !rule_aliases.is_empty() {
        merged["ruleAliases"] = Value::Object(rule_aliases);
    }
    Ok(Some(merged))
}

pub(crate) fn load_rules(
    sources: &[PathBuf],
    design: Option<&DesignSpec>,
) -> Result<(RuleSet, Vec<Value>)> {
    load_rules_with_default(sources, design)
}

fn default_pack_data() -> Result<Value> {
    let relative = Path::new("skills/opal/references/default-rules.json");
    let mut candidates = Vec::new();
    if let Ok(path) = std::env::var("UI_PROBE_DEFAULT_RULES") {
        candidates.push(PathBuf::from(path));
    }
    candidates.push(relative.to_path_buf());
    if let Ok(executable) = std::env::current_exe() {
        let mut directory = executable.parent();
        for _ in 0..6 {
            let Some(current) = directory else {
                break;
            };
            candidates.push(current.join(relative));
            directory = current.parent();
        }
    }
    let path = candidates
        .into_iter()
        .find(|path| path.is_file())
        .context("read default rules file: no default pack found")?;
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("read default rules file {}", path.display()))?;
    let mut data: Value = serde_json::from_str(&text).context("parse default rules")?;
    for key in ["rules", "structural"] {
        if let Some(rules) = data[key].as_array_mut() {
            for rule in rules {
                if let Some(object) = rule.as_object_mut() {
                    object.insert("_source".to_string(), json!("default-rules.json"));
                }
            }
        }
    }
    Ok(data)
}

pub(crate) fn load_default_rules(design: Option<&DesignSpec>) -> Result<(RuleSet, Vec<Value>)> {
    parse_rules(&default_pack_data()?, design)
}

fn rule_identity(rule: &Value) -> String {
    rule["id"]
        .as_str()
        .or_else(|| rule["name"].as_str())
        .unwrap_or("")
        .to_string()
}

fn declarative_rule(rule: &Value) -> bool {
    rule.get("selector").is_some()
        || rule.get("selectors").is_some()
        || rule.get("properties").is_some()
        || rule.get("tests").is_some()
        || rule.get("attributes").is_some()
        || rule.get("when").is_some()
        || rule.get("condition").is_some()
        || rule.get("expr").is_some()
        || rule.get("cardinality").is_some()
        || rule.get("constraints").is_some()
        || rule.get("scope").is_some()
}

pub(crate) fn load_rules_with_default(
    sources: &[PathBuf],
    design: Option<&DesignSpec>,
) -> Result<(RuleSet, Vec<Value>)> {
    let Some(custom) = merge_rules(sources)? else {
        return load_default_rules(design);
    };
    let mut default = default_pack_data()?;
    let custom_version = custom["version"].as_u64().unwrap_or(1);
    let mut default_rules = default["rules"].as_array().cloned().unwrap_or_default();
    let mut indexes: HashMap<String, usize> = HashMap::new();
    for (index, rule) in default_rules.iter().enumerate() {
        let identity = rule_identity(rule);
        if !identity.is_empty() {
            indexes.insert(identity, index);
        }
    }
    let mut extra_rules = Vec::new();
    for rule in custom["rules"].as_array().cloned().unwrap_or_default() {
        let identity = rule_identity(&rule);
        if let Some(index) = indexes.get(&identity).copied() {
            if declarative_rule(&rule) {
                default_rules[index] = rule;
                continue;
            }
            let Some(target) = default_rules[index].as_object_mut() else {
                continue;
            };
            if rule["enabled"].as_bool() == Some(false) {
                target.insert("enabled".to_string(), json!(false));
            }
            for key in ["severity", "layer"] {
                if let Some(value) = rule.get(key) {
                    target.insert(key.to_string(), value.clone());
                }
            }
            if let Some(params) = rule.get("params") {
                target.insert("params".to_string(), params.clone());
            }
            if let Some(source) = rule.get("_source") {
                target.insert("_source".to_string(), source.clone());
            }
        } else {
            if custom_version == 1 && !declarative_rule(&rule) {
                anyhow::bail!("rules: unknown rule id {identity}");
            }
            extra_rules.push(rule);
        }
    }
    default_rules.extend(extra_rules);
    default["rules"] = Value::Array(default_rules);
    let mut structural = default["structural"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    structural.extend(custom["structural"].as_array().cloned().unwrap_or_default());
    default["structural"] = Value::Array(structural);
    for key in ["aliases", "alias", "kinds", "kindAliases", "classAliases"] {
        if let Some(custom_value) = custom.get(key).cloned() {
            match (default.get_mut(key), custom_value) {
                (Some(Value::Object(base)), Value::Object(overlay)) => {
                    for (name, value) in overlay {
                        base.insert(name, value);
                    }
                }
                (Some(Value::Array(base)), Value::Array(overlay)) => {
                    base.extend(overlay);
                }
                (_, value) => {
                    default[key] = value;
                }
            }
        }
    }
    if let Some(Value::Object(base)) = default.get_mut("ruleAliases") {
        if let Some(Value::Object(overlay)) = custom.get("ruleAliases") {
            for (name, value) in overlay {
                base.insert(name.clone(), value.clone());
            }
        }
    } else if let Some(value) = custom.get("ruleAliases") {
        default["ruleAliases"] = value.clone();
    }
    default["version"] = json!(2);
    parse_rules(&default, design)
}

pub(crate) fn parse_rules(
    data: &Value,
    design: Option<&DesignSpec>,
) -> Result<(RuleSet, Vec<Value>)> {
    let version = data["version"].as_u64().unwrap_or(1);
    if version >= 2 {
        parse_rules_v2(data, design)
    } else {
        anyhow::bail!("rules: version {version} is legacy and must be loaded through the compatibility overlay")
    }
}

pub(crate) fn parse_logic(value: &Value, fallback: Logic) -> Logic {
    match value.as_str().unwrap_or("") {
        "all" | "and" | "every" => Logic::All,
        "any" | "or" | "some" => Logic::Any,
        "one" | "only-one" | "only_one" | "onlyOne" | "exactly-one" | "exactly_one"
        | "exactlyOne" | "exactly one" => Logic::One,
        _ => fallback,
    }
}

pub(crate) fn parse_aliases(data: &Value) -> HashMap<String, Vec<String>> {
    let mut groups: Vec<Vec<String>> = Vec::new();
    let mut add_group = |value: &Value| {
        let mut members = Vec::new();
        match value {
            Value::String(raw) => members.push(raw.clone()),
            Value::Array(values) => {
                for value in values {
                    if let Some(raw) = value.as_str() {
                        members.push(raw.to_string());
                    }
                }
            }
            Value::Object(object) => {
                for key in ["class", "classes", "aliases", "names", "members"] {
                    if let Some(values) = object.get(key) {
                        if let Value::Array(values) = values {
                            for value in values {
                                if let Some(raw) = value.as_str() {
                                    members.push(raw.to_string());
                                }
                            }
                        } else if let Some(raw) = values.as_str() {
                            members.push(raw.to_string());
                        }
                    }
                }
            }
            _ => {}
        }
        if !members.is_empty() {
            groups.push(members);
        }
    };
    for key in ["aliases", "alias", "kinds", "kindAliases", "classAliases"] {
        match &data[key] {
            Value::Object(object) => {
                for (name, value) in object {
                    let mut group = vec![name.clone()];
                    match value {
                        Value::String(raw) => group.push(raw.clone()),
                        Value::Array(values) => {
                            for value in values {
                                if let Some(raw) = value.as_str() {
                                    group.push(raw.to_string());
                                }
                            }
                        }
                        Value::Object(object) => {
                            for key in ["class", "classes", "aliases", "names", "members"] {
                                if let Some(values) = object.get(key) {
                                    if let Value::Array(values) = values {
                                        for value in values {
                                            if let Some(raw) = value.as_str() {
                                                group.push(raw.to_string());
                                            }
                                        }
                                    } else if let Some(raw) = values.as_str() {
                                        group.push(raw.to_string());
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                    add_group(&json!(group));
                }
            }
            Value::Array(values) => {
                if values.iter().all(Value::is_string) {
                    add_group(&Value::Array(values.clone()));
                }
                for value in values {
                    match value {
                        Value::Array(_) => add_group(value),
                        Value::Object(object) => {
                            let mut group = Vec::new();
                            for key in ["kind", "name", "class"] {
                                if let Some(raw) = object.get(key).and_then(Value::as_str) {
                                    group.push(raw.to_string());
                                }
                            }
                            for key in ["class", "classes", "aliases", "names", "members"] {
                                if let Some(values) = object.get(key).and_then(Value::as_array) {
                                    for value in values {
                                        if let Some(raw) = value.as_str() {
                                            group.push(raw.to_string());
                                        }
                                    }
                                }
                            }
                            if !group.is_empty() {
                                add_group(&json!(group));
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    let mut aliases: HashMap<String, Vec<String>> = HashMap::new();
    for group in groups {
        let mut unique: Vec<String> = Vec::new();
        for member in group {
            let member = member.trim_start_matches('.').to_string();
            if !member.is_empty() && !unique.contains(&member) {
                unique.push(member);
            }
        }
        if unique.len() < 2 {
            continue;
        }
        for member in &unique {
            aliases.insert(member.clone(), unique.clone());
        }
    }
    aliases
}

pub(crate) fn expand_kind_aliases(
    selector: &str,
    aliases: &HashMap<String, Vec<String>>,
) -> String {
    if aliases.is_empty() {
        return selector.to_string();
    }
    let chars: Vec<char> = selector.chars().collect();
    let mut out = String::new();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '.'
            && index + 1 < chars.len()
            && (chars[index + 1].is_alphabetic()
                || chars[index + 1] == '_'
                || chars[index + 1] == '-')
        {
            let start = index + 1;
            let mut end = start;
            while end < chars.len()
                && (chars[end].is_alphanumeric() || chars[end] == '_' || chars[end] == '-')
            {
                end += 1;
            }
            let name: String = chars[start..end].iter().collect();
            if let Some(group) = aliases.get(&name) {
                out.push_str(":is(");
                for (position, member) in group.iter().enumerate() {
                    if position > 0 {
                        out.push(',');
                    }
                    out.push('.');
                    out.push_str(member);
                }
                out.push(')');
                index = end;
                continue;
            }
            out.push('.');
            out.push_str(&name);
            index = end;
            continue;
        }
        out.push(chars[index]);
        index += 1;
    }
    out
}

pub(crate) fn add_selector(selectors: &mut Vec<String>, selector: String) {
    if !selector.trim().is_empty() && !selectors.contains(&selector) {
        selectors.push(selector);
    }
}

pub(crate) fn selector_from_value(value: &Value) -> Option<String> {
    value.as_str().map(str::to_string).or_else(|| {
        value
            .get("selector")
            .and_then(Value::as_str)
            .map(str::to_string)
    })
}

pub(crate) fn parse_constraints(
    value: &Value,
    aliases: &HashMap<String, Vec<String>>,
) -> Vec<Constraint> {
    let mut out = Vec::new();
    let values: Vec<Value> = match value {
        Value::Array(values) => values.clone(),
        Value::Null => Vec::new(),
        other => vec![other.clone()],
    };
    for entry in values {
        if let Some(selector) = entry.as_str() {
            out.push(Constraint {
                any: true,
                selector: expand_kind_aliases(selector, aliases),
            });
            continue;
        }
        let mut any = true;
        let selector = if let Some(raw) = entry.get("any").and_then(Value::as_str) {
            raw.to_string()
        } else if let Some(raw) = entry.get("none").and_then(Value::as_str) {
            any = false;
            raw.to_string()
        } else if entry.get("exists").and_then(Value::as_bool) == Some(false)
            || entry.get("present").and_then(Value::as_bool) == Some(false)
            || entry.get("not").and_then(Value::as_bool) == Some(true)
        {
            any = false;
            entry
                .get("selector")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        } else {
            entry
                .get("selector")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        if !selector.trim().is_empty() {
            out.push(Constraint {
                any,
                selector: expand_kind_aliases(&selector, aliases),
            });
        }
    }
    out
}

pub(crate) fn parse_selector_list(
    value: &Value,
    aliases: &HashMap<String, Vec<String>>,
) -> Vec<String> {
    let mut out = Vec::new();
    let values: Vec<Value> = match value {
        Value::Array(values) => values.clone(),
        Value::Null => Vec::new(),
        Value::Object(object) => object
            .get("selectors")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_else(|| vec![value.clone()]),
        other => vec![other.clone()],
    };
    for entry in values {
        if let Some(selector) = selector_from_value(&entry) {
            add_selector(&mut out, expand_kind_aliases(&selector, aliases));
        }
    }
    out
}

pub(crate) fn parse_verb(value: &Value, context: &str) -> Result<Vec<PropertyTest>> {
    if let Some(raw) = value.as_str() {
        return Ok(vec![PropertyTest {
            verb: Verb::Equals(json!(raw)),
        }]);
    }
    let Some(object) = value.as_object() else {
        anyhow::bail!("rules: {context} needs a verb object");
    };
    if let Some(name) = object.get("verb").and_then(Value::as_str) {
        let operand = object
            .get("operand")
            .or_else(|| object.get("value"))
            .cloned()
            .unwrap_or(Value::Null);
        let synthesized = json!({ name: operand });
        return parse_verb(&synthesized, context);
    }
    let mut tests = Vec::new();
    for (key, operand) in object {
        match key.as_str() {
            "in" | "allowFrom" | "oneOf" | "member" | "members" | "list" => {
                tests.push(PropertyTest {
                    verb: Verb::In(operand.clone()),
                });
            }
            "notIn" | "not_in" | "outside" | "noneOf" => {
                tests.push(PropertyTest {
                    verb: Verb::NotIn(operand.clone()),
                });
            }
            "allowed" | "allowedValues" | "allowed_values" | "allow" | "allowList" | "values" => {
                tests.push(PropertyTest {
                    verb: Verb::NotIn(operand.clone()),
                });
            }
            "equals" | "eq" | "is" => {
                tests.push(PropertyTest {
                    verb: Verb::Equals(operand.clone()),
                });
            }
            "notEquals" | "not_equals" | "ne" | "isNot" => {
                tests.push(PropertyTest {
                    verb: Verb::NotEquals(operand.clone()),
                });
            }
            "present" => {
                if operand.as_bool() == Some(false) {
                    tests.push(PropertyTest { verb: Verb::Absent });
                } else {
                    tests.push(PropertyTest {
                        verb: Verb::Present,
                    });
                }
            }
            "absent" => {
                if operand.as_bool() == Some(false) {
                    tests.push(PropertyTest {
                        verb: Verb::Present,
                    });
                } else {
                    tests.push(PropertyTest { verb: Verb::Absent });
                }
            }
            "deny" => {
                if let Some(values) = operand.as_array() {
                    for item in values {
                        tests.push(PropertyTest {
                            verb: Verb::NotEquals(item.clone()),
                        });
                    }
                } else {
                    tests.push(PropertyTest {
                        verb: Verb::NotEquals(operand.clone()),
                    });
                }
            }
            "tests" | "constraints" | "all" | "any" | "one" => {
                if let Some(list) = operand.as_array() {
                    for item in list {
                        tests.extend(parse_verb(item, context)?);
                    }
                } else {
                    tests.extend(parse_verb(operand, context)?);
                }
            }
            "operator" | "components" | "component" | "property" | "attribute" | "selector" => {}
            other => anyhow::bail!("rules: unknown verb {other} in {context}"),
        }
    }
    if tests.is_empty() {
        anyhow::bail!("rules: {context} carries no verb");
    }
    Ok(tests)
}

pub(crate) fn note_missing_sets(
    tests: &[PropertyTest],
    design: Option<&DesignSpec>,
    notes: &mut Vec<Value>,
    rule: &str,
    source: &str,
) {
    let mut seen = HashSet::new();
    for test in tests {
        let set = match &test.verb {
            Verb::In(Value::String(set)) | Verb::NotIn(Value::String(set)) => Some(set.as_str()),
            _ => None,
        };
        let Some(set) = set else {
            continue;
        };
        if !seen.insert(set.to_string()) {
            continue;
        }
        let standard = matches!(
            set,
            "colors"
                | "color"
                | "palette"
                | "colour"
                | "fonts"
                | "font"
                | "font-family"
                | "spacing"
                | "space"
                | "dimension"
                | "radii"
                | "radius"
                | "typography"
                | "type"
                | "font-size"
        );
        if design.is_none() {
            notes.push(json!({
                "check": rule,
                "reason": "missing design parameters",
                "rule": rule,
                "source": set,
                "ruleSource": source,
                "packSource": source
            }));
        } else {
            let available = design
                .map(|design| {
                    if standard {
                        match set {
                            "colors" | "color" | "palette" | "colour" => design.colors.is_some(),
                            "fonts" | "font" | "font-family" => design.fonts.is_some(),
                            "spacing" | "space" | "dimension" => design.spacing.is_some(),
                            "radii" | "radius" => design.radii.is_some(),
                            "typography" | "type" | "font-size" => design.typography.is_some(),
                            _ => false,
                        }
                    } else {
                        resolve_param(Some(design), set).is_some()
                    }
                })
                .unwrap_or(false);
            if available {
                continue;
            }
            notes.push(json!({
                "check": rule,
                "reason": "coverage gap: token set not present in design file",
                "coverageGap": true,
                "rule": rule,
                "source": set,
                "ruleSource": source,
                "packSource": source
            }));
        }
    }
}

pub(crate) fn parse_property_rule(
    property: &str,
    value: &Value,
    design: Option<&DesignSpec>,
    notes: &mut Vec<Value>,
    rule: &str,
    source: &str,
) -> Result<Option<PropertyRule>> {
    if let Some(object) = value.as_object() {
        if object.contains_key("deny") || object.contains_key("allowFrom") {
            check_keys(value, &["deny", "allowFrom"], "property constraint")?;
            if object.get("deny").is_some() == object.get("allowFrom").is_some() {
                anyhow::bail!("rules: property {property} needs exactly one of deny or allowFrom");
            }
            if let Some(values) = object.get("deny").and_then(Value::as_array) {
                let deny: Vec<String> = values
                    .iter()
                    .map(|value| resolve_params(value.clone(), design, rule, notes))
                    .filter_map(|value| match value {
                        Value::String(raw) => Some(raw.to_lowercase()),
                        Value::Number(number) => Some(number.to_string()),
                        _ => None,
                    })
                    .collect();
                if deny.is_empty() {
                    anyhow::bail!("rules: property {property} deny list is empty");
                }
                return Ok(Some(PropertyRule::Deny(deny)));
            }
            if let Some(set) = object.get("allowFrom").and_then(Value::as_str) {
                if design.is_none() {
                    notes.push(json!({
                                "check": rule,
                                "reason": "missing design parameters",
                                "rule": rule,
                                "source": set,
                                "ruleSource": source,
                    "packSource": source
                            }));
                    return Ok(None);
                }
                return Ok(Some(PropertyRule::AllowFrom(set.to_string())));
            }
        }
        if let Some(components) = object
            .get("components")
            .or_else(|| object.get("component"))
            .and_then(Value::as_object)
        {
            let mut map = HashMap::new();
            for (name, value) in components {
                let tests = parse_verb(value, &format!("component {name} of {property}"))?;
                map.insert(name.clone(), tests);
            }
            let all_tests: Vec<PropertyTest> = map
                .values()
                .flat_map(|tests| tests.iter().cloned())
                .collect();
            note_missing_sets(&all_tests, design, notes, rule, source);
            return Ok(Some(PropertyRule::Components(map)));
        }
        if let Some(inner) = object.get("tests").or_else(|| object.get("constraints")) {
            let list = if let Some(list) = inner.as_array() {
                list.clone()
            } else {
                vec![inner.clone()]
            };
            let mut tests = Vec::new();
            for item in list {
                tests.extend(parse_verb(&item, property)?);
            }
            note_missing_sets(&tests, design, notes, rule, source);
            return Ok(Some(PropertyRule::Tests(tests)));
        }
    }
    if let Some(list) = value.as_array() {
        if list
            .iter()
            .all(|item| item.is_string() || item.is_number() || item.is_boolean())
        {
            let tests = vec![PropertyTest {
                verb: Verb::NotIn(Value::Array(list.clone())),
            }];
            note_missing_sets(&tests, design, notes, rule, source);
            return Ok(Some(PropertyRule::Tests(tests)));
        }
        let mut tests = Vec::new();
        for item in list {
            tests.extend(parse_verb(item, property)?);
        }
        note_missing_sets(&tests, design, notes, rule, source);
        return Ok(Some(PropertyRule::Tests(tests)));
    }
    let tests = parse_verb(value, property)?;
    note_missing_sets(&tests, design, notes, rule, source);
    Ok(Some(PropertyRule::Tests(tests)))
}

pub(crate) fn parse_cardinality(value: &Value) -> Option<Cardinality> {
    if let Some(count) = value.as_u64().or_else(|| {
        value
            .as_str()
            .and_then(|raw| raw.trim().parse::<u64>().ok())
    }) {
        return Some(Cardinality {
            minimum: Some(count),
            maximum: Some(count),
            region: None,
        });
    }
    let object = value.as_object()?;
    let number = |keys: &[&str]| {
        keys.iter().find_map(|key| {
            object.get(*key).and_then(|value| {
                value
                    .as_u64()
                    .or_else(|| value.as_f64().map(|number| number.max(0.0) as u64))
                    .or_else(|| value.as_str().and_then(|raw| raw.trim().parse().ok()))
            })
        })
    };
    let exactly = number(&["exactly", "count", "equals"]);
    let minimum = number(&["min", "minimum", "minCount", "atLeast", "at_least"]).or(exactly);
    let maximum = number(&["max", "maximum", "maxCount", "atMost", "at_most"]).or(exactly);
    let region = object
        .get("region")
        .or_else(|| object.get("within"))
        .or_else(|| object.get("scope"))
        .and_then(Value::as_str)
        .filter(|value| !matches!(*value, "page" | "document" | "viewport"))
        .map(str::to_string);
    if minimum.is_none() && maximum.is_none() {
        return None;
    }
    Some(Cardinality {
        minimum,
        maximum,
        region,
    })
}

pub(crate) fn parse_rules_v2(
    data: &Value,
    design: Option<&DesignSpec>,
) -> Result<(RuleSet, Vec<Value>)> {
    let mut notes = Vec::new();
    let mut selectors: Vec<String> = Vec::new();
    let mut properties: Vec<String> = Vec::new();
    let mut structural = Vec::new();
    let mut checks = Vec::new();
    let mut coverage = Vec::new();
    let mut lifecycles = Vec::new();
    let aliases = parse_aliases(data);
    let mut rule_aliases = data["ruleAliases"].as_object().cloned().unwrap_or_default();
    for key in ["aliases", "alias", "kinds", "kindAliases", "classAliases"] {
        if let Some(object) = data[key].as_object() {
            for (name, value) in object {
                let target = value.as_str().map(str::to_string).or_else(|| {
                    value
                        .as_array()
                        .and_then(|values| values.first())
                        .and_then(Value::as_str)
                        .map(str::to_string)
                });
                if let Some(target) = target {
                    rule_aliases.entry(name.clone()).or_insert(json!(target));
                }
            }
        }
    }

    let mut add_rule_alias = |entry: &Value| {
        if let Some(pair) = entry.as_array() {
            if let (Some(from), Some(to)) = (
                pair.first().and_then(Value::as_str),
                pair.get(1).and_then(Value::as_str),
            ) {
                rule_aliases.entry(from.to_string()).or_insert(json!(to));
            }
            return;
        }
        let Some(object) = entry.as_object() else {
            return;
        };
        let from = ["from", "old", "rule", "name"]
            .iter()
            .find_map(|key| object.get(*key).and_then(Value::as_str));
        let to = ["to", "new", "target", "baseline", "behaviour", "behavior"]
            .iter()
            .find_map(|key| object.get(*key).and_then(Value::as_str));
        if let (Some(from), Some(to)) = (from, to) {
            rule_aliases.entry(from.to_string()).or_insert(json!(to));
        }
    };
    if let Some(list) = data["ruleAliases"].as_array() {
        for entry in list {
            add_rule_alias(entry);
        }
    }
    for key in ["aliases", "alias", "kinds", "kindAliases", "classAliases"] {
        if let Some(list) = data[key].as_array() {
            for entry in list {
                add_rule_alias(entry);
            }
        }
    }

    let mut parse_rule = |rule: &Value| -> Result<()> {
        let name = rule["name"]
            .as_str()
            .or_else(|| rule["id"].as_str())
            .unwrap_or("")
            .to_string();
        let source = rule["_source"].as_str().unwrap_or("").to_string();
        if rule["enabled"].as_bool() == Some(false) {
            if !name.is_empty() {
                checks.push(CheckRule {
                    id: name.clone(),
                    output: name,
                    source,
                    enabled: false,
                    severity: None,
                    layer: None,
                });
            }
            return Ok(());
        }
        let mut rule_selectors = parse_selector_list(&rule["selectors"], &aliases);
        for selector in parse_selector_list(&rule["selector"], &aliases) {
            add_selector(&mut rule_selectors, selector);
        }
        if let Some(matched) = rule.get("match") {
            if let Some(selector) = matched.as_str() {
                add_selector(&mut rule_selectors, expand_kind_aliases(selector, &aliases));
            } else {
                for selector in parse_selector_list(&matched["selectors"], &aliases) {
                    add_selector(&mut rule_selectors, selector);
                }
                for selector in parse_selector_list(&matched["selector"], &aliases) {
                    add_selector(&mut rule_selectors, selector);
                }
            }
        }
        if let Some(list) = rule["constraints"].as_array() {
            for entry in list {
                if entry.get("attribute").is_none() && entry.get("membership").is_none() {
                    if let Some(selector) = selector_from_value(entry) {
                        add_selector(
                            &mut rule_selectors,
                            expand_kind_aliases(&selector, &aliases),
                        );
                    }
                }
            }
        }

        let declarative = !rule_selectors.is_empty()
            || rule.get("properties").is_some()
            || rule.get("tests").is_some()
            || rule.get("cardinality").is_some()
            || rule.get("ancestors").is_some()
            || rule.get("descendants").is_some()
            || rule.get("constraints").is_some()
            || rule.get("attributes").is_some()
            || rule.get("when").is_some()
            || rule.get("condition").is_some()
            || rule.get("expr").is_some();
        if !declarative {
            if name.is_empty() {
                anyhow::bail!("rules: named rule needs an id");
            }
            coverage.push(json!({
                "check": name,
                "rule": name,
                "name": name,
                "reason": "coverage gap: no default rule or declarative rule body",
                "coverageGap": true,
                "source": source,
                "ruleSource": source,
                "pack": source
            }));
            return Ok(());
        }

        if name.is_empty() {
            anyhow::bail!("rules: declarative rule needs a name");
        }
        if rule_selectors.is_empty()
            && !matches!(
                rule["scope"].as_str().unwrap_or(""),
                "page" | "document" | "sheet"
            )
            && !matches!(
                rule["target"].as_str().unwrap_or(""),
                "page" | "document" | "sheet"
            )
        {
            anyhow::bail!("rules: rule {name} needs a selector");
        }
        for selector in &rule_selectors {
            add_selector(&mut selectors, selector.clone());
        }
        let severity = rule["severity"].as_str().unwrap_or("advisory").to_string();
        let layer = rule["layer"].as_str().unwrap_or("layout").to_string();
        validate_severity_layer(&severity, &layer)?;
        let operator = parse_logic(&rule["operator"], Logic::All);
        let match_operator = parse_logic(
            rule.get("matchOperator")
                .or_else(|| rule.get("selectorsOperator"))
                .or_else(|| rule["match"].get("operator"))
                .unwrap_or(&Value::Null),
            operator,
        );
        let mut ancestors = parse_constraints(&rule["ancestors"], &aliases);
        let descendants = parse_constraints(&rule["descendants"], &aliases);
        if let Some(list) = rule["constraints"].as_array() {
            for entry in list {
                if entry.get("any").is_some() || entry.get("none").is_some() {
                    ancestors.extend(parse_constraints(entry, &aliases));
                }
            }
        }
        for constraint in ancestors.iter().chain(descendants.iter()) {
            add_selector(&mut selectors, constraint.selector.clone());
        }
        let mut exceptions = Vec::new();
        for key in ["exceptions", "exception", "except", "exclude", "excludes"] {
            for selector in parse_selector_list(&rule[key], &aliases) {
                add_selector(&mut exceptions, selector);
            }
        }
        for selector in &exceptions {
            add_selector(&mut selectors, selector.clone());
        }

        let mut rule_properties: Vec<(String, PropertyRule)> = Vec::new();
        let mut property_entries: Vec<(String, Value)> = Vec::new();
        if let Some(object) = rule["properties"].as_object() {
            for (property, value) in object {
                property_entries.push((property.clone(), value.clone()));
            }
        } else if let Some(list) = rule["properties"].as_array() {
            for entry in list {
                if let Some(property) = entry["property"].as_str() {
                    property_entries.push((property.to_string(), entry.clone()));
                }
            }
        }
        if let Some(object) = rule["tests"].as_object() {
            for (property, value) in object {
                property_entries.push((property.clone(), value.clone()));
            }
        } else if let Some(list) = rule["tests"].as_array() {
            for entry in list {
                if let Some(property) = entry["property"].as_str() {
                    property_entries.push((property.to_string(), entry.clone()));
                }
            }
        }
        if let Some(object) = rule["attributes"].as_object() {
            for (attribute, value) in object {
                property_entries.push((format!("@{attribute}"), value.clone()));
            }
        }
        for key in ["membership", "attributeValues", "attribute-values"] {
            let Some(value) = rule.get(key) else {
                continue;
            };
            if let Some(object) = value.as_object() {
                if let Some(attribute) = object.get("attribute").and_then(Value::as_str) {
                    let mut test = value.clone();
                    if let Some(test_object) = test.as_object_mut() {
                        test_object.remove("attribute");
                    }
                    property_entries.push((format!("@{attribute}"), test));
                } else {
                    for (attribute, test) in object {
                        if attribute != "operator" {
                            let test = if test.is_array() {
                                json!({ "notIn": test.clone() })
                            } else {
                                test.clone()
                            };
                            property_entries.push((format!("@{attribute}"), test));
                        }
                    }
                }
            } else if let Some(list) = value.as_array() {
                for entry in list {
                    if let Some(attribute) = entry.get("attribute").and_then(Value::as_str) {
                        let mut test = entry.clone();
                        if let Some(test_object) = test.as_object_mut() {
                            test_object.remove("attribute");
                        }
                        property_entries.push((format!("@{attribute}"), test));
                    }
                }
            }
        }
        if let Some(list) = rule["constraints"].as_array() {
            for entry in list {
                if let Some(property) = entry["property"].as_str() {
                    property_entries.push((property.to_string(), entry.clone()));
                } else if let Some(membership) = entry.get("attribute").and_then(Value::as_str) {
                    let mut value = entry.clone();
                    if let Some(object) = value.as_object_mut() {
                        object.remove("attribute");
                    }
                    property_entries.push((format!("@{membership}"), value));
                }
            }
        }
        for (property, value) in property_entries {
            let encoded = if property.starts_with('@')
                || property.starts_with("attr:")
                || property.starts_with("attribute:")
            {
                let attribute = property
                    .strip_prefix('@')
                    .or_else(|| property.strip_prefix("attr:"))
                    .or_else(|| property.strip_prefix("attribute:"))
                    .unwrap_or(&property)
                    .to_string();
                format!("@{attribute}")
            } else if property.starts_with("data-") || property.starts_with("aria-") {
                format!("@{property}")
            } else {
                if !CSS_PROPERTIES.contains(&property.as_str()) {
                    anyhow::bail!("rules: unknown CSS property {property} in rule {name}");
                }
                property.clone()
            };
            if !properties.contains(&encoded) {
                properties.push(encoded.clone());
            }
            if let Some(constraint) =
                parse_property_rule(&encoded, &value, design, &mut notes, &name, &source)?
            {
                rule_properties.push((encoded, constraint));
            }
        }
        let mut cardinality = parse_cardinality(&rule["cardinality"])
            .or_else(|| parse_cardinality(&rule["count"]))
            .or_else(|| {
                if rule.get("minCount").is_some()
                    || rule.get("maxCount").is_some()
                    || rule.get("minimum").is_some()
                    || rule.get("maximum").is_some()
                {
                    parse_cardinality(rule)
                } else {
                    None
                }
            });
        if let Some(cardinality) = cardinality.as_mut() {
            if let Some(region) = cardinality.region.take() {
                let region = expand_kind_aliases(&region, &aliases);
                add_selector(&mut selectors, region.clone());
                cardinality.region = Some(region);
            }
        }
        let when = rule
            .get("when")
            .or_else(|| rule.get("condition"))
            .or_else(|| rule.get("expr"))
            .cloned();
        let message = rule["message"].as_str().map(str::to_string);
        let scope = rule["scope"]
            .as_str()
            .or_else(|| rule["target"].as_str())
            .unwrap_or("element")
            .to_lowercase();
        let lifecycle = rule["lifecycle"]
            .as_str()
            .or_else(|| rule["at"].as_str())
            .or_else(|| rule["evidence"].as_str())
            .map(str::to_string);
        if let Some(lifecycle) = &lifecycle {
            if !lifecycles.contains(lifecycle) {
                lifecycles.push(lifecycle.clone());
            }
        }
        if let Some(reference) = dangling_param(&rule["params"]) {
            anyhow::bail!("rules: dangling param reference {reference} in rule {name}");
        }
        let params = resolve_params(rule["params"].clone(), design, &name, &mut notes);
        structural.push(StructuralRule {
            name,
            source,
            ancestors,
            descendants,
            selectors: rule_selectors,
            operator,
            match_operator,
            exceptions,
            properties: rule_properties,
            when,
            message,
            params,
            cardinality,
            scope,
            lifecycle,
            legacy: false,
            severity,
            layer,
        });
        Ok(())
    };

    for rule in data["structural"].as_array().cloned().unwrap_or_default() {
        parse_rule(&rule)?;
    }
    for rule in data["rules"].as_array().cloned().unwrap_or_default() {
        parse_rule(&rule)?;
    }

    Ok((
        RuleSet {
            structural,
            checks,
            selectors,
            properties,
            coverage,
        },
        notes,
    ))
}

pub(crate) fn validate_severity_layer(severity: &str, layer: &str) -> Result<()> {
    if !["error", "advisory", "P0", "P1", "P2", "P3"].contains(&severity) {
        anyhow::bail!("rules: severity must be error, advisory, or P0-P3");
    }
    if !["layout", "sheet", "content", "source"].contains(&layer) {
        anyhow::bail!("rules: unknown layer {layer}");
    }
    Ok(())
}

pub(crate) fn font_stack_allowed(stack: &str, allowed: &[String]) -> bool {
    font_families(stack)
        .iter()
        .all(|family| generic_family(family) || allowed.iter().any(|entry| entry == family))
}

pub(crate) fn property_allowed(source: &str, actual: &str, design: &DesignSpec) -> bool {
    match source {
        "colors" => {
            let Some(allowed) = &design.colors else {
                return true;
            };
            let Some(rgb) = parse_color_rgb(actual) else {
                return true;
            };
            allowed
                .iter()
                .any(|entry| entry.0 == rgb.0 && entry.1 == rgb.1 && entry.2 == rgb.2)
        }
        "fonts" => {
            let Some(allowed) = &design.fonts else {
                return true;
            };
            font_stack_allowed(actual, allowed)
        }
        "spacing" => {
            let Some(allowed) = &design.spacing else {
                return true;
            };
            let Some((number, unit)) = parse_dimension(actual) else {
                return true;
            };
            unit == "px" && allowed.iter().any(|entry| (entry - number).abs() < 0.5)
        }
        "radii" => {
            let Some(allowed) = &design.radii else {
                return true;
            };
            radius_allowed(actual, allowed)
        }
        "typography" => {
            let Some(steps) = &design.typography else {
                return true;
            };
            let Some((number, unit)) = parse_dimension(actual) else {
                return true;
            };
            unit == "px" && steps.iter().any(|step| (step.size - number).abs() <= 0.5)
        }
        _ => true,
    }
}

pub(crate) fn parse_color_alpha(css: &str) -> Option<(f64, f64, f64, f64)> {
    if let Some(values) = parse_css_color(css) {
        return Some(values);
    }
    let value = css.trim().to_lowercase();
    if value == "transparent" {
        return Some((0.0, 0.0, 0.0, 0.0));
    }
    let hex = value.strip_prefix('#')?;
    let expanded = match hex.len() {
        3 | 4 => hex
            .chars()
            .map(|character| format!("{character}{character}"))
            .collect::<String>(),
        6 | 8 => hex.to_string(),
        _ => return None,
    };
    let channel = |range: std::ops::Range<usize>| {
        i64::from_str_radix(&expanded[range], 16)
            .ok()
            .map(|value| value as f64)
    };
    let r = channel(0..2)?;
    let g = channel(2..4)?;
    let b = channel(4..6)?;
    let a = if expanded.len() == 8 {
        channel(6..8)? / 255.0
    } else {
        1.0
    };
    Some((r, g, b, a))
}

pub(crate) fn color_matches(actual: &str, expected: &str) -> bool {
    match (parse_color_alpha(actual), parse_color_alpha(expected)) {
        (Some(actual), Some(expected)) => {
            (actual.0 - expected.0).abs() < 0.5
                && (actual.1 - expected.1).abs() < 0.5
                && (actual.2 - expected.2).abs() < 0.5
                && (actual.3 - expected.3).abs() < 0.01
        }
        _ => false,
    }
}

pub(crate) fn deny_value_matches(property: &str, actual: &str, expected: &str) -> bool {
    if property == "color" || property.ends_with("-color") {
        color_matches(actual, expected)
    } else {
        actual.eq_ignore_ascii_case(expected)
    }
}

pub(crate) fn ancestor_any(elements: &[Value], index: usize, matches: &HashSet<usize>) -> bool {
    let mut current = elements[index]["parentIndex"].as_u64();
    while let Some(parent) = current {
        if matches.contains(&(parent as usize)) {
            return true;
        }
        current = elements
            .get(parent as usize)
            .and_then(|element| element["parentIndex"].as_u64());
    }
    false
}

pub(crate) fn descendant_any(elements: &[Value], index: usize, matches: &HashSet<usize>) -> bool {
    for &candidate in matches {
        if candidate == index {
            continue;
        }
        let mut current = elements
            .get(candidate)
            .and_then(|element| element["parentIndex"].as_u64());
        while let Some(parent) = current {
            if parent as usize == index {
                return true;
            }
            current = elements
                .get(parent as usize)
                .and_then(|element| element["parentIndex"].as_u64());
        }
    }
    false
}

pub(crate) fn element_ignored(element: &Value, rule: &str) -> bool {
    element["ignoreRules"]
        .as_array()
        .map(|list| {
            list.iter().any(|value| {
                value
                    .as_str()
                    .map(|entry| entry == rule || entry == "*")
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

pub(crate) fn split_css_values(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut depth = 0i32;
    for character in raw.trim().chars() {
        match character {
            '(' => {
                depth += 1;
                current.push(character);
            }
            ')' => {
                depth -= 1;
                current.push(character);
            }
            '/' if depth == 0 => {
                if !current.trim().is_empty() {
                    out.push(current.trim().to_string());
                }
                current.clear();
            }
            value if value.is_whitespace() && depth == 0 => {
                if !current.trim().is_empty() {
                    out.push(current.trim().to_string());
                    current.clear();
                }
            }
            _ => current.push(character),
        }
    }
    if !current.trim().is_empty() {
        out.push(current.trim().to_string());
    }
    out
}

pub(crate) fn box_components(property: &str, raw: &str) -> HashMap<String, String> {
    let values = split_css_values(raw);
    let mut out = HashMap::new();
    if values.is_empty() {
        return out;
    }
    let names = if property == "border-radius" {
        ["top-left", "top-right", "bottom-right", "bottom-left"]
    } else {
        ["top", "right", "bottom", "left"]
    };
    let expanded = match values.len() {
        1 => vec![values[0].clone(); 4],
        2 => vec![
            values[0].clone(),
            values[1].clone(),
            values[0].clone(),
            values[1].clone(),
        ],
        3 => vec![
            values[0].clone(),
            values[1].clone(),
            values[2].clone(),
            values[1].clone(),
        ],
        _ => values.iter().take(4).cloned().collect(),
    };
    for (name, value) in names.iter().zip(expanded) {
        out.insert((*name).to_string(), value);
    }
    let aliases = [
        ("top-left", ["topLeft", "topleft"]),
        ("top-right", ["topRight", "topright"]),
        ("bottom-right", ["bottomRight", "bottomright"]),
        ("bottom-left", ["bottomLeft", "bottomleft"]),
    ];
    for (canonical, alternates) in aliases {
        if let Some(value) = out.get(canonical).cloned() {
            for alternate in alternates {
                out.insert(alternate.to_string(), value.clone());
            }
        }
    }
    if property == "gap" || property == "grid-gap" {
        if let Some(first) = out.get("top").cloned() {
            out.insert("row".to_string(), first);
        }
        if let Some(second) = out.get("right").cloned() {
            out.insert("column".to_string(), second.clone());
            out.insert("column-gap".to_string(), second);
        }
    }
    out
}

pub(crate) fn colour_property(property: &str) -> bool {
    property == "color"
        || property.ends_with("-color")
        || property == "background"
        || property == "border"
        || property == "outline"
        || property == "box-shadow"
        || property == "text-decoration"
}

pub(crate) fn empty_property_value(property: &str, actual: &str) -> bool {
    let value = actual.trim().to_lowercase();
    if value.is_empty() {
        return true;
    }
    if property == "box-shadow" || property == "text-shadow" {
        return value == "none" || value == "transparent";
    }
    if property == "background"
        || property == "background-color"
        || property == "background-image"
        || property.ends_with("-color")
    {
        if value == "transparent" || value == "rgba(0, 0, 0, 0)" || value == "#00000000" {
            return true;
        }
        if let Some((_, _, _, alpha)) = parse_color_alpha(&value) {
            return alpha <= 0.001;
        }
    }
    if value == "none"
        && (property == "border" || property == "outline" || property == "background-image")
    {
        return true;
    }
    false
}

pub(crate) fn literal_value_matches(property: &str, actual: &str, expected: &str) -> bool {
    if colour_property(property) && color_matches(actual, expected) {
        return true;
    }
    let actual = actual.trim();
    let expected = expected.trim();
    if actual.eq_ignore_ascii_case(expected) {
        return true;
    }
    match (parse_dimension(actual), parse_dimension(expected)) {
        (Some((left_number, left_unit)), Some((right_number, right_unit))) => {
            left_unit == right_unit && (left_number - right_number).abs() < 0.001
        }
        _ => false,
    }
}

pub(crate) fn operand_values(operand: &Value) -> Vec<Value> {
    match operand {
        Value::Array(values) => values.clone(),
        Value::Object(object) => object
            .get("values")
            .or_else(|| object.get("$value"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_else(|| vec![operand.clone()]),
        other => vec![other.clone()],
    }
}

pub(crate) fn set_membership(
    source: &str,
    actual: &str,
    design: Option<&DesignSpec>,
) -> Option<bool> {
    let design = design?;
    let source = match source {
        "color" | "palette" | "colour" => "colors",
        "font" | "font-family" => "fonts",
        "space" | "dimension" => "spacing",
        "radius" => "radii",
        "type" | "font-size" => "typography",
        other => other,
    };
    if ["colors", "fonts", "spacing", "radii", "typography"].contains(&source) {
        return Some(property_allowed(source, actual, design));
    }
    let values = resolve_param(Some(design), source)?;
    let list = operand_values(&values);
    if list.is_empty() {
        return None;
    }
    Some(list.iter().any(|value| match value {
        Value::String(raw) => literal_value_matches("", actual, raw),
        Value::Number(number) => literal_value_matches("", actual, &number.to_string()),
        _ => false,
    }))
}

pub(crate) fn verb_passes(
    verb: &Verb,
    property: &str,
    actual: &str,
    design: Option<&DesignSpec>,
) -> bool {
    match verb {
        Verb::Present => !empty_property_value(property, actual),
        Verb::Absent => empty_property_value(property, actual),
        Verb::Equals(expected) => {
            let raw = match expected {
                Value::String(raw) => raw.clone(),
                Value::Number(number) => number.to_string(),
                Value::Bool(value) => value.to_string(),
                _ => expected.to_string(),
            };
            literal_value_matches(property, actual, &raw)
        }
        Verb::NotEquals(expected) => {
            let raw = match expected {
                Value::String(raw) => raw.clone(),
                Value::Number(number) => number.to_string(),
                Value::Bool(value) => value.to_string(),
                _ => expected.to_string(),
            };
            !literal_value_matches(property, actual, &raw)
        }
        Verb::In(operand) => {
            if let Some(source) = operand.as_str() {
                set_membership(source, actual, design).unwrap_or(true)
            } else {
                operand_values(operand).iter().any(|value| {
                    let raw = value.as_str().map(str::to_string).unwrap_or_else(|| {
                        value
                            .as_f64()
                            .map(|number| number.to_string())
                            .unwrap_or_default()
                    });
                    literal_value_matches(property, actual, &raw)
                })
            }
        }
        Verb::NotIn(operand) => {
            if let Some(source) = operand.as_str() {
                set_membership(source, actual, design)
                    .map(|member| !member)
                    .unwrap_or(true)
            } else {
                !operand_values(operand).iter().any(|value| {
                    let raw = value.as_str().map(str::to_string).unwrap_or_else(|| {
                        value
                            .as_f64()
                            .map(|number| number.to_string())
                            .unwrap_or_default()
                    });
                    literal_value_matches(property, actual, &raw)
                })
            }
        }
    }
}

pub(crate) fn combine_outcomes(operator: Logic, outcomes: &[bool]) -> bool {
    match operator {
        Logic::All => outcomes.iter().all(|value| *value),
        Logic::Any => outcomes.iter().any(|value| *value),
        Logic::One => outcomes.iter().filter(|value| **value).count() == 1,
    }
}

pub(crate) fn describe_verb(verb: &Verb) -> (&'static str, Value) {
    match verb {
        Verb::In(operand) => ("in", operand.clone()),
        Verb::NotIn(operand) => ("notIn", operand.clone()),
        Verb::Equals(operand) => ("equals", operand.clone()),
        Verb::NotEquals(operand) => ("notEquals", operand.clone()),
        Verb::Present => ("present", json!(true)),
        Verb::Absent => ("absent", json!(true)),
    }
}

pub(crate) fn evaluate_property_rule(
    rule: &StructuralRule,
    property: &str,
    actual: &str,
    design: Option<&DesignSpec>,
) -> Option<(String, String, String)> {
    match &rule
        .properties
        .iter()
        .find(|(name, _)| name == property)
        .map(|(_, rule)| rule)
    {
        Some(PropertyRule::Deny(deny)) => {
            if actual.is_empty() {
                return None;
            }
            if deny
                .iter()
                .any(|entry| deny_value_matches(property, actual, entry))
            {
                Some((property.to_string(), deny.join(","), actual.to_string()))
            } else {
                None
            }
        }
        Some(PropertyRule::AllowFrom(source)) => {
            if actual.is_empty() {
                return None;
            }
            if let Some(design) = design {
                if !property_allowed(source, actual, design) {
                    return Some((
                        property.to_string(),
                        "parameter set".to_string(),
                        actual.to_string(),
                    ));
                }
            }
            None
        }
        Some(PropertyRule::Tests(tests)) => {
            let outcomes: Vec<bool> = tests
                .iter()
                .map(|test| verb_passes(&test.verb, property, actual, design))
                .collect();
            if !combine_outcomes(rule.operator, &outcomes) {
                return None;
            }
            let failed = tests
                .iter()
                .zip(outcomes.iter())
                .find(|(_, matched)| **matched)
                .map(|(test, _)| test)
                .unwrap_or(&tests[0]);
            let (verb, operand) = describe_verb(&failed.verb);
            Some((
                property.to_string(),
                serde_json::to_string(&json!({"verb": verb, "operand": operand, "operator": format!("{:?}", rule.operator).to_lowercase()}))
                    .unwrap_or_default(),
                actual.to_string(),
            ))
        }
        Some(PropertyRule::Components(components)) => {
            let values = box_components(property, actual);
            let mut outcomes = Vec::new();
            for (component, tests) in components {
                let value = values
                    .get(component)
                    .cloned()
                    .unwrap_or_else(|| actual.to_string());
                for test in tests {
                    outcomes.push(verb_passes(&test.verb, property, &value, design));
                }
            }
            if outcomes.is_empty() || !combine_outcomes(rule.operator, &outcomes) {
                return None;
            }
            Some((
                property.to_string(),
                "component set".to_string(),
                actual.to_string(),
            ))
        }
        None => None,
    }
}

fn expr_path<'a>(root: &'a Value, path: &str) -> &'a Value {
    let mut current = root;
    for part in path.split('.') {
        if part.is_empty() {
            continue;
        }
        if let Ok(index) = part.parse::<usize>() {
            current = current.get(index).unwrap_or(&Value::Null);
        } else {
            current = current.get(part).unwrap_or(&Value::Null);
        }
    }
    current
}

fn expr_field<'a>(path: &str, element: &'a Value, page: &'a Value, sheet: &'a str) -> Value {
    if let Some(path) = path.strip_prefix("page.") {
        return expr_path(page, path).clone();
    }
    if path == "page" {
        return page.clone();
    }
    if path == "self" || path == "." {
        return element.clone();
    }
    if path == "sheet" || path == "sheet.text" {
        return json!(sheet);
    }
    expr_path(element, path).clone()
}

fn expr_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(number) => number.as_f64().unwrap_or(0.0) != 0.0,
        Value::String(value) => !value.is_empty(),
        Value::Array(values) => !values.is_empty(),
        Value::Object(_) => true,
    }
}

fn expr_number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(raw) => raw
            .trim()
            .parse::<f64>()
            .ok()
            .or_else(|| parse_dimension(raw).map(|(number, _)| number)),
        Value::Bool(value) => Some(if *value { 1.0 } else { 0.0 }),
        _ => None,
    }
}

fn expr_equal(left: &Value, right: &Value) -> bool {
    if let (Some(left), Some(right)) = (expr_number(left), expr_number(right)) {
        return (left - right).abs() < 0.001;
    }
    match (left, right) {
        (Value::String(left), Value::String(right)) => left.eq_ignore_ascii_case(right),
        _ => left == right,
    }
}

fn expr_text(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        other => other.to_string(),
    }
}

fn expr_colors(value: &Value) -> Vec<(f64, f64, f64, f64)> {
    let text = expr_text(value);
    let mut out = Vec::new();
    for pattern in [r"#[0-9a-fA-F]{3,8}\b", r"rgba?\([^)]*\)"] {
        let Ok(regex) = Regex::new(pattern) else {
            continue;
        };
        for capture in regex.find_iter(&text) {
            if let Some(color) = parse_color_alpha(capture.as_str()) {
                out.push(color);
            }
        }
    }
    if out.is_empty() {
        if let Some(color) = parse_color_alpha(&text) {
            out.push(color);
        }
    }
    out
}

fn expr_color(value: &Value) -> Option<(f64, f64, f64, f64)> {
    expr_colors(value).into_iter().next()
}

fn expr_chroma(color: (f64, f64, f64, f64)) -> f64 {
    color.0.max(color.1).max(color.2) - color.0.min(color.1).min(color.2)
}

fn expr_hue(color: (f64, f64, f64, f64)) -> f64 {
    let (r, g, b) = (color.0 / 255.0, color.1 / 255.0, color.2 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    if delta <= f64::EPSILON {
        return 0.0;
    }
    let raw = if max == r {
        ((g - b) / delta) % 6.0
    } else if max == g {
        (b - r) / delta + 2.0
    } else {
        (r - g) / delta + 4.0
    };
    (raw * 60.0 + 360.0) % 360.0
}

type ShadowParts = (f64, f64, f64, f64, (f64, f64, f64, f64), bool);

fn expr_shadow(value: &Value) -> Option<ShadowParts> {
    let text = expr_text(value);
    let first = text.split("),").next().unwrap_or(&text);
    let inset = first.contains("inset");
    let Ok(regex) = Regex::new(r"(-?\d*\.?\d+)px") else {
        return None;
    };
    let numbers: Vec<f64> = regex
        .captures_iter(first)
        .filter_map(|capture| capture[1].parse::<f64>().ok())
        .collect();
    if numbers.len() < 3 {
        return None;
    }
    let color = expr_colors(&json!(first))
        .into_iter()
        .next()
        .unwrap_or((0.0, 0.0, 0.0, 1.0));
    Some((
        numbers[0],
        numbers[1],
        numbers[2],
        *numbers.get(3).unwrap_or(&0.0),
        color,
        inset,
    ))
}

fn eval_expr(expr: &Value, element: &Value, page: &Value, sheet: &str) -> Value {
    let Some(object) = expr.as_object() else {
        return expr.clone();
    };
    if let Some(path) = object.get("field").and_then(Value::as_str) {
        return expr_field(path, element, page, sheet);
    }
    if object.len() != 1 {
        return Value::Null;
    }
    let (op, operand) = object.iter().next().unwrap();
    if matches!(op.as_str(), "map" | "flatMap" | "filter") {
        let Some(items) = operand
            .as_array()
            .and_then(|values| values.first())
            .map(|value| eval_expr(value, element, page, sheet))
            .and_then(|value| value.as_array().cloned())
        else {
            return Value::Array(Vec::new());
        };
        let Some(predicate) = operand.as_array().and_then(|values| values.get(1)) else {
            return Value::Array(Vec::new());
        };
        let mut out = Vec::new();
        for item in items {
            let value = eval_expr(predicate, &item, page, sheet);
            match op.as_str() {
                "filter" => {
                    if expr_truthy(&value) {
                        out.push(item);
                    }
                }
                "flatMap" => match value {
                    Value::Array(values) => out.extend(values),
                    other => out.push(other),
                },
                _ => out.push(value),
            }
        }
        return Value::Array(out);
    }
    if matches!(op.as_str(), "groupAny" | "groupAll") {
        let Some(items) = operand
            .as_array()
            .and_then(|values| values.first())
            .map(|value| eval_expr(value, element, page, sheet))
            .and_then(|value| value.as_array().cloned())
        else {
            return Value::Bool(false);
        };
        let Some(key_expression) = operand.as_array().and_then(|values| values.get(1)) else {
            return Value::Bool(false);
        };
        let Some(predicate) = operand.as_array().and_then(|values| values.get(2)) else {
            return Value::Bool(false);
        };
        let mut groups: Vec<(Value, Vec<Value>)> = Vec::new();
        for item in items {
            let key = eval_expr(key_expression, &item, page, sheet);
            if let Some((_, values)) = groups.iter_mut().find(|(existing, _)| *existing == key) {
                values.push(item);
            } else {
                groups.push((key, vec![item]));
            }
        }
        let total = groups.len();
        let mut matched = 0usize;
        for (key, values) in groups {
            let context = json!({ "key": key, "values": values });
            if expr_truthy(&eval_expr(predicate, &context, page, sheet)) {
                matched += 1;
            }
        }
        return Value::Bool(if op == "groupAny" {
            matched > 0
        } else {
            matched == total
        });
    }
    if op == "firstOf" {
        let Some(items) = operand
            .as_array()
            .and_then(|values| values.first())
            .map(|value| eval_expr(value, element, page, sheet))
            .and_then(|value| value.as_array().cloned())
        else {
            return Value::Null;
        };
        let Some(predicate) = operand.as_array().and_then(|values| values.get(1)) else {
            return Value::Null;
        };
        for item in items {
            if expr_truthy(&eval_expr(predicate, &item, page, sheet)) {
                return item;
            }
        }
        return Value::Null;
    }
    if op == "pairAny" {
        let Some(items) = operand
            .as_array()
            .and_then(|values| values.first())
            .map(|value| eval_expr(value, element, page, sheet))
            .and_then(|value| value.as_array().cloned())
        else {
            return Value::Bool(false);
        };
        let Some(predicate) = operand.as_array().and_then(|values| values.get(1)) else {
            return Value::Bool(false);
        };
        for left in 0..items.len() {
            for right in left + 1..items.len() {
                let context = json!({ "left": items[left], "right": items[right] });
                if expr_truthy(&eval_expr(predicate, &context, page, sheet)) {
                    return Value::Bool(true);
                }
            }
        }
        return Value::Bool(false);
    }
    if matches!(op.as_str(), "adjacentAny" | "adjacentAll") {
        let Some(items) = operand
            .as_array()
            .and_then(|values| values.first())
            .map(|value| eval_expr(value, element, page, sheet))
            .and_then(|value| value.as_array().cloned())
        else {
            return Value::Null;
        };
        let Some(predicate) = operand.as_array().and_then(|values| values.get(1)) else {
            return Value::Null;
        };
        let total = items.len().saturating_sub(1);
        let mut matched = 0usize;
        for pair in items.windows(2) {
            let context = json!({ "left": pair[0], "right": pair[1] });
            if expr_truthy(&eval_expr(predicate, &context, page, sheet)) {
                matched += 1;
            }
        }
        return Value::Bool(if op == "adjacentAny" {
            matched > 0
        } else {
            matched == total
        });
    }
    if matches!(op.as_str(), "anyOf" | "allOf" | "countOf" | "sumOf") {
        let Some(items) = operand
            .as_array()
            .and_then(|values| values.first())
            .map(|value| eval_expr(value, element, page, sheet))
            .and_then(|value| value.as_array().cloned())
        else {
            return Value::Null;
        };
        let Some(predicate) = operand.as_array().and_then(|values| values.get(1)) else {
            return Value::Null;
        };
        let total = items.len();
        let mut matched = 0usize;
        let mut sum = 0.0;
        for item in items {
            let value = eval_expr(predicate, &item, page, sheet);
            if expr_truthy(&value) {
                matched += 1;
            }
            if op == "sumOf" {
                sum += expr_number(&value).unwrap_or(0.0);
            }
        }
        return match op.as_str() {
            "anyOf" => Value::Bool(matched > 0),
            "allOf" => Value::Bool(matched == total),
            "countOf" => json!(matched),
            _ => json!(sum),
        };
    }
    let values: Vec<Value> = match operand {
        Value::Array(values) => values
            .iter()
            .map(|value| eval_expr(value, element, page, sheet))
            .collect(),
        other => vec![eval_expr(other, element, page, sheet)],
    };
    match op.as_str() {
        "all" | "and" => Value::Bool(values.iter().all(expr_truthy)),
        "any" | "or" => Value::Bool(values.iter().any(expr_truthy)),
        "not" => Value::Bool(!values.first().map(expr_truthy).unwrap_or(false)),
        "if" => {
            if values.first().map(expr_truthy).unwrap_or(false) {
                values.get(1).cloned().unwrap_or(Value::Null)
            } else {
                values.get(2).cloned().unwrap_or(Value::Null)
            }
        }
        "get" => {
            let Some(path) = values.get(1).map(expr_text) else {
                return Value::Null;
            };
            values
                .first()
                .map(|value| expr_path(value, &path).clone())
                .unwrap_or(Value::Null)
        }
        "ancestor" => {
            let mut found = false;
            if let (Some(left), Some(right)) = (
                values.first().and_then(expr_number),
                values.get(1).and_then(expr_number),
            ) {
                let elements = page["elements"].as_array().cloned().unwrap_or_default();
                let mut current = elements
                    .get(right as usize)
                    .and_then(|element| element["parentIndex"].as_u64());
                while let Some(parent) = current {
                    if parent == left as u64 {
                        found = true;
                        break;
                    }
                    current = elements
                        .get(parent as usize)
                        .and_then(|element| element["parentIndex"].as_u64());
                }
            }
            Value::Bool(found)
        }
        "param" => {
            let key = values.first().map(expr_text).unwrap_or_default();
            page["ruleParams"].get(&key).cloned().unwrap_or(Value::Null)
        }
        "contrast" => crate::contrast_ratio(element)
            .map(|value| json!(value))
            .unwrap_or(Value::Null),
        "shadowInset" => Value::Bool(
            values
                .first()
                .and_then(expr_shadow)
                .map(|shadow| shadow.5)
                .unwrap_or(false),
        ),
        "shadowBlur" => json!(values
            .first()
            .and_then(expr_shadow)
            .map(|shadow| shadow.2)
            .unwrap_or(0.0)),
        "shadowOffset" => json!(values
            .first()
            .and_then(expr_shadow)
            .map(|shadow| (shadow.0 * shadow.0 + shadow.1 * shadow.1).sqrt())
            .unwrap_or(0.0)),
        "shadowAlpha" => json!(values
            .first()
            .and_then(expr_shadow)
            .map(|shadow| shadow.4 .3)
            .unwrap_or(0.0)),
        "shadowChromatic" => Value::Bool(
            values
                .first()
                .and_then(expr_shadow)
                .map(|shadow| expr_chroma(shadow.4) > 25.0 && shadow.4 .3 > 0.2)
                .unwrap_or(false),
        ),
        "alpha" => json!(values
            .first()
            .and_then(expr_color)
            .map(|value| value.3)
            .unwrap_or(0.0)),
        "chroma" => json!(values
            .first()
            .and_then(expr_color)
            .map(expr_chroma)
            .unwrap_or(0.0)),
        "hue" => json!(values
            .first()
            .and_then(expr_color)
            .map(expr_hue)
            .unwrap_or(0.0)),
        "luminance" => json!(values
            .first()
            .and_then(expr_color)
            .map(|value| crate::luminance(value.0, value.1, value.2))
            .unwrap_or(0.0)),
        "chromatic" => Value::Bool(
            values
                .first()
                .map(expr_colors)
                .unwrap_or_default()
                .iter()
                .any(|value| expr_chroma(*value) > 25.0 && value.3 > 0.2),
        ),
        "dark" => Value::Bool(
            values
                .first()
                .and_then(expr_color)
                .map(|value| crate::luminance(value.0, value.1, value.2) < 0.35)
                .unwrap_or(false),
        ),
        "red" | "r" => json!(values
            .first()
            .and_then(expr_color)
            .map(|value| value.0)
            .unwrap_or(0.0)),
        "green" | "g" => json!(values
            .first()
            .and_then(expr_color)
            .map(|value| value.1)
            .unwrap_or(0.0)),
        "blue" | "b" => json!(values
            .first()
            .and_then(expr_color)
            .map(|value| value.2)
            .unwrap_or(0.0)),
        "gt" => Value::Bool(
            values.len() >= 2
                && expr_number(&values[0]).unwrap_or(0.0) > expr_number(&values[1]).unwrap_or(0.0),
        ),
        "gte" | "atLeast" => Value::Bool(
            values.len() >= 2
                && expr_number(&values[0]).unwrap_or(0.0) >= expr_number(&values[1]).unwrap_or(0.0),
        ),
        "lt" => Value::Bool(
            values.len() >= 2
                && expr_number(&values[0]).unwrap_or(0.0) < expr_number(&values[1]).unwrap_or(0.0),
        ),
        "lte" | "atMost" => Value::Bool(
            values.len() >= 2
                && expr_number(&values[0]).unwrap_or(0.0) <= expr_number(&values[1]).unwrap_or(0.0),
        ),
        "equals" | "eq" => Value::Bool(values.len() >= 2 && expr_equal(&values[0], &values[1])),
        "notEquals" | "ne" => Value::Bool(values.len() >= 2 && !expr_equal(&values[0], &values[1])),
        "contains" => {
            Value::Bool(values.len() >= 2 && expr_text(&values[0]).contains(&expr_text(&values[1])))
        }
        "matches" => {
            let text = values.first().map(expr_text).unwrap_or_default();
            let pattern = values.get(1).map(expr_text).unwrap_or_default();
            Value::Bool(
                Regex::new(&pattern)
                    .map(|regex| regex.is_match(&text))
                    .unwrap_or(false),
            )
        }
        "countMatches" | "count_matches" => {
            let text = values.first().map(expr_text).unwrap_or_default();
            let pattern = values.get(1).map(expr_text).unwrap_or_default();
            json!(Regex::new(&pattern)
                .map(|regex| regex.find_iter(&text).count())
                .unwrap_or(0))
        }
        "in" => Value::Bool(
            values.len() >= 2
                && values[1]
                    .as_array()
                    .map(|list| list.iter().any(|item| expr_equal(&values[0], item)))
                    .unwrap_or(false),
        ),
        "number" | "numeric" => Value::Bool(values.first().and_then(expr_number).is_some()),
        "headingLevel" => json!(values
            .first()
            .map(expr_text)
            .unwrap_or_default()
            .trim_start_matches('h')
            .parse::<u32>()
            .unwrap_or(0)),
        "between" => Value::Bool(
            values.len() >= 3
                && expr_number(&values[0])
                    .map(|value| {
                        value >= expr_number(&values[1]).unwrap_or(value)
                            && value <= expr_number(&values[2]).unwrap_or(value)
                    })
                    .unwrap_or(false),
        ),
        "length" => Value::Number(
            values
                .first()
                .map(|value| match value {
                    Value::String(value) => value.chars().count(),
                    Value::Array(values) => values.len(),
                    other => other.to_string().chars().count(),
                })
                .unwrap_or(0)
                .into(),
        ),
        "concat" => {
            let mut out = Vec::new();
            for value in values {
                match value {
                    Value::Array(items) => out.extend(items),
                    other => out.push(other),
                }
            }
            Value::Array(out)
        }
        "unique" => {
            let mut out = Vec::new();
            if let Some(Value::Array(items)) = values.first() {
                for item in items {
                    if !out.contains(item) {
                        out.push(item.clone());
                    }
                }
            }
            Value::Array(out)
        }
        "sortNumbers" => {
            let mut out = values
                .first()
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            out.sort_by(|left, right| {
                expr_number(left)
                    .unwrap_or(0.0)
                    .partial_cmp(&expr_number(right).unwrap_or(0.0))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            Value::Array(out)
        }
        "lower" => json!(values
            .first()
            .map(expr_text)
            .unwrap_or_default()
            .to_lowercase()),
        "blocks" => {
            let text = values.first().map(expr_text).unwrap_or_default();
            let blocks: Vec<Value> = if text.contains("\n\n") {
                text.split("\n\n")
                    .map(str::trim)
                    .map(|value| json!(value))
                    .collect()
            } else {
                text.lines()
                    .map(str::trim)
                    .map(|value| json!(value))
                    .collect()
            };
            Value::Array(blocks)
        }
        "lastSentence" => {
            let text = values.first().map(expr_text).unwrap_or_default();
            let last = text
                .split(['.', '!', '?'])
                .rev()
                .find(|sentence| !sentence.trim().is_empty())
                .unwrap_or("")
                .trim();
            json!(last)
        }
        "containsAny" => {
            let text = values
                .first()
                .map(expr_text)
                .unwrap_or_default()
                .to_lowercase();
            let patterns = values
                .get(1)
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            Value::Bool(
                patterns
                    .iter()
                    .any(|pattern| text.contains(&expr_text(pattern).to_lowercase())),
            )
        }
        "overshooting" => {
            let text = values.first().map(expr_text).unwrap_or_default();
            let lower = text.to_lowercase();
            let mut found = lower.contains("bounce") || lower.contains("elastic");
            if !found {
                if let Ok(regex) = Regex::new(r"cubic-bezier\(([^)]*)\)") {
                    found = regex
                        .captures_iter(&text)
                        .any(|capture| is_overshooting(capture.get(0).unwrap().as_str()));
                }
            }
            Value::Bool(found)
        }
        "byIndex" => {
            let index = values.get(1).and_then(expr_number).unwrap_or(-1.0);
            values
                .first()
                .and_then(Value::as_array)
                .and_then(|items| items.get(index as usize))
                .cloned()
                .unwrap_or(Value::Null)
        }
        "join" => {
            let delimiter = values.get(1).map(expr_text).unwrap_or_default();
            json!(values
                .first()
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .map(expr_text)
                        .collect::<Vec<_>>()
                        .join(&delimiter)
                })
                .unwrap_or_default())
        }
        "nearestCard" => {
            let elements = page["elements"].as_array().cloned().unwrap_or_default();
            let mut current = element["parentIndex"].as_u64();
            let mut found = Value::Null;
            while let Some(parent) = current {
                let Some(candidate) = elements.get(parent as usize) else {
                    break;
                };
                if candidate["cardLike"].as_bool().unwrap_or(false) {
                    found = json!(parent);
                    break;
                }
                current = candidate["parentIndex"].as_u64();
            }
            found
        }
        "uniqueCount" => {
            let mut unique = HashSet::new();
            if let Some(Value::Array(items)) = values.first() {
                for item in items {
                    unique.insert(item.to_string());
                }
            }
            json!(unique.len())
        }
        "words" => json!(values
            .first()
            .map(expr_text)
            .unwrap_or_default()
            .split_whitespace()
            .count()),
        "min" | "max" => {
            let list: Vec<f64> = if values.len() > 1 {
                values.iter().filter_map(expr_number).collect()
            } else {
                values
                    .first()
                    .map(|value| match value {
                        Value::Array(values) => values.iter().filter_map(expr_number).collect(),
                        other => vec![expr_number(other).unwrap_or(0.0)],
                    })
                    .unwrap_or_default()
            };
            let result = if op == "min" {
                list.iter().copied().fold(f64::INFINITY, f64::min)
            } else {
                list.iter().copied().fold(f64::NEG_INFINITY, f64::max)
            };
            json!(if result.is_finite() { result } else { 0.0 })
        }
        "nthLargest" => {
            let mut list: Vec<f64> = values
                .first()
                .and_then(Value::as_array)
                .map(|values| values.iter().filter_map(expr_number).collect())
                .unwrap_or_default();
            list.sort_by(|left, right| {
                right.partial_cmp(left).unwrap_or(std::cmp::Ordering::Equal)
            });
            let index = values.get(1).and_then(expr_number).unwrap_or(0.0) as usize;
            json!(list.get(index).copied().unwrap_or(0.0))
        }
        "add" => json!(values.iter().filter_map(expr_number).sum::<f64>()),
        "sub" => json!(
            values.first().and_then(expr_number).unwrap_or(0.0)
                - values.get(1).and_then(expr_number).unwrap_or(0.0)
        ),
        "mul" => json!(values.iter().filter_map(expr_number).product::<f64>()),
        "div" => {
            let denominator = values.get(1).and_then(expr_number).unwrap_or(1.0);
            json!(values.first().and_then(expr_number).unwrap_or(0.0) / denominator)
        }
        _ => Value::Null,
    }
}

pub(crate) fn is_first_paint(lifecycle: &str) -> bool {
    let lifecycle = lifecycle.to_lowercase().replace(['_', '-', ' '], "");
    lifecycle.contains("firstpaint")
        || lifecycle.contains("firstframe")
        || lifecycle.contains("domcontentloaded")
        || lifecycle.contains("staged")
}

pub(crate) fn rule_findings(
    rules: &RuleSet,
    oracle: &Oracle,
    baseline: &Value,
    design: Option<&DesignSpec>,
    sheet_text: &str,
    lifecycle_evidence: Option<&Value>,
) -> Vec<Value> {
    let elements = baseline["elements"].as_array().cloned().unwrap_or_default();
    let mut findings = Vec::new();

    for rule in &rules.structural {
        let wants_first_paint = rule.lifecycle.as_deref().is_some_and(is_first_paint);
        let staged = if wants_first_paint {
            lifecycle_evidence.filter(|value| {
                value["baseline"]["elements"]
                    .as_array()
                    .map(|elements| elements.len() > 2)
                    .unwrap_or(false)
            })
        } else {
            None
        };
        if wants_first_paint && staged.is_none() {
            continue;
        }
        let staged_oracle = staged.and_then(|value| oracle_from_value(&value["oracle"]).ok());
        let staged_elements = staged
            .and_then(|value| value["baseline"]["elements"].as_array())
            .map(Vec::as_slice);
        let elements = staged_elements.unwrap_or(elements.as_slice());
        let page = staged.map(|value| &value["baseline"]).unwrap_or(baseline);
        let expression_page = if rule.params.is_object() {
            let mut page = page.clone();
            page["ruleParams"] = rule.params.clone();
            Some(page)
        } else {
            None
        };
        let expr_page = expression_page.as_ref().unwrap_or(page);
        let oracle = staged_oracle.as_ref().unwrap_or(oracle);
        if matches!(rule.scope.as_str(), "page" | "document" | "sheet") {
            if let Some(expression) = &rule.when {
                let value = eval_expr(expression, &Value::Null, expr_page, sheet_text);
                if expr_truthy(&value) {
                    let mut finding = json!({
                        "check": rule.name,
                        "ruleSource": rule.source,
                        "packSource": rule.source,
                        "severity": rule.severity,
                        "layer": rule.layer,
                        "selector": ":root",
                        "property": "when",
                        "message": rule.message.clone().unwrap_or_else(|| format!("{} matched", rule.name)),
                        "evidence": { "expression": expression }
                    });
                    if let Some(lifecycle) = &rule.lifecycle {
                        finding["lifecycle"] = json!(lifecycle);
                    }
                    findings.push(finding);
                }
            }
            continue;
        }
        let selector_count = rule.selectors.len().max(1);
        let mut selector_hits: HashMap<usize, usize> = HashMap::new();
        for selector in &rule.selectors {
            for index in oracle.matches(selector) {
                *selector_hits.entry(*index).or_insert(0) += 1;
            }
        }
        let candidate_indexes: Vec<usize> = selector_hits
            .iter()
            .filter(|(_, hits)| match rule.match_operator {
                Logic::All => **hits == selector_count,
                Logic::Any => **hits > 0,
                Logic::One => **hits == 1,
            })
            .map(|(index, _)| *index)
            .collect();
        let mut exception_matches: HashSet<usize> = HashSet::new();
        for selector in &rule.exceptions {
            exception_matches.extend(oracle.matches(selector).iter().copied());
        }
        let region_matches: HashSet<usize> = rule
            .cardinality
            .as_ref()
            .and_then(|cardinality| cardinality.region.as_ref())
            .map(|selector| oracle.matches(selector).iter().copied().collect())
            .unwrap_or_default();
        let mut candidates = Vec::new();
        let mut legacy_conditions: Vec<(usize, String, String)> = Vec::new();
        for index in candidate_indexes {
            let Some(element) = elements.get(index) else {
                continue;
            };
            if !element["visible"].as_bool().unwrap_or(false) {
                continue;
            }
            if element_ignored(element, &rule.name) {
                continue;
            }
            if exception_matches.contains(&index)
                || ancestor_any(elements, index, &exception_matches)
            {
                continue;
            }
            let mut conditions = Vec::new();
            let mut condition_failure: Option<(String, String)> = None;
            for constraint in &rule.ancestors {
                let matches: HashSet<usize> = oracle
                    .matches(&constraint.selector)
                    .iter()
                    .copied()
                    .collect();
                let found = ancestor_any(elements, index, &matches);
                conditions.push(found == constraint.any);
                if found != constraint.any && condition_failure.is_none() {
                    condition_failure =
                        Some(("ancestors".to_string(), constraint.selector.clone()));
                }
            }
            for constraint in &rule.descendants {
                let matches: HashSet<usize> = oracle
                    .matches(&constraint.selector)
                    .iter()
                    .copied()
                    .collect();
                let found = descendant_any(elements, index, &matches);
                conditions.push(found == constraint.any);
                if found != constraint.any && condition_failure.is_none() {
                    condition_failure =
                        Some(("descendants".to_string(), constraint.selector.clone()));
                }
            }
            if !conditions.is_empty() && !combine_outcomes(rule.match_operator, &conditions) {
                if rule.legacy {
                    if let Some((property, selector)) = condition_failure {
                        legacy_conditions.push((index, property, selector));
                    }
                }
                continue;
            }
            candidates.push(index);
        }

        for (index, property, expected) in legacy_conditions {
            let Some(element) = elements.get(index) else {
                continue;
            };
            let mut finding = json!({
                "check": rule.name,
                "ruleSource": rule.source,
                "packSource": rule.source,
                "severity": rule.severity,
                "layer": rule.layer,
                "selector": element["selector"],
                "property": property,
                "message": format!("{} violated", rule.name),
                "evidence": { "expected": expected, "actual": "" }
            });
            if let Some(lifecycle) = &rule.lifecycle {
                finding["lifecycle"] = json!(lifecycle);
            }
            findings.push(finding);
        }

        if let Some(cardinality) = &rule.cardinality {
            let count = candidates
                .iter()
                .filter(|index| {
                    if cardinality.region.is_none() {
                        return true;
                    }
                    region_matches.contains(index)
                        || ancestor_any(elements, **index, &region_matches)
                })
                .count() as u64;
            let below = cardinality.minimum.map(|min| count < min).unwrap_or(false);
            let above = cardinality.maximum.map(|max| count > max).unwrap_or(false);
            if below || above {
                let minimum = cardinality.minimum.map(|value| value.to_string());
                let maximum = cardinality.maximum.map(|value| value.to_string());
                let mut finding = json!({
                    "check": rule.name,
                    "ruleSource": rule.source,
                    "packSource": rule.source,
                    "severity": rule.severity,
                    "layer": rule.layer,
                    "selector": cardinality.region.clone().unwrap_or_else(|| "page".to_string()),
                    "property": "cardinality",
                    "message": format!("{} matched set outside its cardinality", rule.name),
                    "evidence": {
                        "count": count,
                        "min": minimum,
                        "max": maximum,
                        "region": cardinality.region
                    }
                });
                if let Some(lifecycle) = &rule.lifecycle {
                    finding["lifecycle"] = json!(lifecycle);
                }
                findings.push(finding);
            }
        }

        for index in candidates {
            let Some(element) = elements.get(index) else {
                continue;
            };
            if let Some(expression) = &rule.when {
                if !expr_truthy(&eval_expr(expression, element, expr_page, sheet_text)) {
                    continue;
                }
                let mut finding = json!({
                    "check": rule.name,
                    "ruleSource": rule.source,
                    "packSource": rule.source,
                    "severity": rule.severity,
                    "layer": rule.layer,
                    "selector": element["selector"],
                    "property": "when",
                    "message": rule.message.clone().unwrap_or_else(|| format!("{} matched", rule.name)),
                    "evidence": {
                        "expression": expression,
                        "color": element["color"],
                        "backgroundColor": element["backgroundColor"],
                        "fontSize": element["fontSize"],
                        "fontWeight": element["fontWeight"],
                        "text": element["text"],
                        "tag": element["tag"],
                        "rect": element["rect"]
                    }
                });
                if let Some(lifecycle) = &rule.lifecycle {
                    finding["lifecycle"] = json!(lifecycle);
                }
                findings.push(finding);
                continue;
            }
            let mut failures: Vec<(String, String, String)> = Vec::new();
            let mut passes = 0usize;
            for (property, _) in &rule.properties {
                let actual = rule
                    .selectors
                    .iter()
                    .find_map(|selector| oracle.value(selector, property, index))
                    .unwrap_or("")
                    .trim();
                match evaluate_property_rule(rule, property, actual, design) {
                    Some(failure) => failures.push(failure),
                    None => passes += 1,
                }
            }
            let violated = if rule.legacy {
                match rule.operator {
                    Logic::Any => !rule.properties.is_empty() && passes == 0,
                    Logic::One => !rule.properties.is_empty() && failures.len() != 1,
                    Logic::All => !failures.is_empty(),
                }
            } else {
                match rule.operator {
                    Logic::Any => !failures.is_empty(),
                    Logic::One => failures.len() == 1,
                    Logic::All => {
                        !rule.properties.is_empty() && failures.len() == rule.properties.len()
                    }
                }
            };
            if !violated {
                continue;
            }
            let (property, expected, actual) = failures.into_iter().next().unwrap_or((
                "constraint".to_string(),
                "operator".to_string(),
                String::new(),
            ));
            let mut finding = json!({
                "check": rule.name,
                "ruleSource": rule.source,
                "packSource": rule.source,
                "severity": rule.severity,
                "layer": rule.layer,
                "selector": element["selector"],
                "property": property.strip_prefix('@').unwrap_or(&property),
                "message": format!("{} violated", rule.name),
                "evidence": { "expected": expected, "actual": actual }
            });
            if let Some(lifecycle) = &rule.lifecycle {
                finding["lifecycle"] = json!(lifecycle);
            }
            findings.push(finding);
        }
    }

    findings
}

pub(crate) fn apply_check_rules(findings: Vec<Value>, rules: &[CheckRule]) -> Vec<Value> {
    if rules.is_empty() {
        return findings;
    }
    findings
        .into_iter()
        .filter_map(|mut finding| {
            let check = finding["check"].as_str().unwrap_or("").to_string();
            let Some(rule) = rules.iter().find(|rule| rule.id == check) else {
                return Some(finding);
            };
            if !rule.enabled {
                return None;
            }
            if let Some(severity) = &rule.severity {
                finding["severity"] = json!(severity);
            }
            if let Some(layer) = &rule.layer {
                finding["layer"] = json!(layer);
            }
            if !rule.output.is_empty() {
                finding["check"] = json!(rule.output);
            }
            if !rule.source.is_empty() {
                finding["ruleSource"] = json!(rule.source);
                finding["packSource"] = json!(rule.source);
            }
            Some(finding)
        })
        .collect()
}

pub(crate) fn apply_ruleset(findings: Vec<Value>, ruleset: Option<&RuleSet>) -> Vec<Value> {
    match ruleset {
        Some(rules) => apply_check_rules(findings, &rules.checks),
        None => findings,
    }
}

pub(crate) fn apply_unverified_rules(entries: Vec<Value>, ruleset: Option<&RuleSet>) -> Vec<Value> {
    let Some(rules) = ruleset else {
        return entries;
    };
    entries
        .into_iter()
        .filter(|entry| {
            let check = entry["check"].as_str().unwrap_or("");
            !rules
                .checks
                .iter()
                .any(|rule| rule.id == check && !rule.enabled)
        })
        .collect()
}
