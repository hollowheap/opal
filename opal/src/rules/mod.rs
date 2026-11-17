use crate::{
    duration_values, font_families, generic_family, is_overshooting, oracle_from_value,
    parse_color_rgb, parse_css_color, parse_dimension, radius_allowed, DesignSpec, Oracle,
};
use anyhow::{Context, Result};
use regex::Regex;
use serde_json::{json, Value};
use std::borrow::Cow;
use std::cell::RefCell;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub(crate) struct RuleSet {
    structural: Vec<StructuralRule>,
    checks: Vec<CheckRule>,
    source_rules: Vec<SourceRule>,
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

#[derive(Clone, Debug)]
pub(crate) struct SourceRule {
    pub id: String,
    pub severity: String,
    pub layer: String,
    pub message: String,
    pub pattern: Regex,
    pub exclude_paths: Vec<Regex>,
    pub source: String,
}

impl RuleSet {
    pub(crate) fn source_rules(&self) -> &[SourceRule] {
        &self.source_rules
    }

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
    "backdrop-filter",
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
    "transition-property",
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
            "sourceRules",
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
    let mut source_rules: Vec<Value> = Vec::new();
    let mut aliases = serde_json::Map::new();
    let mut rule_aliases = serde_json::Map::new();
    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut source_rule_indexes: HashMap<String, usize> = HashMap::new();
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
        for mut rule in data["sourceRules"].as_array().cloned().unwrap_or_default() {
            let identity = rule["id"].as_str().unwrap_or("").to_string();
            if let Some(object) = rule.as_object_mut() {
                object.insert("_source".to_string(), json!(label));
            }
            if !identity.is_empty() {
                if let Some(&index) = source_rule_indexes.get(&identity) {
                    if let (Some(target), Some(lower)) =
                        (source_rules[index].as_object_mut(), rule.as_object())
                    {
                        for (key, value) in lower {
                            target.entry(key.clone()).or_insert_with(|| value.clone());
                        }
                    }
                    continue;
                }
                source_rule_indexes.insert(identity, source_rules.len());
            }
            source_rules.push(rule);
        }
    }
    let mut merged = json!({
        "version": version,
        "pack": pack.unwrap_or_else(|| "custom".to_string()),
        "structural": structural,
        "rules": named
    });
    if !source_rules.is_empty() {
        merged["sourceRules"] = Value::Array(source_rules);
    }
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
    candidates.push(PathBuf::from("scripts/rules.json"));
    candidates.push(PathBuf::from("scripts/default-rules.json"));
    if let Ok(executable) = std::env::current_exe() {
        let mut directory = executable.parent();
        for _ in 0..6 {
            let Some(current) = directory else {
                break;
            };
            candidates.push(current.join(relative));
            candidates.push(current.join("rules.json"));
            candidates.push(current.join("default-rules.json"));
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
    for key in ["rules", "structural", "sourceRules"] {
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
    let custom_rules = custom["rules"].as_array().cloned().unwrap_or_default();
    let mut extra_rules = Vec::new();
    let mut extra_indexes: HashMap<String, usize> = HashMap::new();
    let mut seen_default_declarative = HashSet::new();
    for rule in custom_rules.iter().filter(|rule| declarative_rule(rule)) {
        let identity = rule_identity(rule);
        if let Some(index) = indexes.get(&identity).copied() {
            if seen_default_declarative.insert(identity) {
                default_rules[index] = rule.clone();
            }
        } else if let std::collections::hash_map::Entry::Vacant(entry) =
            extra_indexes.entry(identity)
        {
            entry.insert(extra_rules.len());
            extra_rules.push(rule.clone());
        }
    }
    let apply_overlay = |target: &mut Value, rule: &Value| {
        let Some(target) = target.as_object_mut() else {
            return;
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
    };
    for rule in custom_rules.iter().filter(|rule| !declarative_rule(rule)) {
        let identity = rule_identity(rule);
        if let Some(index) = indexes.get(&identity).copied() {
            apply_overlay(&mut default_rules[index], rule);
            continue;
        }
        if let Some(index) = extra_indexes.get(&identity).copied() {
            apply_overlay(&mut extra_rules[index], rule);
            continue;
        }
        if custom_version == 1 {
            anyhow::bail!("rules: unknown rule id {identity}");
        }
        extra_rules.push(rule.clone());
    }
    default_rules.extend(extra_rules);
    default["rules"] = Value::Array(default_rules);
    let mut default_source = default["sourceRules"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut source_indexes: HashMap<String, usize> = HashMap::new();
    for (index, rule) in default_source.iter().enumerate() {
        let identity = rule_identity(rule);
        if !identity.is_empty() {
            source_indexes.insert(identity, index);
        }
    }
    let mut extra_source: Vec<Value> = Vec::new();
    let mut extra_source_indexes: HashMap<String, usize> = HashMap::new();
    for rule in custom["sourceRules"]
        .as_array()
        .cloned()
        .unwrap_or_default()
    {
        let identity = rule_identity(&rule);
        if identity.is_empty() {
            anyhow::bail!("rules: source rule needs an id");
        }
        if let Some(index) = source_indexes.get(&identity).copied() {
            if let Some(target) = default_source[index].as_object_mut() {
                if let Some(overlay) = rule.as_object() {
                    for (key, value) in overlay {
                        target.insert(key.clone(), value.clone());
                    }
                }
            }
        } else if let Some(index) = extra_source_indexes.get(&identity).copied() {
            if let Some(target) = extra_source[index].as_object_mut() {
                if let Some(overlay) = rule.as_object() {
                    for (key, value) in overlay {
                        target.insert(key.clone(), value.clone());
                    }
                }
            }
        } else {
            extra_source_indexes.insert(identity, extra_source.len());
            extra_source.push(rule);
        }
    }
    default_source.extend(extra_source);
    default["sourceRules"] = Value::Array(default_source);
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
    let mut source_rules = Vec::new();
    for rule in data["sourceRules"].as_array().cloned().unwrap_or_default() {
        check_keys(
            &rule,
            &[
                "id",
                "enabled",
                "severity",
                "layer",
                "message",
                "pattern",
                "excludePaths",
                "_source",
            ],
            "source rule",
        )?;
        let id = rule["id"].as_str().unwrap_or("").trim().to_string();
        if id.is_empty() {
            anyhow::bail!("rules: source rule needs an id");
        }
        let source = rule["_source"].as_str().unwrap_or("").to_string();
        if rule["enabled"].as_bool() == Some(false) {
            continue;
        }
        let severity = rule["severity"].as_str().unwrap_or("P3").to_string();
        let layer = rule["layer"].as_str().unwrap_or("source").to_string();
        validate_severity_layer(&severity, &layer)?;
        let message = rule["message"]
            .as_str()
            .filter(|value| !value.trim().is_empty())
            .with_context(|| format!("rules: source rule {id} needs a message"))?
            .to_string();
        let pattern_raw = rule["pattern"]
            .as_str()
            .filter(|value| !value.is_empty())
            .with_context(|| format!("rules: source rule {id} needs a pattern"))?;
        let pattern = Regex::new(pattern_raw)
            .with_context(|| format!("rules: invalid regex in source rule {id}"))?;
        let mut exclude_paths = Vec::new();
        if let Some(list) = rule.get("excludePaths").and_then(Value::as_array) {
            for item in list {
                let pat = item
                    .as_str()
                    .filter(|value| !value.is_empty())
                    .with_context(|| format!("rules: invalid excludePaths entry in {id}"))?;
                exclude_paths.push(
                    Regex::new(pat)
                        .with_context(|| format!("rules: invalid excludePaths regex in {id}"))?,
                );
            }
        }
        source_rules.push(SourceRule {
            id,
            severity,
            layer,
            message,
            pattern,
            exclude_paths,
            source,
        });
    }

    Ok((
        RuleSet {
            structural,
            checks,
            source_rules,
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
    value == "none"
        && (property == "border" || property == "outline" || property == "background-image")
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

#[derive(Clone, Copy)]
enum EvalScope<'a> {
    None,
    Pair { left: &'a Value, right: &'a Value },
    Group { key: &'a Value, values: &'a Value },
}

#[derive(Clone, Copy)]
struct EvalEnv<'a> {
    element: &'a Value,
    page: &'a Value,
    sheet: &'a str,
    rule_params: &'a Value,
    parents: &'a [Option<usize>],
    scope: EvalScope<'a>,
}

impl<'a> EvalEnv<'a> {
    fn for_root(
        element: &'a Value,
        page: &'a Value,
        sheet: &'a str,
        rule_params: &'a Value,
        parents: &'a [Option<usize>],
    ) -> Self {
        Self {
            element,
            page,
            sheet,
            rule_params,
            parents,
            scope: EvalScope::None,
        }
    }

    fn with_element(&self, element: &'a Value) -> Self {
        Self {
            element,
            page: self.page,
            sheet: self.sheet,
            rule_params: self.rule_params,
            parents: self.parents,
            scope: EvalScope::None,
        }
    }

    fn with_pair(&self, left: &'a Value, right: &'a Value) -> Self {
        Self {
            element: self.element,
            page: self.page,
            sheet: self.sheet,
            rule_params: self.rule_params,
            parents: self.parents,
            scope: EvalScope::Pair { left, right },
        }
    }

    fn with_group(&self, key: &'a Value, values: &'a Value) -> Self {
        Self {
            element: self.element,
            page: self.page,
            sheet: self.sheet,
            rule_params: self.rule_params,
            parents: self.parents,
            scope: EvalScope::Group { key, values },
        }
    }
}

fn expr_field_cow<'a>(path: &str, env: &EvalEnv<'a>) -> Cow<'a, Value> {
    if let Some(rest) = path.strip_prefix("page.") {
        if rest == "ruleParams" {
            return Cow::Borrowed(env.rule_params);
        }
        if let Some(param_path) = rest.strip_prefix("ruleParams.") {
            return Cow::Borrowed(expr_path(env.rule_params, param_path));
        }
        return Cow::Borrowed(expr_path(env.page, rest));
    }
    if path == "page" {
        return Cow::Borrowed(env.page);
    }
    if path == "sheet" || path == "sheet.text" {
        return Cow::Owned(json!(env.sheet));
    }
    match env.scope {
        EvalScope::None => {
            if path == "self" || path == "." {
                Cow::Borrowed(env.element)
            } else {
                Cow::Borrowed(expr_path(env.element, path))
            }
        }
        EvalScope::Pair { left, right } => {
            if path == "left" {
                Cow::Borrowed(left)
            } else if let Some(rest) = path.strip_prefix("left.") {
                Cow::Borrowed(expr_path(left, rest))
            } else if path == "right" {
                Cow::Borrowed(right)
            } else if let Some(rest) = path.strip_prefix("right.") {
                Cow::Borrowed(expr_path(right, rest))
            } else if path == "self" || path == "." {
                Cow::Owned(json!({ "left": left, "right": right }))
            } else {
                Cow::Borrowed(&Value::Null)
            }
        }
        EvalScope::Group { key, values } => {
            if path == "key" {
                Cow::Borrowed(key)
            } else if let Some(rest) = path.strip_prefix("key.") {
                Cow::Borrowed(expr_path(key, rest))
            } else if path == "values" {
                Cow::Borrowed(values)
            } else if let Some(rest) = path.strip_prefix("values.") {
                Cow::Borrowed(expr_path(values, rest))
            } else if path == "self" || path == "." {
                Cow::Owned(json!({ "key": key, "values": values }))
            } else {
                Cow::Borrowed(&Value::Null)
            }
        }
    }
}

fn operand_args(operand: &Value) -> &[Value] {
    match operand {
        Value::Array(values) => values.as_slice(),
        other => std::slice::from_ref(other),
    }
}

fn cached_regex_with<R>(pattern: &str, f: impl FnOnce(&Regex) -> R) -> Option<R> {
    thread_local! {
        static REGEX_CACHE: RefCell<HashMap<String, Option<Regex>>> = RefCell::new(HashMap::new());
    }
    REGEX_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if !cache.contains_key(pattern) {
            if cache.len() >= 512 {
                cache.clear();
            }
            cache.insert(pattern.to_string(), Regex::new(pattern).ok());
        }
        cache.get(pattern).and_then(|entry| entry.as_ref()).map(f)
    })
}

#[derive(Hash, PartialEq, Eq)]
enum ExactKey {
    Null,
    Bool(bool),
    Int(i128),
    FloatBits(u64),
    Str(String),
    Complex(String),
}

fn exact_key(value: &Value) -> ExactKey {
    match value {
        Value::Null => ExactKey::Null,
        Value::Bool(value) => ExactKey::Bool(*value),
        Value::Number(number) => {
            if let Some(value) = number.as_u64() {
                ExactKey::Int(value as i128)
            } else if let Some(value) = number.as_i64() {
                ExactKey::Int(value as i128)
            } else if let Some(value) = number.as_f64() {
                ExactKey::FloatBits(if value == 0.0 { 0 } else { value.to_bits() })
            } else {
                ExactKey::Complex(number.to_string())
            }
        }
        Value::String(value) => ExactKey::Str(value.clone()),
        other => ExactKey::Complex(other.to_string()),
    }
}

#[derive(Hash, PartialEq, Eq)]
enum JoinBucket {
    Null,
    Bool(bool),
    NumBin(i64),
    Str(String),
    Fallback,
}

fn join_bucket(value: &Value) -> JoinBucket {
    if let Some(number) = expr_number(value) {
        if number.is_finite() && number.abs() < 1e13 {
            return JoinBucket::NumBin((number * 500.0).round() as i64);
        }
        return JoinBucket::Fallback;
    }
    match value {
        Value::Null => JoinBucket::Null,
        Value::Bool(value) => JoinBucket::Bool(*value),
        Value::String(value) => JoinBucket::Str(value.to_ascii_lowercase()),
        _ => JoinBucket::Fallback,
    }
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

fn expr_text_cow(value: &Value) -> Cow<'_, str> {
    match value {
        Value::String(value) => Cow::Borrowed(value.as_str()),
        other => Cow::Owned(other.to_string()),
    }
}

fn expr_colors_str(text: &str) -> Vec<(f64, f64, f64, f64)> {
    static HEX_RE: OnceLock<Regex> = OnceLock::new();
    static RGB_RE: OnceLock<Regex> = OnceLock::new();
    let hex_re =
        HEX_RE.get_or_init(|| Regex::new(r"#[0-9a-fA-F]{3,8}\b").expect("valid hex regex"));
    let rgb_re = RGB_RE.get_or_init(|| Regex::new(r"rgba?\([^)]*\)").expect("valid rgb regex"));
    let mut out = Vec::new();
    for regex in [hex_re, rgb_re] {
        for capture in regex.find_iter(text) {
            if let Some(color) = parse_color_alpha(capture.as_str()) {
                out.push(color);
            }
        }
    }
    if out.is_empty() {
        if let Some(color) = parse_color_alpha(text) {
            out.push(color);
        }
    }
    out
}

fn expr_colors(value: &Value) -> Vec<(f64, f64, f64, f64)> {
    let text = expr_text_cow(value);
    expr_colors_str(&text)
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
    static PX_RE: OnceLock<Regex> = OnceLock::new();
    let text = expr_text_cow(value);
    let first = text.split("),").next().unwrap_or(text.as_ref());
    let inset = first.contains("inset");
    let regex = PX_RE.get_or_init(|| Regex::new(r"(-?\d*\.?\d+)px").expect("valid px regex"));
    let numbers: Vec<f64> = regex
        .captures_iter(first)
        .filter_map(|capture| capture[1].parse::<f64>().ok())
        .collect();
    if numbers.len() < 3 {
        return None;
    }
    let color = expr_colors_str(first)
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

fn collect_all_conjuncts<'a>(expr: &'a Value, out: &mut Vec<&'a Value>) {
    if let Some(object) = expr.as_object() {
        if object.len() == 1 {
            if let Some((op, operand)) = object.iter().next() {
                if matches!(op.as_str(), "all" | "and") {
                    for item in operand_args(operand) {
                        collect_all_conjuncts(item, out);
                    }
                    return;
                }
            }
        }
    }
    out.push(expr);
}

fn expr_scope_usage(expr: &Value) -> (bool, bool, bool) {
    let Some(object) = expr.as_object() else {
        return (false, false, false);
    };
    if let Some(path) = object.get("field").and_then(Value::as_str) {
        if path == "page" || path.starts_with("page.") || path == "sheet" || path == "sheet.text" {
            return (false, false, false);
        }
        if path == "left" || path.starts_with("left.") {
            return (true, false, false);
        }
        if path == "right" || path.starts_with("right.") {
            return (false, true, false);
        }
        return (false, false, true);
    }
    if object.len() != 1 {
        return (false, false, false);
    }
    let (op, operand) = object.iter().next().unwrap();
    if matches!(
        op.as_str(),
        "nearestCard"
            | "contrast"
            | "map"
            | "flatMap"
            | "filter"
            | "groupAny"
            | "groupAll"
            | "firstOf"
            | "pairAny"
            | "adjacentAny"
            | "adjacentAll"
            | "anyOf"
            | "allOf"
            | "countOf"
            | "sumOf"
    ) {
        return (false, false, true);
    }
    let mut left = false;
    let mut right = false;
    let mut opaque = false;
    for arg in operand_args(operand) {
        let (l, r, o) = expr_scope_usage(arg);
        left |= l;
        right |= r;
        opaque |= o;
    }
    (left, right, opaque)
}

#[derive(Copy, Clone, PartialEq)]
struct OrdF64(f64);

impl Eq for OrdF64 {}

impl PartialOrd for OrdF64 {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OrdF64 {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

struct IntervalBound<'a> {
    left_start: &'a Value,
    left_end: &'a Value,
    right_start: &'a Value,
    right_end: &'a Value,
    threshold: f64,
}

fn single_op_args<'a>(expr: &'a Value, expected: &[&str]) -> Option<&'a [Value]> {
    let object = expr.as_object()?;
    if object.len() != 1 {
        return None;
    }
    let (op, operand) = object.iter().next()?;
    if !expected.contains(&op.as_str()) {
        return None;
    }
    operand.as_array().map(Vec::as_slice)
}

fn split_left_right_pair<'a>(a: &'a Value, b: &'a Value) -> Option<(&'a Value, &'a Value)> {
    match (expr_scope_usage(a), expr_scope_usage(b)) {
        ((true, false, false), (false, true, false)) => Some((a, b)),
        ((false, true, false), (true, false, false)) => Some((b, a)),
        _ => None,
    }
}

fn extract_interval_bound<'a>(cond: &'a Value, env: &EvalEnv<'_>) -> Option<IntervalBound<'a>> {
    let outer = single_op_args(cond, &["gt", "gte", "atLeast"])?;
    if outer.len() != 2 || expr_scope_usage(&outer[1]) != (false, false, false) {
        return None;
    }
    let threshold = eval_num(&outer[1], env)?;
    if !threshold.is_finite() || threshold < 0.0 {
        return None;
    }
    let sub_args = single_op_args(&outer[0], &["sub"])?;
    if sub_args.len() != 2 {
        return None;
    }
    let min_args = single_op_args(&sub_args[0], &["min"])?;
    let max_args = single_op_args(&sub_args[1], &["max"])?;
    if min_args.len() != 2 || max_args.len() != 2 {
        return None;
    }
    let (left_end, right_end) = split_left_right_pair(&min_args[0], &min_args[1])?;
    let (left_start, right_start) = split_left_right_pair(&max_args[0], &max_args[1])?;
    Some(IntervalBound {
        left_start,
        left_end,
        right_start,
        right_end,
        threshold,
    })
}

fn pair_conds_match(
    pair_conds: &[&Value],
    left_item: &Value,
    right_item: &Value,
    env: &EvalEnv<'_>,
) -> bool {
    let pair_env = env.with_pair(left_item, right_item);
    pair_conds
        .iter()
        .all(|cond| expr_truthy(&eval_ref(cond, &pair_env)))
}

fn try_ancestor_join(
    left: &[usize],
    right: &[usize],
    pair_conds: &[&Value],
    items: &[Value],
    env: &EvalEnv<'_>,
) -> Option<bool> {
    if env.parents.is_empty() {
        return None;
    }
    let (left_idx_expr, right_idx_expr) = pair_conds.iter().find_map(|cond| {
        let args = single_op_args(cond, &["ancestor"])?;
        if args.len() != 2 {
            return None;
        }
        if expr_scope_usage(&args[0]) == (true, false, false)
            && expr_scope_usage(&args[1]) == (false, true, false)
        {
            Some((&args[0], &args[1]))
        } else {
            None
        }
    })?;

    let mut left_by_node: HashMap<usize, Vec<usize>> = HashMap::with_capacity(left.len());
    for &i in left {
        let value = eval_num(left_idx_expr, &env.with_pair(&items[i], &Value::Null))?;
        if !value.is_finite() || value < 0.0 || value.fract() != 0.0 {
            return None;
        }
        let node = value as usize;
        if node >= env.parents.len() {
            return None;
        }
        left_by_node.entry(node).or_default().push(i);
    }

    let mut right_nodes = Vec::with_capacity(right.len());
    for &j in right {
        let value = eval_num(right_idx_expr, &env.with_pair(&Value::Null, &items[j]))?;
        if !value.is_finite() || value < 0.0 || value.fract() != 0.0 {
            return None;
        }
        let node = value as usize;
        if node >= env.parents.len() {
            return None;
        }
        right_nodes.push((j, node));
    }

    for (j, start_node) in right_nodes {
        let mut current = env.parents[start_node];
        let mut depth = 0usize;
        while let Some(parent) = current {
            if let Some(left_candidates) = left_by_node.get(&parent) {
                let end = left_candidates.partition_point(|&i| i < j);
                for &i in &left_candidates[..end] {
                    if pair_conds_match(pair_conds, &items[i], &items[j], env) {
                        return Some(true);
                    }
                }
            }
            depth += 1;
            if depth >= env.parents.len() {
                break;
            }
            current = env.parents.get(parent).copied().flatten();
        }
    }
    Some(false)
}

fn try_interval_sweep(
    left: &[usize],
    right: &[usize],
    pair_conds: &[&Value],
    items: &[Value],
    env: &EvalEnv<'_>,
) -> Option<bool> {
    const EPS: f64 = 1e-6;
    let bounds: Vec<IntervalBound<'_>> = pair_conds
        .iter()
        .filter_map(|cond| extract_interval_bound(cond, env))
        .collect();
    if bounds.is_empty() {
        return None;
    }

    type SweepIntervals = Vec<(usize, f64, f64)>;
    let mut best_axis: Option<(f64, SweepIntervals, SweepIntervals)> = None;
    for bound in &bounds {
        let mut l_intervals = Vec::with_capacity(left.len());
        let mut r_intervals = Vec::with_capacity(right.len());
        let mut min_start = f64::INFINITY;
        let mut max_end = f64::NEG_INFINITY;
        let mut sum_len = 0.0;

        for &i in left {
            let l_env = env.with_pair(&items[i], &Value::Null);
            let s = eval_num(bound.left_start, &l_env)?;
            let e = eval_num(bound.left_end, &l_env)?;
            if !s.is_finite() || !e.is_finite() {
                return None;
            }
            let trim_end = e - bound.threshold + EPS;
            if trim_end >= s {
                min_start = min_start.min(s);
                max_end = max_end.max(trim_end);
                sum_len += trim_end - s;
                l_intervals.push((i, s, trim_end));
            }
        }
        if l_intervals.is_empty() {
            return Some(false);
        }

        for &j in right {
            let r_env = env.with_pair(&Value::Null, &items[j]);
            let s = eval_num(bound.right_start, &r_env)?;
            let e = eval_num(bound.right_end, &r_env)?;
            if !s.is_finite() || !e.is_finite() {
                return None;
            }
            let trim_end = e - bound.threshold + EPS;
            if trim_end >= s {
                min_start = min_start.min(s);
                max_end = max_end.max(trim_end);
                sum_len += trim_end - s;
                r_intervals.push((j, s, trim_end));
            }
        }
        if r_intervals.is_empty() {
            return Some(false);
        }

        let span = (max_end - min_start).max(1.0);
        let density = sum_len / span;
        if best_axis
            .as_ref()
            .map(|(best_d, _, _)| density < *best_d)
            .unwrap_or(true)
        {
            best_axis = Some((density, l_intervals, r_intervals));
        }
    }

    let (_, mut l_intervals, mut r_intervals) = best_axis?;
    l_intervals.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    r_intervals.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));

    struct ActivePool {
        items: Vec<usize>,
        pos: Vec<usize>,
        heap: BinaryHeap<Reverse<(OrdF64, usize)>>,
    }

    impl ActivePool {
        fn with_capacity(count: usize) -> Self {
            Self {
                items: Vec::new(),
                pos: vec![usize::MAX; count],
                heap: BinaryHeap::new(),
            }
        }

        fn evict_before(&mut self, start: f64) {
            while let Some(&Reverse((end, slot))) = self.heap.peek() {
                if end.0 < start {
                    self.heap.pop();
                    let p = self.pos[slot];
                    if p != usize::MAX {
                        let last = self.items.pop().expect("non-empty active pool");
                        if p < self.items.len() {
                            self.items[p] = last;
                            self.pos[last] = p;
                        }
                        self.pos[slot] = usize::MAX;
                    }
                } else {
                    break;
                }
            }
        }

        fn insert(&mut self, slot: usize, end: f64) {
            self.pos[slot] = self.items.len();
            self.items.push(slot);
            self.heap.push(Reverse((OrdF64(end), slot)));
        }
    }

    let mut active_left = ActivePool::with_capacity(l_intervals.len());
    let mut active_right = ActivePool::with_capacity(r_intervals.len());
    let mut li = 0usize;
    let mut ri = 0usize;

    while li < l_intervals.len() || ri < r_intervals.len() {
        let take_left = match (l_intervals.get(li), r_intervals.get(ri)) {
            (Some(l), Some(r)) => l.1.total_cmp(&r.1) != std::cmp::Ordering::Greater,
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => break,
        };
        if take_left {
            let (i, start, end) = l_intervals[li];
            active_left.evict_before(start);
            active_right.evict_before(start);
            for &r_slot in &active_right.items {
                let j = r_intervals[r_slot].0;
                if i < j && pair_conds_match(pair_conds, &items[i], &items[j], env) {
                    return Some(true);
                }
            }
            active_left.insert(li, end);
            li += 1;
        } else {
            let (j, start, end) = r_intervals[ri];
            active_left.evict_before(start);
            active_right.evict_before(start);
            for &l_slot in &active_left.items {
                let i = l_intervals[l_slot].0;
                if i < j && pair_conds_match(pair_conds, &items[i], &items[j], env) {
                    return Some(true);
                }
            }
            active_right.insert(ri, end);
            ri += 1;
        }
    }
    Some(false)
}

fn scan_pair_candidates(
    left: &[usize],
    right: &[usize],
    pair_conds: &[&Value],
    items: &[Value],
    env: &EvalEnv<'_>,
) -> bool {
    if left.is_empty() || right.is_empty() {
        return false;
    }
    if left.len().saturating_mul(right.len()) > 64 {
        if let Some(found) = try_ancestor_join(left, right, pair_conds, items, env) {
            return found;
        }
        if let Some(found) = try_interval_sweep(left, right, pair_conds, items, env) {
            return found;
        }
    }
    for &i in left {
        let start = right.partition_point(|&j| j <= i);
        for &j in &right[start..] {
            if pair_conds_match(pair_conds, &items[i], &items[j], env) {
                return true;
            }
        }
    }
    false
}

fn eval_pair_any(items_expr: &Value, predicate: &Value, env: &EvalEnv<'_>) -> bool {
    let list_val = eval_ref(items_expr, env);
    let items = list_val.as_array().map(Vec::as_slice).unwrap_or(&[]);
    if items.len() < 2 {
        return false;
    }

    let mut conjuncts = Vec::new();
    collect_all_conjuncts(predicate, &mut conjuncts);
    if conjuncts.is_empty() {
        return true;
    }

    let mut const_conds = Vec::new();
    let mut left_conds = Vec::new();
    let mut right_conds = Vec::new();
    let mut pair_conds = Vec::new();
    for cond in conjuncts {
        match expr_scope_usage(cond) {
            (false, false, false) => const_conds.push(cond),
            (true, false, false) => left_conds.push(cond),
            (false, true, false) => right_conds.push(cond),
            _ => pair_conds.push(cond),
        }
    }

    for cond in const_conds {
        if !expr_truthy(&eval_ref(cond, env)) {
            return false;
        }
    }

    let left_indices: Vec<usize> = (0..items.len())
        .filter(|&i| {
            let pair_env = env.with_pair(&items[i], &Value::Null);
            left_conds
                .iter()
                .all(|cond| expr_truthy(&eval_ref(cond, &pair_env)))
        })
        .collect();
    if left_indices.is_empty() {
        return false;
    }

    let right_indices: Vec<usize> = (0..items.len())
        .filter(|&j| {
            let pair_env = env.with_pair(&Value::Null, &items[j]);
            right_conds
                .iter()
                .all(|cond| expr_truthy(&eval_ref(cond, &pair_env)))
        })
        .collect();
    if right_indices.is_empty() {
        return false;
    }

    let join_keys = pair_conds.iter().find_map(|cond| {
        let args = single_op_args(cond, &["equals", "eq"])?;
        if args.len() != 2 {
            return None;
        }
        split_left_right_pair(&args[0], &args[1])
    });

    if let Some((left_key_expr, right_key_expr)) = join_keys {
        let mut right_buckets: HashMap<JoinBucket, Vec<usize>> = HashMap::new();
        let mut all_right_bucketed = true;
        for &j in &right_indices {
            let value = eval_ref(right_key_expr, &env.with_pair(&Value::Null, &items[j]));
            let bucket = join_bucket(&value);
            if matches!(bucket, JoinBucket::Fallback) {
                all_right_bucketed = false;
                break;
            }
            right_buckets.entry(bucket).or_default().push(j);
        }
        if all_right_bucketed {
            let mut left_buckets: HashMap<JoinBucket, Vec<usize>> = HashMap::new();
            let mut fallback_left: Vec<usize> = Vec::new();
            for &i in &left_indices {
                let left_val = eval_ref(left_key_expr, &env.with_pair(&items[i], &Value::Null));
                match join_bucket(&left_val) {
                    JoinBucket::Fallback => fallback_left.push(i),
                    key => left_buckets.entry(key).or_default().push(i),
                }
            }
            if !fallback_left.is_empty()
                && scan_pair_candidates(&fallback_left, &right_indices, &pair_conds, items, env)
            {
                return true;
            }
            for (bucket, left_group) in left_buckets {
                match bucket {
                    JoinBucket::NumBin(bin) => {
                        for probe in [bin - 1, bin, bin + 1] {
                            if let Some(right_slice) = right_buckets.get(&JoinBucket::NumBin(probe))
                            {
                                if scan_pair_candidates(
                                    &left_group,
                                    right_slice,
                                    &pair_conds,
                                    items,
                                    env,
                                ) {
                                    return true;
                                }
                            }
                        }
                    }
                    JoinBucket::Fallback => unreachable!(),
                    key => {
                        if let Some(right_slice) = right_buckets.get(&key) {
                            if scan_pair_candidates(
                                &left_group,
                                right_slice,
                                &pair_conds,
                                items,
                                env,
                            ) {
                                return true;
                            }
                        }
                    }
                }
            }
            return false;
        }
    }

    scan_pair_candidates(&left_indices, &right_indices, &pair_conds, items, env)
}

fn eval_ancestor(left_id: f64, right_id: f64, env: &EvalEnv<'_>) -> bool {
    if !left_id.is_finite() || !right_id.is_finite() || left_id < 0.0 || right_id < 0.0 {
        return false;
    }
    let target = left_id as u64;
    let start = right_id as usize;
    if !env.parents.is_empty() {
        let mut current = env.parents.get(start).copied().flatten();
        while let Some(parent) = current {
            if parent as u64 == target {
                return true;
            }
            current = env.parents.get(parent).copied().flatten();
        }
        return false;
    }
    let Some(elements) = env.page["elements"].as_array() else {
        return false;
    };
    let mut current = elements
        .get(start)
        .and_then(|element| element["parentIndex"].as_u64());
    while let Some(parent) = current {
        if parent == target {
            return true;
        }
        current = elements
            .get(parent as usize)
            .and_then(|element| element["parentIndex"].as_u64());
    }
    false
}

fn eval_num(expr: &Value, env: &EvalEnv<'_>) -> Option<f64> {
    match expr {
        Value::Number(number) => number.as_f64(),
        Value::String(raw) => raw
            .trim()
            .parse::<f64>()
            .ok()
            .or_else(|| parse_dimension(raw).map(|(number, _)| number)),
        Value::Bool(value) => Some(if *value { 1.0 } else { 0.0 }),
        Value::Object(object) => {
            if let Some(path) = object.get("field").and_then(Value::as_str) {
                return expr_number(&expr_field_cow(path, env));
            }
            if object.len() != 1 {
                return None;
            }
            let (op, operand) = object.iter().next().unwrap();
            let args = operand_args(operand);
            match op.as_str() {
                "add" => Some(args.iter().filter_map(|arg| eval_num(arg, env)).sum()),
                "sub" => Some(
                    args.first()
                        .and_then(|arg| eval_num(arg, env))
                        .unwrap_or(0.0)
                        - args
                            .get(1)
                            .and_then(|arg| eval_num(arg, env))
                            .unwrap_or(0.0),
                ),
                "mul" => Some(args.iter().filter_map(|arg| eval_num(arg, env)).product()),
                "div" => {
                    let denominator = args
                        .get(1)
                        .and_then(|arg| eval_num(arg, env))
                        .unwrap_or(1.0);
                    Some(
                        args.first()
                            .and_then(|arg| eval_num(arg, env))
                            .unwrap_or(0.0)
                            / denominator,
                    )
                }
                "min" | "max" if matches!(operand, Value::Array(list) if list.len() > 1) => {
                    let is_min = op == "min";
                    let mut acc = if is_min {
                        f64::INFINITY
                    } else {
                        f64::NEG_INFINITY
                    };
                    for arg in args {
                        if let Some(number) = eval_num(arg, env) {
                            acc = if is_min {
                                acc.min(number)
                            } else {
                                acc.max(number)
                            };
                        }
                    }
                    Some(if acc.is_finite() { acc } else { 0.0 })
                }
                "round" => args
                    .first()
                    .and_then(|arg| eval_num(arg, env))
                    .map(f64::round),
                _ => expr_number(&eval_ref(expr, env)),
            }
        }
        _ => None,
    }
}

#[cfg(test)]
fn eval_expr(expr: &Value, element: &Value, page: &Value, sheet: &str) -> Value {
    let env = EvalEnv::for_root(element, page, sheet, &page["ruleParams"], &[]);
    eval_ref(expr, &env).into_owned()
}

fn eval_ref<'a>(expr: &'a Value, env: &EvalEnv<'a>) -> Cow<'a, Value> {
    let Some(object) = expr.as_object() else {
        return Cow::Borrowed(expr);
    };
    if let Some(path) = object.get("field").and_then(Value::as_str) {
        return expr_field_cow(path, env);
    }
    if object.len() != 1 {
        return Cow::Owned(Value::Null);
    }
    let (op, operand) = object.iter().next().unwrap();
    let args = operand_args(operand);
    match op.as_str() {
        "map" | "flatMap" | "filter" => {
            let Some(items_expr) = args.first() else {
                return Cow::Owned(Value::Array(Vec::new()));
            };
            let Some(predicate) = args.get(1) else {
                return Cow::Owned(Value::Array(Vec::new()));
            };
            let list_val = eval_ref(items_expr, env);
            let items = list_val.as_array().map(Vec::as_slice).unwrap_or(&[]);
            let mut out = Vec::new();
            for item in items {
                let value = eval_ref(predicate, &env.with_element(item));
                match op.as_str() {
                    "filter" => {
                        if expr_truthy(&value) {
                            out.push(item.clone());
                        }
                    }
                    "flatMap" => match value.into_owned() {
                        Value::Array(values) => out.extend(values),
                        other => out.push(other),
                    },
                    _ => out.push(value.into_owned()),
                }
            }
            Cow::Owned(Value::Array(out))
        }
        "groupAny" | "groupAll" => {
            let (Some(items_expr), Some(key_expression), Some(predicate)) =
                (args.first(), args.get(1), args.get(2))
            else {
                return Cow::Owned(Value::Bool(false));
            };
            let list_val = eval_ref(items_expr, env);
            let items = list_val.as_array().map(Vec::as_slice).unwrap_or(&[]);
            let mut groups: Vec<(Value, Vec<Value>)> = Vec::new();
            let mut key_index: HashMap<ExactKey, usize> = HashMap::new();
            for item in items {
                let key = eval_ref(key_expression, &env.with_element(item));
                let exact = exact_key(&key);
                if let Some(&idx) = key_index.get(&exact) {
                    groups[idx].1.push(item.clone());
                } else {
                    let idx = groups.len();
                    key_index.insert(exact, idx);
                    groups.push((key.into_owned(), vec![item.clone()]));
                }
            }
            let total = groups.len();
            let is_any = op == "groupAny";
            let mut matched = 0usize;
            for (key, values) in groups {
                let values_array = Value::Array(values);
                let ok = expr_truthy(&eval_ref(predicate, &env.with_group(&key, &values_array)));
                if is_any && ok {
                    return Cow::Owned(Value::Bool(true));
                }
                if !is_any && !ok {
                    return Cow::Owned(Value::Bool(false));
                }
                if ok {
                    matched += 1;
                }
            }
            Cow::Owned(Value::Bool(if is_any {
                matched > 0
            } else {
                matched == total
            }))
        }
        "firstOf" => {
            let (Some(items_expr), Some(predicate)) = (args.first(), args.get(1)) else {
                return Cow::Owned(Value::Null);
            };
            let list_val = eval_ref(items_expr, env);
            let items = list_val.as_array().map(Vec::as_slice).unwrap_or(&[]);
            for item in items {
                if expr_truthy(&eval_ref(predicate, &env.with_element(item))) {
                    return Cow::Owned(item.clone());
                }
            }
            Cow::Owned(Value::Null)
        }
        "pairAny" => {
            let (Some(items_expr), Some(predicate)) = (args.first(), args.get(1)) else {
                return Cow::Owned(Value::Bool(false));
            };
            Cow::Owned(Value::Bool(eval_pair_any(items_expr, predicate, env)))
        }
        "adjacentAny" | "adjacentAll" => {
            let (Some(items_expr), Some(predicate)) = (args.first(), args.get(1)) else {
                return Cow::Owned(Value::Null);
            };
            let list_val = eval_ref(items_expr, env);
            let items = list_val.as_array().map(Vec::as_slice).unwrap_or(&[]);
            let is_any = op == "adjacentAny";
            for pair in items.windows(2) {
                let ok = expr_truthy(&eval_ref(predicate, &env.with_pair(&pair[0], &pair[1])));
                if is_any && ok {
                    return Cow::Owned(Value::Bool(true));
                }
                if !is_any && !ok {
                    return Cow::Owned(Value::Bool(false));
                }
            }
            Cow::Owned(Value::Bool(!is_any))
        }
        "anyOf" | "allOf" | "countOf" | "sumOf" => {
            let (Some(items_expr), Some(predicate)) = (args.first(), args.get(1)) else {
                return Cow::Owned(Value::Null);
            };
            let list_val = eval_ref(items_expr, env);
            let items = list_val.as_array().map(Vec::as_slice).unwrap_or(&[]);
            let mut matched = 0usize;
            let mut sum = 0.0;
            for item in items {
                let value = eval_ref(predicate, &env.with_element(item));
                let truthy = expr_truthy(&value);
                match op.as_str() {
                    "anyOf" if truthy => return Cow::Owned(Value::Bool(true)),
                    "allOf" if !truthy => return Cow::Owned(Value::Bool(false)),
                    "sumOf" => sum += expr_number(&value).unwrap_or(0.0),
                    _ if truthy => matched += 1,
                    _ => {}
                }
            }
            Cow::Owned(match op.as_str() {
                "anyOf" => Value::Bool(false),
                "allOf" => Value::Bool(true),
                "countOf" => json!(matched),
                _ => json!(sum),
            })
        }
        "all" | "and" => Cow::Owned(Value::Bool(
            args.iter().all(|arg| expr_truthy(&eval_ref(arg, env))),
        )),
        "any" | "or" => Cow::Owned(Value::Bool(
            args.iter().any(|arg| expr_truthy(&eval_ref(arg, env))),
        )),
        "not" => Cow::Owned(Value::Bool(
            !args
                .first()
                .map(|arg| expr_truthy(&eval_ref(arg, env)))
                .unwrap_or(false),
        )),
        "if" => {
            let cond = args
                .first()
                .map(|arg| expr_truthy(&eval_ref(arg, env)))
                .unwrap_or(false);
            let branch = if cond { args.get(1) } else { args.get(2) };
            match branch {
                Some(expr) => eval_ref(expr, env),
                None => Cow::Owned(Value::Null),
            }
        }
        "get" => {
            let (Some(target_expr), Some(path_expr)) = (args.first(), args.get(1)) else {
                return Cow::Owned(Value::Null);
            };
            let path = expr_text_cow(&eval_ref(path_expr, env)).into_owned();
            match eval_ref(target_expr, env) {
                Cow::Borrowed(value) => Cow::Borrowed(expr_path(value, &path)),
                Cow::Owned(value) => Cow::Owned(expr_path(&value, &path).clone()),
            }
        }
        "ancestor" => {
            let found = if let (Some(arg0), Some(arg1)) = (args.first(), args.get(1)) {
                if let (Some(left), Some(right)) = (eval_num(arg0, env), eval_num(arg1, env)) {
                    eval_ancestor(left, right, env)
                } else {
                    false
                }
            } else {
                false
            };
            Cow::Owned(Value::Bool(found))
        }
        "param" => {
            let key = args
                .first()
                .map(|arg| expr_text_cow(&eval_ref(arg, env)).into_owned())
                .unwrap_or_default();
            Cow::Borrowed(env.rule_params.get(&key).unwrap_or(&Value::Null))
        }
        "contrast" => Cow::Owned(
            crate::contrast_ratio(env.element)
                .map(|value| json!(value))
                .unwrap_or(Value::Null),
        ),
        "shadowInset" => Cow::Owned(Value::Bool(
            args.first()
                .and_then(|arg| expr_shadow(&eval_ref(arg, env)))
                .map(|shadow| shadow.5)
                .unwrap_or(false),
        )),
        "shadowBlur" => Cow::Owned(json!(args
            .first()
            .and_then(|arg| expr_shadow(&eval_ref(arg, env)))
            .map(|shadow| shadow.2)
            .unwrap_or(0.0))),
        "shadowOffset" => Cow::Owned(json!(args
            .first()
            .and_then(|arg| expr_shadow(&eval_ref(arg, env)))
            .map(|shadow| (shadow.0 * shadow.0 + shadow.1 * shadow.1).sqrt())
            .unwrap_or(0.0))),
        "shadowAlpha" => Cow::Owned(json!(args
            .first()
            .and_then(|arg| expr_shadow(&eval_ref(arg, env)))
            .map(|shadow| shadow.4 .3)
            .unwrap_or(0.0))),
        "shadowChromatic" => Cow::Owned(Value::Bool(
            args.first()
                .and_then(|arg| expr_shadow(&eval_ref(arg, env)))
                .map(|shadow| expr_chroma(shadow.4) > 25.0 && shadow.4 .3 > 0.2)
                .unwrap_or(false),
        )),
        "alpha" => Cow::Owned(json!(args
            .first()
            .and_then(|arg| expr_color(&eval_ref(arg, env)))
            .map(|value| value.3)
            .unwrap_or(0.0))),
        "chroma" => Cow::Owned(json!(args
            .first()
            .and_then(|arg| expr_color(&eval_ref(arg, env)))
            .map(expr_chroma)
            .unwrap_or(0.0))),
        "hue" => Cow::Owned(json!(args
            .first()
            .and_then(|arg| expr_color(&eval_ref(arg, env)))
            .map(expr_hue)
            .unwrap_or(0.0))),
        "luminance" => Cow::Owned(json!(args
            .first()
            .and_then(|arg| expr_color(&eval_ref(arg, env)))
            .map(|value| crate::luminance(value.0, value.1, value.2))
            .unwrap_or(0.0))),
        "chromatic" => Cow::Owned(Value::Bool(
            args.first()
                .map(|arg| expr_colors(&eval_ref(arg, env)))
                .unwrap_or_default()
                .iter()
                .any(|value| expr_chroma(*value) > 25.0 && value.3 > 0.2),
        )),
        "dark" => Cow::Owned(Value::Bool(
            args.first()
                .and_then(|arg| expr_color(&eval_ref(arg, env)))
                .map(|value| crate::luminance(value.0, value.1, value.2) < 0.35)
                .unwrap_or(false),
        )),
        "red" | "r" => Cow::Owned(json!(args
            .first()
            .and_then(|arg| expr_color(&eval_ref(arg, env)))
            .map(|value| value.0)
            .unwrap_or(0.0))),
        "green" | "g" => Cow::Owned(json!(args
            .first()
            .and_then(|arg| expr_color(&eval_ref(arg, env)))
            .map(|value| value.1)
            .unwrap_or(0.0))),
        "blue" | "b" => Cow::Owned(json!(args
            .first()
            .and_then(|arg| expr_color(&eval_ref(arg, env)))
            .map(|value| value.2)
            .unwrap_or(0.0))),
        "gt" => Cow::Owned(Value::Bool(
            args.len() >= 2
                && eval_num(&args[0], env).unwrap_or(0.0) > eval_num(&args[1], env).unwrap_or(0.0),
        )),
        "gte" | "atLeast" => Cow::Owned(Value::Bool(
            args.len() >= 2
                && eval_num(&args[0], env).unwrap_or(0.0) >= eval_num(&args[1], env).unwrap_or(0.0),
        )),
        "lt" => Cow::Owned(Value::Bool(
            args.len() >= 2
                && eval_num(&args[0], env).unwrap_or(0.0) < eval_num(&args[1], env).unwrap_or(0.0),
        )),
        "lte" | "atMost" => Cow::Owned(Value::Bool(
            args.len() >= 2
                && eval_num(&args[0], env).unwrap_or(0.0) <= eval_num(&args[1], env).unwrap_or(0.0),
        )),
        "equals" | "eq" => Cow::Owned(Value::Bool(
            args.len() >= 2 && expr_equal(&eval_ref(&args[0], env), &eval_ref(&args[1], env)),
        )),
        "notEquals" | "ne" => Cow::Owned(Value::Bool(
            args.len() >= 2 && !expr_equal(&eval_ref(&args[0], env), &eval_ref(&args[1], env)),
        )),
        "contains" => {
            let matched = if args.len() >= 2 {
                let haystack = eval_ref(&args[0], env);
                let needle = eval_ref(&args[1], env);
                expr_text_cow(&haystack).contains(expr_text_cow(&needle).as_ref())
            } else {
                false
            };
            Cow::Owned(Value::Bool(matched))
        }
        "matches" => {
            let matched = if let (Some(arg0), Some(arg1)) = (args.first(), args.get(1)) {
                let text_val = eval_ref(arg0, env);
                let pat_val = eval_ref(arg1, env);
                let text = expr_text_cow(&text_val);
                let pattern = expr_text_cow(&pat_val);
                cached_regex_with(&pattern, |regex| regex.is_match(&text)).unwrap_or(false)
            } else {
                false
            };
            Cow::Owned(Value::Bool(matched))
        }
        "countMatches" | "count_matches" => {
            let count = if let (Some(arg0), Some(arg1)) = (args.first(), args.get(1)) {
                let text_val = eval_ref(arg0, env);
                let pat_val = eval_ref(arg1, env);
                let text = expr_text_cow(&text_val);
                let pattern = expr_text_cow(&pat_val);
                cached_regex_with(&pattern, |regex| regex.find_iter(&text).count()).unwrap_or(0)
            } else {
                0
            };
            Cow::Owned(json!(count))
        }
        "in" => {
            let matched = if args.len() >= 2 {
                let target = eval_ref(&args[0], env);
                let list = eval_ref(&args[1], env);
                list.as_array()
                    .map(|items| items.iter().any(|item| expr_equal(&target, item)))
                    .unwrap_or(false)
            } else {
                false
            };
            Cow::Owned(Value::Bool(matched))
        }
        "number" | "numeric" => Cow::Owned(Value::Bool(
            args.first().and_then(|arg| eval_num(arg, env)).is_some(),
        )),
        "headingLevel" => {
            let text_val = args
                .first()
                .map(|arg| eval_ref(arg, env))
                .unwrap_or(Cow::Borrowed(&Value::Null));
            Cow::Owned(json!(expr_text_cow(&text_val)
                .trim_start_matches('h')
                .parse::<u32>()
                .unwrap_or(0)))
        }
        "between" => {
            let matched = if args.len() >= 3 {
                eval_num(&args[0], env)
                    .map(|value| {
                        let min = eval_num(&args[1], env).unwrap_or(value);
                        let max = eval_num(&args[2], env).unwrap_or(value);
                        value >= min && value <= max
                    })
                    .unwrap_or(false)
            } else {
                false
            };
            Cow::Owned(Value::Bool(matched))
        }
        "length" => {
            let len = args
                .first()
                .map(|arg| match eval_ref(arg, env).as_ref() {
                    Value::Null => 0,
                    Value::String(value) => value.chars().count(),
                    Value::Array(values) => values.len(),
                    other => other.to_string().chars().count(),
                })
                .unwrap_or(0);
            Cow::Owned(Value::Number(len.into()))
        }
        "concat" => {
            let mut out = Vec::new();
            for arg in args {
                match eval_ref(arg, env).into_owned() {
                    Value::Array(items) => out.extend(items),
                    other => out.push(other),
                }
            }
            Cow::Owned(Value::Array(out))
        }
        "unique" => {
            let mut out = Vec::new();
            let mut seen = HashSet::new();
            if let Some(arg0) = args.first() {
                if let Some(items) = eval_ref(arg0, env).as_array() {
                    for item in items {
                        if seen.insert(exact_key(item)) {
                            out.push(item.clone());
                        }
                    }
                }
            }
            Cow::Owned(Value::Array(out))
        }
        "sortNumbers" => {
            let mut out = args
                .first()
                .and_then(|arg| eval_ref(arg, env).as_array().cloned())
                .unwrap_or_default();
            out.sort_by(|left, right| {
                expr_number(left)
                    .unwrap_or(0.0)
                    .partial_cmp(&expr_number(right).unwrap_or(0.0))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            Cow::Owned(Value::Array(out))
        }
        "lower" => {
            let text_val = args
                .first()
                .map(|arg| eval_ref(arg, env))
                .unwrap_or(Cow::Borrowed(&Value::Null));
            Cow::Owned(json!(expr_text_cow(&text_val).to_lowercase()))
        }
        "blocks" => {
            let text_val = args
                .first()
                .map(|arg| eval_ref(arg, env))
                .unwrap_or(Cow::Borrowed(&Value::Null));
            let text = expr_text_cow(&text_val);
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
            Cow::Owned(Value::Array(blocks))
        }
        "lastSentence" => {
            let text_val = args
                .first()
                .map(|arg| eval_ref(arg, env))
                .unwrap_or(Cow::Borrowed(&Value::Null));
            let text = expr_text_cow(&text_val);
            let last = text
                .split(['.', '!', '?'])
                .rev()
                .find(|sentence| !sentence.trim().is_empty())
                .unwrap_or("")
                .trim();
            Cow::Owned(json!(last))
        }
        "containsAny" => {
            let matched = if let (Some(arg0), Some(arg1)) = (args.first(), args.get(1)) {
                let text_val = eval_ref(arg0, env);
                let text = expr_text_cow(&text_val).to_lowercase();
                let patterns_val = eval_ref(arg1, env);
                patterns_val
                    .as_array()
                    .map(|patterns| {
                        patterns
                            .iter()
                            .any(|pattern| text.contains(&expr_text_cow(pattern).to_lowercase()))
                    })
                    .unwrap_or(false)
            } else {
                false
            };
            Cow::Owned(Value::Bool(matched))
        }
        "overshooting" => {
            static CURVE_RE: OnceLock<Regex> = OnceLock::new();
            let text_val = args
                .first()
                .map(|arg| eval_ref(arg, env))
                .unwrap_or(Cow::Borrowed(&Value::Null));
            let text = expr_text_cow(&text_val);
            let lower = text.to_lowercase();
            let mut found = lower.contains("bounce") || lower.contains("elastic");
            if !found {
                let regex =
                    CURVE_RE.get_or_init(|| Regex::new(r"cubic-bezier\(([^)]*)\)").unwrap());
                found = regex
                    .captures_iter(&text)
                    .any(|capture| is_overshooting(capture.get(0).unwrap().as_str()));
            }
            Cow::Owned(Value::Bool(found))
        }
        "byIndex" => {
            let (Some(arg0), Some(arg1)) = (args.first(), args.get(1)) else {
                return Cow::Owned(Value::Null);
            };
            let Some(raw_index) =
                eval_num(arg1, env).filter(|value| value.is_finite() && *value >= 0.0)
            else {
                return Cow::Owned(Value::Null);
            };
            let index = raw_index as usize;
            match eval_ref(arg0, env) {
                Cow::Borrowed(value) => Cow::Borrowed(
                    value
                        .as_array()
                        .and_then(|items| items.get(index))
                        .unwrap_or(&Value::Null),
                ),
                Cow::Owned(value) => Cow::Owned(
                    value
                        .as_array()
                        .and_then(|items| items.get(index))
                        .cloned()
                        .unwrap_or(Value::Null),
                ),
            }
        }
        "join" => {
            let delimiter = args
                .get(1)
                .map(|arg| expr_text_cow(&eval_ref(arg, env)).into_owned())
                .unwrap_or_default();
            let out = args
                .first()
                .and_then(|arg| {
                    eval_ref(arg, env).as_array().map(|items| {
                        items
                            .iter()
                            .map(|item| expr_text_cow(item).into_owned())
                            .collect::<Vec<_>>()
                            .join(&delimiter)
                    })
                })
                .unwrap_or_default();
            Cow::Owned(json!(out))
        }
        "nearestCard" => {
            let elements = env.page["elements"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let mut current = env.element["parentIndex"].as_u64();
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
            Cow::Owned(found)
        }
        "uniqueCount" => {
            let mut unique = HashSet::new();
            if let Some(arg0) = args.first() {
                if let Some(items) = eval_ref(arg0, env).as_array() {
                    for item in items {
                        unique.insert(exact_key(item));
                    }
                }
            }
            Cow::Owned(json!(unique.len()))
        }
        "words" => {
            let text_val = args
                .first()
                .map(|arg| eval_ref(arg, env))
                .unwrap_or(Cow::Borrowed(&Value::Null));
            Cow::Owned(json!(expr_text_cow(&text_val).split_whitespace().count()))
        }
        "min" | "max" => {
            let is_min = op == "min";
            let result = if matches!(operand, Value::Array(list) if list.len() > 1) {
                let mut acc = if is_min {
                    f64::INFINITY
                } else {
                    f64::NEG_INFINITY
                };
                for arg in args {
                    if let Some(number) = eval_num(arg, env) {
                        acc = if is_min {
                            acc.min(number)
                        } else {
                            acc.max(number)
                        };
                    }
                }
                acc
            } else if let Some(arg0) = args.first() {
                match eval_ref(arg0, env).as_ref() {
                    Value::Array(items) => {
                        let mut acc = if is_min {
                            f64::INFINITY
                        } else {
                            f64::NEG_INFINITY
                        };
                        for item in items {
                            if let Some(number) = expr_number(item) {
                                acc = if is_min {
                                    acc.min(number)
                                } else {
                                    acc.max(number)
                                };
                            }
                        }
                        acc
                    }
                    other => expr_number(other).unwrap_or(0.0),
                }
            } else if is_min {
                f64::INFINITY
            } else {
                f64::NEG_INFINITY
            };
            Cow::Owned(json!(if result.is_finite() { result } else { 0.0 }))
        }
        "nthLargest" => {
            let mut list: Vec<f64> = args
                .first()
                .and_then(|arg| {
                    eval_ref(arg, env)
                        .as_array()
                        .map(|items| items.iter().filter_map(expr_number).collect())
                })
                .unwrap_or_default();
            list.sort_by(|left, right| {
                right.partial_cmp(left).unwrap_or(std::cmp::Ordering::Equal)
            });
            let index = args
                .get(1)
                .and_then(|arg| eval_num(arg, env))
                .unwrap_or(0.0) as usize;
            Cow::Owned(json!(list.get(index).copied().unwrap_or(0.0)))
        }
        "add" => Cow::Owned(json!(args
            .iter()
            .filter_map(|arg| eval_num(arg, env))
            .sum::<f64>())),
        "sub" => Cow::Owned(json!(
            args.first()
                .and_then(|arg| eval_num(arg, env))
                .unwrap_or(0.0)
                - args
                    .get(1)
                    .and_then(|arg| eval_num(arg, env))
                    .unwrap_or(0.0)
        )),
        "mul" => Cow::Owned(json!(args
            .iter()
            .filter_map(|arg| eval_num(arg, env))
            .product::<f64>())),
        "div" => {
            let denominator = args
                .get(1)
                .and_then(|arg| eval_num(arg, env))
                .unwrap_or(1.0);
            Cow::Owned(json!(
                args.first()
                    .and_then(|arg| eval_num(arg, env))
                    .unwrap_or(0.0)
                    / denominator
            ))
        }
        "round" => Cow::Owned(json!(args
            .first()
            .and_then(|arg| eval_num(arg, env))
            .map(f64::round)
            .unwrap_or(0.0))),
        "durationMax" => {
            let text_val = args
                .first()
                .map(|arg| eval_ref(arg, env))
                .unwrap_or(Cow::Borrowed(&Value::Null));
            let max = duration_values(&expr_text_cow(&text_val))
                .into_iter()
                .fold(0.0_f64, f64::max);
            Cow::Owned(json!(max))
        }
        "hasFocusRule" => {
            let text_val = args
                .first()
                .map(|arg| eval_ref(arg, env))
                .unwrap_or(Cow::Borrowed(&Value::Null));
            Cow::Owned(Value::Bool(has_visible_focus_rule(&expr_text_cow(
                &text_val,
            ))))
        }
        _ => Cow::Owned(Value::Null),
    }
}

fn has_visible_focus_rule(text: &str) -> bool {
    static RULE_RE: OnceLock<Regex> = OnceLock::new();
    static DECL_RE: OnceLock<Regex> = OnceLock::new();
    let rule = RULE_RE
        .get_or_init(|| Regex::new(r"(?s)([^{}]*:focus(?:-visible)?[^{}]*)\{([^{}]*)\}").unwrap());
    let declaration = DECL_RE.get_or_init(|| Regex::new(r"(?i)([a-z-]+)\s*:\s*([^;]+)").unwrap());
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

pub(crate) fn is_first_paint(lifecycle: &str) -> bool {
    let lifecycle = lifecycle.to_lowercase().replace(['_', '-', ' '], "");
    lifecycle.contains("firstpaint")
        || lifecycle.contains("firstframe")
        || lifecycle.contains("domcontentloaded")
        || lifecycle.contains("staged")
}

fn build_parent_index(elements: &[Value]) -> Vec<Option<usize>> {
    elements
        .iter()
        .map(|element| {
            element["parentIndex"]
                .as_u64()
                .map(|parent| parent as usize)
        })
        .collect()
}

fn ancestor_any_parents(parents: &[Option<usize>], index: usize, matches: &HashSet<usize>) -> bool {
    let mut current = parents.get(index).copied().flatten();
    while let Some(parent) = current {
        if matches.contains(&parent) {
            return true;
        }
        current = parents.get(parent).copied().flatten();
    }
    false
}

fn descendant_ancestors_set(parents: &[Option<usize>], matches: &[usize]) -> HashSet<usize> {
    let mut ancestors = HashSet::new();
    for &candidate in matches {
        let mut current = parents.get(candidate).copied().flatten();
        while let Some(parent) = current {
            if !ancestors.insert(parent) {
                break;
            }
            current = parents.get(parent).copied().flatten();
        }
    }
    ancestors
}

pub(crate) fn rule_findings(
    rules: &RuleSet,
    oracle: &Oracle,
    baseline: &Value,
    design: Option<&DesignSpec>,
    sheet_text: &str,
    lifecycle_evidence: Option<&Value>,
) -> Vec<Value> {
    let baseline_elements = baseline["elements"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let baseline_parents = build_parent_index(baseline_elements);
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
        let elements = staged_elements.unwrap_or(baseline_elements);
        let staged_parents = staged_elements.map(build_parent_index);
        let parents = staged_parents.as_deref().unwrap_or(&baseline_parents);
        let page = staged.map(|value| &value["baseline"]).unwrap_or(baseline);
        let oracle = staged_oracle.as_ref().unwrap_or(oracle);
        if matches!(rule.scope.as_str(), "page" | "document" | "sheet") {
            if let Some(expression) = &rule.when {
                let env = EvalEnv::for_root(&Value::Null, page, sheet_text, &rule.params, parents);
                let value = eval_ref(expression, &env);
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
        let ancestor_sets: Vec<(&Constraint, HashSet<usize>)> = rule
            .ancestors
            .iter()
            .map(|constraint| {
                (
                    constraint,
                    oracle
                        .matches(&constraint.selector)
                        .iter()
                        .copied()
                        .collect(),
                )
            })
            .collect();
        let descendant_sets: Vec<(&Constraint, HashSet<usize>)> = rule
            .descendants
            .iter()
            .map(|constraint| {
                (
                    constraint,
                    descendant_ancestors_set(parents, oracle.matches(&constraint.selector)),
                )
            })
            .collect();
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
            if !exception_matches.is_empty()
                && (exception_matches.contains(&index)
                    || ancestor_any_parents(parents, index, &exception_matches))
            {
                continue;
            }
            let mut conditions = Vec::new();
            let mut condition_failure: Option<(String, String)> = None;
            for (constraint, matches) in &ancestor_sets {
                let found = ancestor_any_parents(parents, index, matches);
                conditions.push(found == constraint.any);
                if found != constraint.any && condition_failure.is_none() {
                    condition_failure =
                        Some(("ancestors".to_string(), constraint.selector.clone()));
                }
            }
            for (constraint, ancestors) in &descendant_sets {
                let found = ancestors.contains(&index);
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
                        || ancestor_any_parents(parents, **index, &region_matches)
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
                let env = EvalEnv::for_root(element, page, sheet_text, &rule.params, parents);
                if !expr_truthy(&eval_ref(expression, &env)) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn eval_expr_short_circuits_and_regex_ops() {
        let page = json!({
            "elements": [],
            "ruleParams": {}
        });
        let element = json!({
            "tag": "div",
            "text": "Error: invalid token, retry"
        });

        let cond_if = json!({
            "if": [
                { "equals": [{ "field": "tag" }, "div"] },
                true,
                { "matches": [{ "field": "text" }, "["] }
            ]
        });
        assert_eq!(eval_expr(&cond_if, &element, &page, ""), json!(true));

        let cond_all = json!({
            "all": [
                { "equals": [{ "field": "tag" }, "span"] },
                { "matches": [{ "field": "text" }, "["] }
            ]
        });
        assert_eq!(eval_expr(&cond_all, &element, &page, ""), json!(false));

        let cond_any = json!({
            "any": [
                { "matches": [{ "field": "text" }, "invalid"] },
                { "matches": [{ "field": "text" }, "["] }
            ]
        });
        assert_eq!(eval_expr(&cond_any, &element, &page, ""), json!(true));

        let count_expr = json!({
            "countMatches": [{ "field": "text" }, "invalid|retry"]
        });
        assert_eq!(eval_expr(&count_expr, &element, &page, ""), json!(2));
    }

    #[test]
    fn eval_expr_unique_and_group_ops() {
        let page = json!({
            "elements": [
                { "group": "a", "tag": "div", "value": 10 },
                { "group": "a", "tag": "span", "value": 10 },
                { "group": "b", "tag": "div", "value": 20 }
            ],
            "ruleParams": {}
        });
        let element = &page["elements"][0];

        let uniq = json!({
            "uniqueCount": [{ "map": [{ "field": "page.elements" }, { "field": "group" }] }]
        });
        assert_eq!(eval_expr(&uniq, element, &page, ""), json!(2));

        let grp_any = json!({
            "groupAny": [
                { "field": "page.elements" },
                { "field": "group" },
                {
                    "gt": [
                        { "uniqueCount": [{ "map": [{ "field": "values" }, { "field": "tag" }] }] },
                        1
                    ]
                }
            ]
        });
        assert_eq!(eval_expr(&grp_any, element, &page, ""), json!(true));

        let grp_all = json!({
            "groupAll": [
                { "field": "page.elements" },
                { "field": "group" },
                {
                    "equals": [
                        { "uniqueCount": [{ "map": [{ "field": "values" }, { "field": "value" }] }] },
                        1
                    ]
                }
            ]
        });
        assert_eq!(eval_expr(&grp_all, element, &page, ""), json!(true));
    }

    #[test]
    fn eval_expr_pair_any_and_ancestor_scale_on_large_dom() {
        let mut elements = Vec::with_capacity(1500);
        elements.push(json!({
            "index": 0,
            "parentIndex": null,
            "tag": "main",
            "text": "root",
            "visible": true
        }));
        for i in 1..1500 {
            let parent = if i <= 2 { 0 } else { (i - 1) / 2 };
            elements.push(json!({
                "index": i,
                "parentIndex": parent,
                "tag": "div",
                "text": format!("item-{i}"),
                "visible": i > 1495
            }));
        }
        let page = json!({
            "elements": elements,
            "ruleParams": {}
        });
        let root = &page["elements"][0];

        let eq_join_miss = json!({
            "pairAny": [
                { "field": "page.elements" },
                {
                    "all": [
                        { "equals": [{ "field": "left.parentIndex" }, { "field": "right.parentIndex" }] },
                        { "equals": [{ "field": "left.text" }, { "field": "right.text" }] }
                    ]
                }
            ]
        });

        let start = Instant::now();
        assert_eq!(eval_expr(&eq_join_miss, root, &page, ""), json!(false));

        let filtered_ancestor = json!({
            "pairAny": [
                { "field": "page.elements" },
                {
                    "all": [
                        { "field": "left.visible" },
                        { "field": "right.visible" },
                        { "not": [{ "ancestor": [{ "field": "left.index" }, { "field": "right.index" }] }] }
                    ]
                }
            ]
        });
        assert_eq!(eval_expr(&filtered_ancestor, root, &page, ""), json!(true));
        assert!(start.elapsed().as_millis() < 1000);
    }

    #[test]
    fn eval_expr_interval_sweep_and_ancestor_join_scale_on_flat_siblings() {
        let mut elements = Vec::with_capacity(2001);
        elements.push(json!({
            "index": 0,
            "parentIndex": null,
            "visible": true,
            "rect": { "x": 0.0, "y": 0.0, "w": 1200.0, "h": 40000.0 }
        }));
        for i in 1..2001 {
            let y = ((i - 1) as f64) * 20.0;
            elements.push(json!({
                "index": i,
                "parentIndex": 0,
                "visible": true,
                "text": format!("row-{i}"),
                "rect": { "x": 0.0, "y": y, "w": 1000.0, "h": 18.0 }
            }));
        }
        let mut page = json!({
            "elements": elements,
            "ruleParams": {}
        });
        let parents = build_parent_index(page["elements"].as_array().unwrap());

        let overlap_expr = json!({
            "pairAny": [
                { "field": "page.elements" },
                {
                    "all": [
                        { "equals": [{ "field": "left.visible" }, true] },
                        { "equals": [{ "field": "right.visible" }, true] },
                        { "equals": [{ "field": "left.parentIndex" }, { "field": "right.parentIndex" }] },
                        { "notEquals": [{ "field": "left.text" }, { "field": "right.text" }] },
                        {
                            "gt": [
                                {
                                    "sub": [
                                        {
                                            "min": [
                                                { "add": [{ "field": "left.rect.x" }, { "field": "left.rect.w" }] },
                                                { "add": [{ "field": "right.rect.x" }, { "field": "right.rect.w" }] }
                                            ]
                                        },
                                        { "max": [{ "field": "left.rect.x" }, { "field": "right.rect.x" }] }
                                    ]
                                },
                                4
                            ]
                        },
                        {
                            "gt": [
                                {
                                    "sub": [
                                        {
                                            "min": [
                                                { "add": [{ "field": "left.rect.y" }, { "field": "left.rect.h" }] },
                                                { "add": [{ "field": "right.rect.y" }, { "field": "right.rect.h" }] }
                                            ]
                                        },
                                        { "max": [{ "field": "left.rect.y" }, { "field": "right.rect.y" }] }
                                    ]
                                },
                                4
                            ]
                        }
                    ]
                }
            ]
        });

        let start = Instant::now();
        let root_env = EvalEnv::for_root(
            &page["elements"][0],
            &page,
            "",
            &page["ruleParams"],
            &parents,
        );
        assert_eq!(eval_ref(&overlap_expr, &root_env).as_ref(), &json!(false));

        let anc_expr = json!({
            "pairAny": [
                { "field": "page.elements" },
                {
                    "all": [
                        { "ancestor": [{ "field": "left.index" }, { "field": "right.index" }] },
                        { "equals": [{ "field": "right.text" }, "row-1999"] }
                    ]
                }
            ]
        });
        assert_eq!(eval_ref(&anc_expr, &root_env).as_ref(), &json!(true));
        assert!(start.elapsed().as_millis() < 1000);

        page["elements"][1500]["rect"]["y"] = json!(199.0 * 20.0 + 2.0);
        let parents2 = build_parent_index(page["elements"].as_array().unwrap());
        let env2 = EvalEnv::for_root(
            &page["elements"][0],
            &page,
            "",
            &page["ruleParams"],
            &parents2,
        );
        assert_eq!(eval_ref(&overlap_expr, &env2).as_ref(), &json!(true));
    }

    #[test]
    fn rule_findings_handles_ancestors_descendants_and_rule_params() {
        let pack = json!({
            "version": 2,
            "rules": [
                {
                    "id": "custom-param-check",
                    "severity": "advisory",
                    "layer": "layout",
                    "selector": "button",
                    "ancestors": ["form"],
                    "descendants": ["span"],
                    "params": { "minPx": 16 },
                    "when": { "lt": [{ "field": "fontSize" }, { "field": "page.ruleParams.minPx" }] },
                    "message": "small button inside form"
                }
            ]
        });
        let (ruleset, notes) = parse_rules_v2(&pack, None).expect("valid rules");
        assert!(notes.is_empty());

        let oracle = oracle_from_value(&json!({
            "selectors": {
                "button": { "matches": [1, 4], "values": {} },
                "form": { "matches": [0], "values": {} },
                "span": { "matches": [2], "values": {} }
            }
        }))
        .expect("valid oracle");

        let baseline = json!({
            "elements": [
                { "index": 0, "parentIndex": null, "selector": "form", "tag": "form", "fontSize": "16px", "visible": true, "ignoreRules": [] },
                { "index": 1, "parentIndex": 0, "selector": "button#small", "tag": "button", "fontSize": "12px", "visible": true, "ignoreRules": [] },
                { "index": 2, "parentIndex": 1, "selector": "button#small > span", "tag": "span", "fontSize": "12px", "visible": true, "ignoreRules": [] },
                { "index": 3, "parentIndex": null, "selector": "div", "tag": "div", "fontSize": "16px", "visible": true, "ignoreRules": [] },
                { "index": 4, "parentIndex": 3, "selector": "button#outside", "tag": "button", "fontSize": "12px", "visible": true, "ignoreRules": [] }
            ]
        });

        let findings = rule_findings(&ruleset, &oracle, &baseline, None, "", None);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0]["selector"], json!("button#small"));
        assert_eq!(findings[0]["check"], json!("custom-param-check"));
    }

    #[test]
    fn sheet_and_math_ops_and_source_rules_work_declaratively() {
        let page = json!({
            "elements": [
                { "index": 0, "parentIndex": null, "visible": true, "cardLike": false, "rect": { "x": 0, "y": 0, "w": 800, "h": 400 } },
                { "index": 1, "parentIndex": 0, "visible": true, "cardLike": true, "rect": { "x": 0, "y": 0, "w": 240, "h": 160 } },
                { "index": 2, "parentIndex": 0, "visible": true, "cardLike": true, "rect": { "x": 260, "y": 0, "w": 240, "h": 160 } },
                { "index": 3, "parentIndex": 0, "visible": true, "cardLike": true, "rect": { "x": 520, "y": 0, "w": 240, "h": 160 } }
            ],
            "ruleParams": {}
        });
        let parents = build_parent_index(page["elements"].as_array().unwrap());
        let env = EvalEnv::for_root(
            &Value::Null,
            &page,
            "a:focus { outline: none; }",
            &page["ruleParams"],
            &parents,
        );
        assert_eq!(
            eval_ref(&json!({ "hasFocusRule": { "field": "sheet.text" } }), &env).as_ref(),
            &json!(false)
        );
        let env_ok = EvalEnv::for_root(
            &Value::Null,
            &page,
            "button:focus-visible { outline: 2px solid #111; }",
            &page["ruleParams"],
            &parents,
        );
        assert_eq!(
            eval_ref(
                &json!({ "hasFocusRule": { "field": "sheet.text" } }),
                &env_ok
            )
            .as_ref(),
            &json!(true)
        );
        assert_eq!(
            eval_ref(&json!({ "durationMax": "150ms, 0.35s" }), &env).as_ref(),
            &json!(0.35)
        );
        assert_eq!(
            eval_ref(&json!({ "round": { "div": [243, 8] } }), &env).as_ref(),
            &json!(30.0)
        );

        let (defaults, _) = load_default_rules(None).expect("default rules load");
        assert!(defaults
            .source_rules()
            .iter()
            .any(|rule| rule.id == "raw-color"));

        let grid_expr = json!({
            "groupAny": [
                { "field": "page.elements" },
                { "field": "parentIndex" },
                { "gte": [{ "length": { "field": "values" } }, 3] }
            ]
        });
        let single_card = json!({
            "elements": [
                { "index": 0, "parentIndex": null },
                { "index": 1, "parentIndex": 0 }
            ],
            "ruleParams": {}
        });
        let single_parents = build_parent_index(single_card["elements"].as_array().unwrap());
        let single_env = EvalEnv::for_root(
            &Value::Null,
            &single_card,
            "",
            &single_card["ruleParams"],
            &single_parents,
        );
        assert_eq!(eval_ref(&grid_expr, &single_env).as_ref(), &json!(false));
        assert_eq!(
            eval_ref(&json!({ "length": { "field": "missing" } }), &single_env).as_ref(),
            &json!(0)
        );
        assert_eq!(
            eval_ref(
                &json!({ "byIndex": [{ "field": "page.elements" }, -1] }),
                &single_env
            )
            .as_ref(),
            &Value::Null
        );
        assert_eq!(
            eval_ref(&json!({ "ancestor": [0, 9999] }), &single_env).as_ref(),
            &json!(false)
        );
        assert_eq!(
            eval_ref(&json!({ "ancestor": [-1, 1] }), &single_env).as_ref(),
            &json!(false)
        );

        let tmp = std::env::temp_dir().join(format!("ui-probe-src-test-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let base_path = tmp.join("base.json");
        std::fs::write(
            &base_path,
            json!({
                "version": 2,
                "sourceRules": [
                    { "id": "todo-marker", "severity": "P3", "layer": "source", "message": "todo marker", "pattern": "TODO" }
                ]
            })
            .to_string(),
        )
        .unwrap();
        let override_path = tmp.join("rules.json");
        std::fs::write(
            &override_path,
            json!({
                "version": 2,
                "sourceRules": [
                    { "id": "transition-all", "enabled": false },
                    { "id": "todo-marker", "severity": "P2" }
                ]
            })
            .to_string(),
        )
        .unwrap();
        let (merged, _) = load_rules_with_default(&[override_path.clone(), base_path], None)
            .expect("merged source rules");
        assert!(!merged
            .source_rules()
            .iter()
            .any(|rule| rule.id == "transition-all"));
        let todo = merged
            .source_rules()
            .iter()
            .find(|rule| rule.id == "todo-marker")
            .expect("merged todo-marker");
        assert_eq!(todo.severity, "P2");
        assert!(!merged
            .disabled_checks()
            .contains(&"transition-all".to_string()));

        let invalid_path = tmp.join("invalid-regex.json");
        std::fs::write(
            &invalid_path,
            json!({
                "version": 2,
                "sourceRules": [
                    { "id": "broken-re", "message": "bad", "pattern": "[unclosed" }
                ]
            })
            .to_string(),
        )
        .unwrap();
        assert!(load_rules_with_default(&[invalid_path], None).is_err());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
