use crate::{
    duration_values, parse_css_color, sort_value_entries, split_top_level, visible_elements,
};
use anyhow::{Context, Result};
use regex::Regex;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::PathBuf;

pub(crate) struct DesignSpec {
    pub(crate) colors: Option<Vec<(i64, i64, i64)>>,
    pub(crate) spacing: Option<Vec<f64>>,
    pub(crate) radii: Option<Vec<String>>,
    pub(crate) fonts: Option<Vec<String>>,
    pub(crate) typography: Option<Vec<TypographyStep>>,
    pub(crate) surfaces: Option<Value>,
    pub(crate) motion: Option<Value>,
    pub(crate) params: Value,
}

pub(crate) struct TypographyStep {
    pub(crate) size: f64,
    pub(crate) weight: u64,
}

pub(crate) struct DesignToken {
    path: Vec<String>,
    value: Value,
    kind: Option<String>,
    extensions: Value,
}

pub(crate) fn has_token(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    if object.contains_key("$value") {
        return true;
    }
    object
        .iter()
        .any(|(key, child)| !key.starts_with('$') && has_token(child))
}

pub(crate) fn is_dtcg(data: &Value) -> bool {
    [
        "color",
        "font",
        "typography",
        "spacing",
        "radius",
        "surface",
    ]
    .iter()
    .any(|group| data.get(*group).map(has_token).unwrap_or(false))
}

pub(crate) fn collect_tokens(
    value: &Value,
    path: &mut Vec<String>,
    inherited: Option<&str>,
    inherited_extensions: &Value,
    out: &mut Vec<DesignToken>,
) {
    let Some(object) = value.as_object() else {
        return;
    };
    let kind = object
        .get("$type")
        .and_then(Value::as_str)
        .or(inherited)
        .map(|value| value.to_string());
    let extensions = object
        .get("$extensions")
        .unwrap_or(inherited_extensions)
        .clone();
    if let Some(raw) = object.get("$value") {
        out.push(DesignToken {
            path: path.clone(),
            value: raw.clone(),
            kind,
            extensions,
        });
        return;
    }
    for (key, child) in object {
        if key.starts_with('$') {
            continue;
        }
        path.push(key.clone());
        collect_tokens(child, path, kind.as_deref(), &extensions, out);
        path.pop();
    }
}

pub(crate) fn group_tokens(data: &Value, group: &str) -> Vec<DesignToken> {
    let Some(value) = data.get(group) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut path = vec![group.to_string()];
    let no_extensions = Value::Null;
    collect_tokens(value, &mut path, None, &no_extensions, &mut out);
    out
}

pub(crate) fn clean_family(raw: &str) -> String {
    raw.trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim()
        .to_lowercase()
}

pub(crate) fn generic_family(family: &str) -> bool {
    matches!(
        family,
        "serif"
            | "sans-serif"
            | "monospace"
            | "cursive"
            | "fantasy"
            | "system-ui"
            | "ui-serif"
            | "ui-sans-serif"
            | "ui-monospace"
            | "ui-rounded"
            | "emoji"
            | "math"
            | "fangsong"
            | "-apple-system"
            | "blinkmacsystemfont"
    )
}

pub(crate) fn dimension_px(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(raw) => raw
            .trim()
            .strip_suffix("px")
            .and_then(|number| number.trim().parse::<f64>().ok()),
        _ => None,
    }
}

pub(crate) fn dimension_string(value: &Value) -> Option<String> {
    match value {
        Value::Number(number) => Some(format!("{}px", number.as_f64()?)),
        Value::String(raw) => {
            let value = raw.trim().to_lowercase();
            if value.is_empty() {
                None
            } else if value
                .chars()
                .all(|character| character.is_ascii_digit() || character == '.')
            {
                Some(format!("{value}px"))
            } else {
                Some(value)
            }
        }
        _ => None,
    }
}

pub(crate) fn merge_json(base: &mut Value, overlay: &Value) {
    match (base, overlay) {
        (Value::Object(base_map), Value::Object(overlay_map)) => {
            for (key, value) in overlay_map {
                match base_map.get_mut(key) {
                    Some(existing) => merge_json(existing, value),
                    None => {
                        base_map.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        (base, overlay) => *base = overlay.clone(),
    }
}

pub(crate) fn load_design_sources(
    sources: &[(PathBuf, &'static str)],
) -> Result<(DesignSpec, Vec<Value>)> {
    let mut merged = Value::Object(serde_json::Map::new());
    for (path, _) in sources.iter().rev() {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("read design file {}", path.display()))?;
        let data: Value = serde_json::from_str(&text)
            .with_context(|| format!("parse design file {}", path.display()))?;
        if !data.is_object() {
            anyhow::bail!("design file {} is not a JSON object", path.display());
        }
        merge_json(&mut merged, &data);
    }
    Ok(parse_design(&merged))
}

pub(crate) fn parse_design(data: &Value) -> (DesignSpec, Vec<Value>) {
    let mut notes = Vec::new();
    let spec = if is_dtcg(data) {
        let mut spec = load_dtcg_design(data, &mut notes);
        let flat = load_flat_design(data, &mut notes);
        if spec.colors.is_none() {
            spec.colors = flat.colors;
        }
        if spec.spacing.is_none() {
            spec.spacing = flat.spacing;
        }
        if spec.radii.is_none() {
            spec.radii = flat.radii;
        }
        if spec.fonts.is_none() {
            spec.fonts = flat.fonts;
        }
        if spec.typography.is_none() {
            spec.typography = flat.typography;
        }
        if spec.surfaces.is_none() {
            spec.surfaces = flat.surfaces;
        }
        if spec.motion.is_none() {
            spec.motion = flat.motion;
        }
        spec
    } else {
        load_flat_design(data, &mut notes)
    };
    (spec, notes)
}

pub(crate) fn load_dtcg_design(data: &Value, notes: &mut Vec<Value>) -> DesignSpec {
    let color_tokens = group_tokens(data, "color");
    let mut colors = Vec::new();
    for token in color_tokens {
        if token.kind.as_deref() == Some("gradient") {
            continue;
        }
        match token.value.as_str().and_then(parse_color_rgb) {
            Some(color) => colors.push(color),
            None => notes.push(json!({
                "check": "design",
                "reason": "unsupported colour in design file",
                "path": token.path.join("."),
                "value": token.value
            })),
        }
    }

    let font_tokens = group_tokens(data, "font");
    let mut fonts = Vec::new();
    for token in font_tokens {
        match &token.value {
            Value::String(raw) => {
                for family in split_top_level(raw) {
                    fonts.push(clean_family(&family));
                }
            }
            Value::Array(list) => {
                for value in list {
                    if let Some(raw) = value.as_str() {
                        fonts.push(clean_family(raw));
                    }
                }
            }
            _ => {}
        }
    }
    fonts.retain(|family| !family.is_empty());
    fonts.sort();
    fonts.dedup();

    let spacing_tokens = group_tokens(data, "spacing");
    let mut spacing = Vec::new();
    for token in spacing_tokens {
        match dimension_px(&token.value) {
            Some(value) => spacing.push(value),
            None => notes.push(json!({
                "check": "design",
                "reason": "unsupported spacing token",
                "path": token.path.join("."),
                "value": token.value
            })),
        }
    }

    let radius_tokens = group_tokens(data, "radius");
    let mut radii = Vec::new();
    for token in radius_tokens {
        match dimension_string(&token.value) {
            Some(value) => radii.push(value),
            None => notes.push(json!({
                "check": "design",
                "reason": "unsupported radius token",
                "path": token.path.join("."),
                "value": token.value
            })),
        }
    }

    let typography_tokens = group_tokens(data, "typography");
    let mut typography = Vec::new();
    for token in typography_tokens {
        let Some(value) = token.value.as_object() else {
            continue;
        };
        let Some(size) = value.get("fontSize").and_then(dimension_px) else {
            continue;
        };
        let weight = value
            .get("fontWeight")
            .and_then(|value| {
                value
                    .as_u64()
                    .or_else(|| value.as_str().and_then(|raw| raw.parse::<u64>().ok()))
            })
            .unwrap_or(400);
        typography.push(TypographyStep { size, weight });
    }

    let mut surfaces = serde_json::Map::new();
    for token in group_tokens(data, "surface") {
        let theme = token
            .extensions
            .get("theme")
            .and_then(Value::as_str)
            .map(|value| value.to_string())
            .or_else(|| {
                token
                    .path
                    .iter()
                    .find(|part| matches!(part.as_str(), "light" | "dark"))
                    .cloned()
            });
        let Some(theme) = theme else {
            continue;
        };
        let Some(leaf) = token.path.last() else {
            continue;
        };
        let key = match leaf.as_str() {
            "caret" => "caret",
            "selection" => "selection",
            "focusRing" | "focus-ring" | "focus_ring" => "focusRing",
            _ => continue,
        };
        let entry = surfaces.entry(theme).or_insert_with(|| json!({}));
        let value = if key == "focusRing" {
            match &token.value {
                Value::Array(_) => token.value.clone(),
                Value::String(raw) => json!([raw]),
                _ => continue,
            }
        } else {
            match &token.value {
                Value::String(raw) => json!(raw),
                _ => continue,
            }
        };
        entry[key] = value;
    }

    DesignSpec {
        colors: if colors.is_empty() {
            None
        } else {
            Some(colors)
        },
        spacing: if spacing.is_empty() {
            None
        } else {
            Some(spacing)
        },
        radii: if radii.is_empty() { None } else { Some(radii) },
        fonts: if fonts.is_empty() { None } else { Some(fonts) },
        typography: if typography.is_empty() {
            None
        } else {
            Some(typography)
        },
        surfaces: if surfaces.is_empty() {
            None
        } else {
            Some(Value::Object(surfaces))
        },
        motion: data
            .get("motion")
            .filter(|value| value.is_object())
            .cloned(),
        params: data.clone(),
    }
}

pub(crate) fn load_flat_design(data: &Value, notes: &mut Vec<Value>) -> DesignSpec {
    let mut raw_colors: Vec<String> = Vec::new();
    if let Some(list) = data["colors"].as_array() {
        for value in list {
            if let Some(raw) = value.as_str() {
                raw_colors.push(raw.to_string());
            }
        }
    } else if let Some(object) = data["colors"].as_object() {
        for value in object.values() {
            if let Some(raw) = value.as_str() {
                raw_colors.push(raw.to_string());
            } else if let Some(raw) = value.get("$value").and_then(Value::as_str) {
                raw_colors.push(raw.to_string());
            }
        }
    }
    let colors = if raw_colors.is_empty() {
        None
    } else {
        Some(
            raw_colors
                .into_iter()
                .filter_map(|raw| match parse_color_rgb(&raw) {
                    Some(color) => Some(color),
                    None => {
                        notes.push(json!({
                            "check": "design",
                            "reason": "unsupported colour in design file",
                            "value": raw
                        }));
                        None
                    }
                })
                .collect::<Vec<_>>(),
        )
    };
    let spacing = data["spacingPx"]
        .as_array()
        .map(|list| list.iter().filter_map(Value::as_f64).collect::<Vec<_>>());
    let radii = data["radiiPx"].as_array().map(|list| {
        list.iter()
            .filter_map(|value| value.as_f64())
            .map(|value| format!("{value}px"))
            .collect::<Vec<_>>()
    });
    let fonts = data["fontFamilies"].as_array().map(|list| {
        let mut fonts: Vec<String> = list
            .iter()
            .filter_map(Value::as_str)
            .map(clean_family)
            .filter(|value| !value.is_empty())
            .collect();
        fonts.sort();
        fonts.dedup();
        fonts
    });
    let typography = data["typography"].as_array().map(|list| {
        list.iter()
            .filter_map(|value| {
                Some(TypographyStep {
                    size: value.get("fontSize").and_then(dimension_px)?,
                    weight: value
                        .get("fontWeight")
                        .and_then(Value::as_u64)
                        .unwrap_or(400),
                })
            })
            .collect::<Vec<_>>()
    });

    DesignSpec {
        colors,
        spacing,
        radii,
        fonts,
        typography,
        surfaces: data
            .get("surfaces")
            .filter(|value| value.is_object())
            .cloned(),
        motion: data
            .get("motion")
            .filter(|value| value.is_object())
            .cloned(),
        params: data.clone(),
    }
}

pub(crate) fn parse_dimension(raw: &str) -> Option<(f64, String)> {
    let value = raw.trim().to_lowercase();
    if value == "0" {
        return Some((0.0, "px".to_string()));
    }
    let index = value.find(|character: char| {
        !(character.is_ascii_digit() || character == '.' || character == '-')
    })?;
    let number = value[..index].parse::<f64>().ok()?;
    Some((number, value[index..].trim().to_string()))
}

pub(crate) fn radius_allowed(actual: &str, allowed: &[String]) -> bool {
    let Some((actual_value, actual_unit)) = parse_dimension(actual) else {
        return false;
    };
    allowed.iter().any(|entry| {
        let Some((allowed_value, allowed_unit)) = parse_dimension(entry) else {
            return false;
        };
        allowed_unit == actual_unit
            && (actual_value - allowed_value).abs() <= if actual_unit == "px" { 0.5 } else { 0.01 }
    })
}

pub(crate) fn font_families(stack: &str) -> Vec<String> {
    split_top_level(stack)
        .iter()
        .map(|value| clean_family(value))
        .filter(|value| !value.is_empty())
        .collect()
}

pub(crate) fn sheet_contains_color(text: &str, expected: &str) -> bool {
    let Some(target) = parse_color_rgb(expected) else {
        return text.to_lowercase().contains(&expected.to_lowercase());
    };
    let pattern = Regex::new(r"(?i)#[0-9a-f]{3,8}\b|rgba?\([^)]*\)").unwrap();
    let found = pattern
        .find_iter(text)
        .filter_map(|value| parse_color_rgb(value.as_str()))
        .any(|color| color == target);
    found
}

pub(crate) fn opaque_sampled(raw: &str) -> bool {
    parse_css_color(raw)
        .map(|(_, _, _, alpha)| alpha > 0.0)
        .unwrap_or(false)
}

pub(crate) fn design_checks(
    baseline: &Value,
    spec: &DesignSpec,
    sheet_text: &str,
) -> (Vec<Value>, Vec<Value>) {
    let notes: Vec<Value> = Vec::new();
    let mut findings = Vec::new();
    let mut seen = HashSet::new();

    for element in visible_elements(baseline) {
        let selector = element["selector"].as_str().unwrap_or("?").to_string();

        if let Some(allowed) = &spec.colors {
            let mut colors: Vec<(String, String, (i64, i64, i64))> = Vec::new();
            if let Some(raw) = element["color"].as_str() {
                if opaque_sampled(raw) {
                    if let Some(rgb) = parse_color_rgb(raw) {
                        colors.push(("color".to_string(), raw.to_string(), rgb));
                    }
                }
            }
            if element["backgroundImage"].as_str().unwrap_or("none") == "none" {
                if let Some(raw) = element["backgroundColor"].as_str() {
                    if opaque_sampled(raw) {
                        if let Some(rgb) = parse_color_rgb(raw) {
                            colors.push(("background-color".to_string(), raw.to_string(), rgb));
                        }
                    }
                }
            }
            for (property, value, rgb) in colors {
                if allowed
                    .iter()
                    .any(|entry| entry.0 == rgb.0 && entry.1 == rgb.1 && entry.2 == rgb.2)
                {
                    continue;
                }
                let key = format!("color|{selector}|{property}|{value}");
                if !seen.insert(key) {
                    continue;
                }
                findings.push(json!({
                    "check": "design-color",
                    "severity": "P2",
                    "selector": selector,
                    "property": property.strip_prefix('@').unwrap_or(&property),
                    "message": "computed colour is outside the design palette",
                    "evidence": { "value": value, "property": property }
                }));
            }
        }

        if let Some(allowed) = &spec.spacing {
            let values = [
                element["padding"].as_array().cloned().unwrap_or_default(),
                element["margin"].as_array().cloned().unwrap_or_default(),
                element["gap"].as_array().cloned().unwrap_or_default(),
            ];
            for value in values.into_iter().flatten() {
                let Some(number) = value.as_f64() else {
                    continue;
                };
                if number <= 0.0 || allowed.iter().any(|entry| (entry - number).abs() < 0.5) {
                    continue;
                }
                let key = format!("spacing|{selector}|{number}");
                if !seen.insert(key) {
                    continue;
                }
                findings.push(json!({
                    "check": "design-spacing",
                    "severity": "P3",
                    "selector": selector,
                    "message": "spacing value is outside the design scale",
                    "evidence": { "value": number }
                }));
            }
        }

        if let Some(allowed) = &spec.radii {
            for value in element["radius"].as_array().cloned().unwrap_or_default() {
                let Some(raw) = value.as_str() else {
                    continue;
                };
                let Some((number, _)) = parse_dimension(raw) else {
                    continue;
                };
                if number <= 0.0 || radius_allowed(raw, allowed) {
                    continue;
                }
                let key = format!("radius|{selector}|{raw}");
                if !seen.insert(key) {
                    continue;
                }
                findings.push(json!({
                    "check": "design-radius",
                    "severity": "P3",
                    "selector": selector,
                    "message": "radius is outside the design scale",
                    "evidence": { "value": raw }
                }));
            }
        }

        if let Some(allowed) = &spec.fonts {
            let stack = element["fontFamily"].as_str().unwrap_or("");
            for family in font_families(stack) {
                if generic_family(&family) || allowed.iter().any(|entry| entry == &family) {
                    continue;
                }
                let key = format!("font|{selector}|{family}");
                if !seen.insert(key) {
                    continue;
                }
                findings.push(json!({
                    "check": "design-font",
                    "severity": "P2",
                    "selector": selector,
                    "message": "font family is outside the design system",
                    "evidence": { "family": family }
                }));
            }
        }
    }

    if let Some(steps) = &spec.typography {
        let sizes: Vec<f64> = steps.iter().map(|step| step.size).collect();
        for element in visible_elements(baseline) {
            if !element["hasText"].as_bool().unwrap_or(false) {
                continue;
            }
            let size = element["fontSize"].as_f64().unwrap_or(0.0);
            let weight = element["fontWeight"].as_u64().unwrap_or(400);
            if size <= 0.0
                || steps
                    .iter()
                    .any(|step| (step.size - size).abs() <= 0.5 && step.weight == weight)
            {
                continue;
            }
            let selector = element["selector"].as_str().unwrap_or("?").to_string();
            let key = format!("typography|{selector}|{size}");
            if !seen.insert(key) {
                continue;
            }
            findings.push(json!({
                "check": "design-typography",
                "severity": "P3",
                "selector": selector,
                "property": "font-size",
                "message": "font size is outside the design typography steps",
                "evidence": { "fontSize": size, "steps": sizes }
            }));
        }
    }

    if let Some(surfaces) = &spec.surfaces {
        let scheme = baseline["scheme"].as_str().unwrap_or("light");
        if let Some(surface) = surfaces.get(scheme) {
            if let Some(expected) = surface.get("caret").and_then(Value::as_str) {
                if let Some(expected_rgb) = parse_color_rgb(expected) {
                    for element in visible_elements(baseline) {
                        let tag = element["tag"].as_str().unwrap_or("");
                        if tag != "input" && tag != "textarea" {
                            continue;
                        }
                        if element["caretColor"].as_str().and_then(parse_color_rgb)
                            != Some(expected_rgb)
                        {
                            findings.push(json!({
                                "check": "surface-caret",
                                "severity": "error",
                                "selector": element["selector"],
                                "message": "caret colour does not match the surface recipe",
                                "evidence": { "expected": expected, "actual": element["caretColor"], "theme": scheme }
                            }));
                        }
                    }
                }
            }
            if let Some(selection) = surface.get("selection").and_then(Value::as_str) {
                if !sheet_contains_color(sheet_text, selection) {
                    findings.push(json!({
                        "check": "surface-selection",
                        "severity": "error",
                        "selector": "::selection",
                        "message": "selection colour does not match the surface recipe",
                        "evidence": { "expected": selection, "theme": scheme }
                    }));
                }
            }
            if let Some(ring) = surface.get("focusRing").and_then(Value::as_array) {
                if let Some(color) = ring.first().and_then(Value::as_str) {
                    if !sheet_text.to_lowercase().contains(&color.to_lowercase()) {
                        findings.push(json!({
                            "check": "surface-focus",
                            "severity": "error",
                            "selector": ":focus",
                            "message": "focus ring does not match the surface recipe",
                            "evidence": { "expected": color, "theme": scheme }
                        }));
                    }
                }
            }
        }
    }

    if let Some(motion) = &spec.motion {
        let mut ranges: Vec<(f64, f64)> = Vec::new();
        for key in ["arriveMs", "leaveMs"] {
            if let Some(pair) = motion.get(key).and_then(Value::as_array) {
                if pair.len() == 2 {
                    if let (Some(min), Some(max)) = (pair[0].as_f64(), pair[1].as_f64()) {
                        ranges.push((min / 1000.0, max / 1000.0));
                    }
                }
            }
        }
        if !ranges.is_empty() {
            for element in visible_elements(baseline) {
                for raw in [
                    element["transitionDuration"].as_str().unwrap_or(""),
                    element["animationDuration"].as_str().unwrap_or(""),
                ] {
                    for value in split_top_level(raw) {
                        for seconds in duration_values(&value) {
                            if seconds <= 0.0 {
                                continue;
                            }
                            if ranges
                                .iter()
                                .any(|(min, max)| seconds >= *min && seconds <= *max)
                            {
                                continue;
                            }
                            findings.push(json!({
                                "check": "motion-duration",
                                "severity": "advisory",
                                "selector": element["selector"],
                                "message": "duration is outside the declared motion ranges",
                                "evidence": { "value": value, "seconds": seconds }
                            }));
                        }
                    }
                }
            }
        }
    }

    (findings, notes)
}

pub(crate) fn parse_color_rgb(css: &str) -> Option<(i64, i64, i64)> {
    let css = css.trim().to_lowercase();
    if let Some(hex) = css.strip_prefix('#') {
        let digits = hex.len();
        if !hex.chars().all(|character| character.is_ascii_hexdigit()) {
            return None;
        }
        let pairs: Vec<String> = match digits {
            3 | 4 => hex
                .chars()
                .take(3)
                .map(|character| format!("{character}{character}"))
                .collect(),
            6 | 8 => (0..3)
                .map(|index| hex[index * 2..index * 2 + 2].to_string())
                .collect(),
            _ => return None,
        };
        let channel = |value: &str| i64::from_str_radix(value, 16).ok();
        return Some((
            channel(&pairs[0])?,
            channel(&pairs[1])?,
            channel(&pairs[2])?,
        ));
    }
    parse_css_color(&css).map(|(r, g, b, _)| (r.round() as i64, g.round() as i64, b.round() as i64))
}

pub(crate) fn primary_font(stack: &str) -> String {
    font_families(stack).into_iter().next().unwrap_or_default()
}

pub(crate) fn confidence(count: u64, total: u64) -> f64 {
    if total == 0 {
        return 0.0;
    }
    ((count as f64 / total as f64) * 1000.0).round() / 1000.0
}

pub(crate) fn infer_design(baseline: &Value) -> (Value, Vec<Value>) {
    let mut palette: std::collections::HashMap<(i64, i64, i64), u64> =
        std::collections::HashMap::new();
    let mut spacing: std::collections::HashMap<i64, u64> = std::collections::HashMap::new();
    let mut radii: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    let mut fonts: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    let mut typography: std::collections::HashMap<String, (Value, u64)> =
        std::collections::HashMap::new();

    for element in visible_elements(baseline) {
        if let Some(raw) = element["color"].as_str() {
            if opaque_sampled(raw) {
                if let Some(rgb) = parse_color_rgb(raw) {
                    *palette.entry(rgb).or_insert(0) += 1;
                }
            }
        }
        if element["backgroundImage"].as_str().unwrap_or("none") == "none" {
            if let Some(raw) = element["backgroundColor"].as_str() {
                if opaque_sampled(raw) {
                    if let Some(rgb) = parse_color_rgb(raw) {
                        *palette.entry(rgb).or_insert(0) += 1;
                    }
                }
            }
        }
        for value in [
            element["padding"].as_array().cloned().unwrap_or_default(),
            element["margin"].as_array().cloned().unwrap_or_default(),
            element["gap"].as_array().cloned().unwrap_or_default(),
        ]
        .into_iter()
        .flatten()
        {
            if let Some(number) = value.as_f64() {
                if number > 0.0 {
                    *spacing.entry(number.round() as i64).or_insert(0) += 1;
                }
            }
        }
        for value in element["radius"].as_array().cloned().unwrap_or_default() {
            if let Some(raw) = value.as_str() {
                if let Some((number, _)) = parse_dimension(raw) {
                    if number > 0.0 {
                        *radii.entry(raw.to_lowercase()).or_insert(0) += 1;
                    }
                }
            }
        }
        for family in font_families(element["fontFamily"].as_str().unwrap_or("")) {
            if !generic_family(&family) {
                *fonts.entry(family).or_insert(0) += 1;
            }
        }
        if element["hasText"].as_bool().unwrap_or(false) {
            let size = element["fontSize"].as_f64().unwrap_or(0.0);
            if size <= 0.0 {
                continue;
            }
            let weight = element["fontWeight"].as_u64().unwrap_or(400);
            let line_height = element["lineHeight"]
                .as_str()
                .unwrap_or("normal")
                .to_string();
            let letter_spacing = element["letterSpacing"]
                .as_str()
                .unwrap_or("normal")
                .to_string();
            let family = primary_font(element["fontFamily"].as_str().unwrap_or(""));
            let key = format!("{size}|{weight}|{line_height}|{letter_spacing}|{family}");
            let entry = typography.entry(key).or_insert_with(|| {
                (
                    json!({
                        "fontFamily": family,
                        "fontSize": size,
                        "fontWeight": weight,
                        "lineHeight": line_height,
                        "letterSpacing": letter_spacing
                    }),
                    0,
                )
            });
            entry.1 += 1;
        }
    }

    let mut findings = Vec::new();
    let palette_total: u64 = palette.values().sum();
    let spacing_total: u64 = spacing.values().sum();
    let font_total: u64 = fonts.values().sum();

    let base = if spacing_total >= 10 {
        let mut best: Option<(i64, u64)> = None;
        for candidate in [4i64, 8, 2, 3, 5, 6, 10, 12, 16] {
            let hits: u64 = spacing
                .iter()
                .filter(|(value, _)| **value > 0 && **value % candidate == 0)
                .map(|(_, count)| *count)
                .sum();
            if best.map(|(_, count)| hits > count).unwrap_or(true) {
                best = Some((candidate, hits));
            }
        }
        best.and_then(|(candidate, hits)| {
            if hits as f64 / spacing_total as f64 >= 0.7 {
                Some(candidate)
            } else {
                None
            }
        })
    } else {
        None
    };

    if let Some(base) = base {
        let mut off: Vec<(i64, u64)> = spacing
            .iter()
            .filter(|(value, _)| **value > 0 && **value % base != 0)
            .map(|(value, count)| (*value, *count))
            .collect();
        off.sort_by_key(|entry| std::cmp::Reverse(entry.1));
        for (value, count) in off {
            findings.push(json!({
                "check": "inferred-spacing",
                "severity": "P3",
                "inferred": true,
                "selector": ":root",
                "message": "spacing value breaks the inferred base unit",
                "evidence": { "value": value, "count": count, "base": base }
            }));
        }
    }

    let mut exact: Vec<((i64, i64, i64), u64)> = palette
        .iter()
        .map(|(color, count)| (*color, *count))
        .collect();
    exact.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(&right.0)));
    let mut clusters: Vec<(f64, f64, f64, u64)> = Vec::new();
    for (color, count) in &exact {
        if let Some(cluster) = clusters
            .iter_mut()
            .find(|cluster| color_distance_f(color, cluster) <= 24.0)
        {
            let total = cluster.3 + count;
            cluster.0 =
                (cluster.0 * cluster.3 as f64 + color.0 as f64 * *count as f64) / total as f64;
            cluster.1 =
                (cluster.1 * cluster.3 as f64 + color.1 as f64 * *count as f64) / total as f64;
            cluster.2 =
                (cluster.2 * cluster.3 as f64 + color.2 as f64 * *count as f64) / total as f64;
            cluster.3 = total;
        } else {
            clusters.push((color.0 as f64, color.1 as f64, color.2 as f64, *count));
        }
    }

    if palette_total >= 10 {
        let mut singletons: Vec<((i64, i64, i64), f64)> = exact
            .iter()
            .filter(|(_, count)| *count == 1)
            .map(|(color, _)| {
                let nearest = clusters
                    .iter()
                    .filter(|cluster| cluster.3 >= 2)
                    .map(|cluster| color_distance_f(color, cluster))
                    .fold(f64::INFINITY, f64::min);
                (*color, nearest)
            })
            .filter(|(_, distance)| *distance > 72.0)
            .collect();
        singletons.sort_by(|left, right| {
            right
                .1
                .partial_cmp(&left.1)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        for (color, distance) in singletons {
            findings.push(json!({
                "check": "inferred-color",
                "severity": "P3",
                "inferred": true,
                "selector": ":root",
                "message": "singleton colour is far from every palette cluster",
                "evidence": {
                    "color": format!("rgb({}, {}, {})", color.0, color.1, color.2),
                    "distance": (distance * 100.0).round() / 100.0
                }
            }));
        }
    }

    if font_total >= 10 {
        let mut families: Vec<(String, u64)> = fonts
            .iter()
            .map(|(name, count)| (name.clone(), *count))
            .collect();
        families.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(&right.0)));
        let top_two: u64 = families.iter().take(2).map(|(_, count)| *count).sum();
        if families.len() > 2 && top_two as f64 / font_total as f64 >= 0.7 {
            let (family, count) = families[2].clone();
            findings.push(json!({
                "check": "inferred-font",
                "severity": "P3",
                "inferred": true,
                "selector": ":root",
                "message": "third font family appears on a page dominated by two",
                "evidence": { "family": family, "count": count, "total": font_total }
            }));
        }
    }

    let mut palette_entries: Vec<Value> = clusters
        .iter()
        .map(|cluster| {
            json!({
                "color": format!("rgb({}, {}, {})", cluster.0.round() as i64, cluster.1.round() as i64, cluster.2.round() as i64),
                "count": cluster.3,
                "confidence": confidence(cluster.3, palette_total)
            })
        })
        .collect();
    sort_value_entries(&mut palette_entries);

    let mut spacing_entries: Vec<Value> = spacing
        .iter()
        .map(|(value, count)| json!({ "value": value, "count": count, "confidence": confidence(*count, spacing_total) }))
        .collect();
    sort_value_entries(&mut spacing_entries);

    let mut radius_entries: Vec<Value> = radii
        .iter()
        .map(|(value, count)| json!({ "value": value, "count": count, "confidence": confidence(*count, radii.values().sum::<u64>()) }))
        .collect();
    sort_value_entries(&mut radius_entries);

    let mut font_entries: Vec<Value> = fonts
        .iter()
        .map(|(family, count)| json!({ "family": family, "count": count, "confidence": confidence(*count, font_total) }))
        .collect();
    sort_value_entries(&mut font_entries);

    let typography_total: u64 = typography.values().map(|(_, count)| *count).sum();
    let mut typography_entries: Vec<Value> = typography
        .iter()
        .map(|(_, (value, count))| {
            let mut entry = value.clone();
            if let Some(object) = entry.as_object_mut() {
                object.insert("count".to_string(), json!(count));
                object.insert(
                    "confidence".to_string(),
                    json!(confidence(*count, typography_total)),
                );
            }
            entry
        })
        .collect();
    sort_value_entries(&mut typography_entries);

    (
        json!({
            "palette": palette_entries,
            "spacing": {
                "base": base,
                "confidence": base.map(|value| confidence(
                    spacing.iter().filter(|(entry, _)| **entry > 0 && **entry % value == 0).map(|(_, count)| *count).sum(),
                    spacing_total
                )),
                "values": spacing_entries
            },
            "radii": radius_entries,
            "fonts": font_entries,
            "typography": typography_entries
        }),
        findings,
    )
}

pub(crate) fn color_distance_f(color: &(i64, i64, i64), cluster: &(f64, f64, f64, u64)) -> f64 {
    let dr = color.0 as f64 - cluster.0;
    let dg = color.1 as f64 - cluster.1;
    let db = color.2 as f64 - cluster.2;
    (dr * dr + dg * dg + db * db).sqrt()
}
