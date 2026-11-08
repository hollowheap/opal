use crate::{luminance, parse_css_color, visible_elements};
use regex::Regex;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

type Rgba = (f64, f64, f64, f64);

fn text(element: &Value) -> String {
    element["text"].as_str().unwrap_or("").trim().to_string()
}

fn num(element: &Value, key: &str) -> f64 {
    element[key].as_f64().unwrap_or(0.0)
}

fn rect(element: &Value) -> (f64, f64, f64, f64) {
    (
        element["rect"]["x"].as_f64().unwrap_or(0.0),
        element["rect"]["y"].as_f64().unwrap_or(0.0),
        element["rect"]["w"].as_f64().unwrap_or(0.0),
        element["rect"]["h"].as_f64().unwrap_or(0.0),
    )
}

fn tag(element: &Value) -> &str {
    element["tag"].as_str().unwrap_or("")
}

fn has_text(element: &Value) -> bool {
    element["hasText"].as_bool().unwrap_or(false) && !text(element).is_empty()
}

fn visible_text(data: &Value) -> Vec<Value> {
    visible_elements(data)
        .into_iter()
        .filter(has_text)
        .collect()
}

fn add(
    out: &mut Vec<Value>,
    check: &str,
    severity: &str,
    layer: &str,
    selector: &str,
    message: String,
    evidence: Value,
) {
    out.push(json!({
        "check": check,
        "severity": severity,
        "layer": layer,
        "selector": selector,
        "message": message,
        "evidence": evidence
    }));
}

fn parse_hex(raw: &str) -> Option<Rgba> {
    let hex = raw.trim().strip_prefix('#')?;
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

fn color(raw: &str) -> Option<Rgba> {
    parse_css_color(raw).or_else(|| parse_hex(raw))
}

fn chroma(color: Rgba) -> f64 {
    let max = color.0.max(color.1).max(color.2);
    let min = color.0.min(color.1).min(color.2);
    max - min
}

fn hue(color: Rgba) -> f64 {
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

fn colors_in(raw: &str) -> Vec<Rgba> {
    let mut out = Vec::new();
    let hex = Regex::new(r"#[0-9a-fA-F]{3,8}\b").unwrap();
    let rgb = Regex::new(r"rgba?\([^)]*\)").unwrap();
    for capture in hex.find_iter(raw) {
        if let Some(color) = parse_hex(capture.as_str()) {
            out.push(color);
        }
    }
    for capture in rgb.find_iter(raw) {
        if let Some(color) = parse_css_color(capture.as_str()) {
            out.push(color);
        }
    }
    out
}

fn is_dark(color: Rgba) -> bool {
    luminance(color.0, color.1, color.2) < 0.35
}

fn is_chromatic(color: Rgba) -> bool {
    chroma(color) > 25.0 && color.3 > 0.2
}

fn shadow_parts(raw: &str) -> Option<(f64, f64, f64, f64, Rgba, bool)> {
    let first = raw.split("),").next().unwrap_or(raw);
    let inset = first.contains("inset");
    let numbers: Vec<f64> = Regex::new(r"(-?\d*\.?\d+)px")
        .unwrap()
        .captures_iter(first)
        .filter_map(|capture| capture[1].parse::<f64>().ok())
        .collect();
    if numbers.len() < 3 {
        return None;
    }
    let color = colors_in(first)
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

fn element_index(element: &Value) -> usize {
    element["index"].as_u64().unwrap_or(0) as usize
}

fn parent_index(element: &Value) -> Option<usize> {
    element["parentIndex"].as_u64().map(|value| value as usize)
}

fn is_ancestor(elements: &[Value], ancestor: usize, child: usize) -> bool {
    let mut current = elements.get(child).and_then(parent_index);
    while let Some(index) = current {
        if index == ancestor {
            return true;
        }
        current = elements.get(index).and_then(parent_index);
    }
    false
}

fn body_background(elements: &[Value]) -> Option<Rgba> {
    elements
        .iter()
        .find(|element| tag(element) == "body")
        .and_then(|element| element["backgroundColor"].as_str().and_then(color))
}

fn viewport(data: &Value) -> (f64, f64) {
    (
        data["viewport"]["width"].as_f64().unwrap_or(0.0),
        data["viewport"]["height"].as_f64().unwrap_or(0.0),
    )
}

pub fn checks(data: &Value, design_resolved: bool) -> Vec<Value> {
    let elements = data["elements"].as_array().cloned().unwrap_or_default();
    let visible = visible_elements(data);
    let visible_text_elements = visible_text(data);
    let (viewport_width, viewport_height) = viewport(data);
    let background = body_background(&elements);
    let dark_page = background.map(is_dark).unwrap_or(false);
    let mut out = Vec::new();

    // Page and load health.
    for element in &visible {
        if tag(element) == "img" {
            let src = element["src"].as_str().unwrap_or("");
            let lowered = src.to_lowercase();
            let placeholder = [
                "placeholder",
                "placehold",
                "dummyimage",
                "loremflickr",
                "picsum",
                "via.placeholder",
            ]
            .iter()
            .any(|needle| lowered.contains(needle));
            let broken = src.is_empty()
                || (element["complete"].as_bool() == Some(true)
                    && element["naturalWidth"].as_u64().unwrap_or(0) == 0)
                || placeholder;
            if broken {
                add(
                    &mut out,
                    "image-source-missing",
                    "P2",
                    "content",
                    element["selector"].as_str().unwrap_or("?"),
                    "image has no usable source".to_string(),
                    json!({
                        "src": src,
                        "complete": element["complete"],
                        "naturalWidth": element["naturalWidth"],
                        "placeholder": placeholder
                    }),
                );
            }
        }
    }

    let text_elements: Vec<&Value> = elements
        .iter()
        .filter(|element| {
            element["hasText"].as_bool().unwrap_or(false)
                && element["text"].as_str().map(str::len).unwrap_or(0) >= 20
        })
        .collect();
    let hidden_text: Vec<&&Value> = text_elements
        .iter()
        .filter(|element| {
            let opacity = element["opacity"].as_f64().unwrap_or(1.0);
            let visibility = element["visibility"].as_str().unwrap_or("visible");
            let display = element["display"].as_str().unwrap_or("block");
            (opacity <= 0.01 || visibility == "hidden") && display != "none"
        })
        .collect();
    let total_chars: usize = text_elements
        .iter()
        .map(|element| element["text"].as_str().map(str::len).unwrap_or(0))
        .sum();
    let hidden_chars: usize = hidden_text
        .iter()
        .map(|element| element["text"].as_str().map(str::len).unwrap_or(0))
        .sum();
    if text_elements.len() >= 4
        && hidden_text.len() >= 3
        && total_chars > 0
        && hidden_chars * 100 / total_chars >= 40
    {
        add(
            &mut out,
            "hidden-text-share",
            "P2",
            "content",
            "body",
            format!(
                "{} of {} text elements remain hidden after load",
                hidden_text.len(),
                text_elements.len()
            ),
            json!({
                "hiddenElements": hidden_text.len(),
                "textElements": text_elements.len(),
                "hiddenCharacters": hidden_chars,
                "totalCharacters": total_chars
            }),
        );
    }

    // Geometry and layout.
    for element in &visible {
        let radius = element["radiusPx"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(Value::as_f64)
            .fold(0.0, f64::max);
        let widths = element["borderWidths"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let width = widths.iter().filter_map(Value::as_f64).fold(0.0, f64::max);
        if radius > 0.0 && width >= 3.0 {
            let border = element["borderColors"]
                .as_array()
                .and_then(|list| list.iter().filter_map(Value::as_str).find_map(color));
            if border.map(is_chromatic).unwrap_or(false) {
                add(
                    &mut out,
                    "accent-border-rounded",
                    "P3",
                    "layout",
                    element["selector"].as_str().unwrap_or("?"),
                    "a thick chromatic border follows a rounded corner".to_string(),
                    json!({ "borderWidth": width, "radius": radius, "borderColor": element["borderColors"] }),
                );
            }
        }
    }

    let mut rows: HashMap<i64, Vec<&Value>> = HashMap::new();
    for element in &visible {
        if !element["cardLike"].as_bool().unwrap_or(false) {
            continue;
        }
        let (_, y, _, height) = rect(element);
        if height < 4.0 {
            continue;
        }
        rows.entry(y.round() as i64).or_default().push(element);
    }
    for row in rows.values().filter(|row| row.len() >= 2) {
        let mut thin = 0;
        let mut blurred = 0;
        for element in row {
            let widths = element["borderWidths"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            if widths
                .iter()
                .filter_map(Value::as_f64)
                .any(|value| value > 0.0 && value <= 1.5)
            {
                thin += 1;
            }
            if element["focusStyle"]["boxShadow"]
                .as_str()
                .and_then(shadow_parts)
                .map(|(_, _, blur, _, _, _)| blur >= 8.0)
                .unwrap_or(false)
            {
                blurred += 1;
            }
        }
        if thin == row.len() && blurred == row.len() {
            add(
                &mut out,
                "row-elevation-conflict",
                "P3",
                "layout",
                row[0]["selector"].as_str().unwrap_or("?"),
                format!("{0} cards share a hairline border and a wide shadow", thin),
                json!({ "cards": thin, "selectors": row.iter().map(|element| element["selector"].clone()).collect::<Vec<_>>() }),
            );
        }
    }

    let mut children_by_parent: HashMap<usize, Vec<&Value>> = HashMap::new();
    for element in &visible {
        if element["cardLike"].as_bool().unwrap_or(false) {
            if let Some(parent) = parent_index(element) {
                children_by_parent.entry(parent).or_default().push(element);
            }
        }
    }
    for (parent, children) in &children_by_parent {
        if children.len() < 2 {
            continue;
        }
        let Some(container) = elements.get(*parent) else {
            continue;
        };
        let (cx, _, cw, _) = rect(container);
        let left = children
            .iter()
            .map(|child| rect(child).0)
            .fold(f64::MAX, f64::min);
        let right = children
            .iter()
            .map(|child| {
                let (x, _, w, _) = rect(child);
                x + w
            })
            .fold(f64::MIN, f64::max);
        let left_gap = left - cx;
        let right_gap = (cx + cw) - right;
        if (left_gap.abs() < 2.0 && right_gap >= 12.0)
            || (right_gap.abs() < 2.0 && left_gap >= 12.0)
        {
            add(
                &mut out,
                "scroller-edge-flush",
                "P3",
                "layout",
                container["selector"].as_str().unwrap_or("?"),
                "scrolling cards touch one container edge while the other side keeps a gutter"
                    .to_string(),
                json!({ "leftGap": left_gap, "rightGap": right_gap, "cards": children.len() }),
            );
        }
    }

    let mut overlaps = 0;
    for (index, left) in elements.iter().enumerate() {
        if !left["visible"].as_bool().unwrap_or(false) || !has_text(left) {
            continue;
        }
        let (lx, ly, lw, lh) = rect(left);
        if lw <= 0.0 || lh <= 0.0 {
            continue;
        }
        for right in elements.iter().skip(index + 1) {
            if !right["visible"].as_bool().unwrap_or(false) || !has_text(right) {
                continue;
            }
            if text(left) == text(right) || is_ancestor(&elements, index, element_index(right)) {
                continue;
            }
            let (rx, ry, rw, rh) = rect(right);
            if rw <= 0.0 || rh <= 0.0 {
                continue;
            }
            let overlap_w = (lx + lw).min(rx + rw) - lx.max(rx);
            let overlap_h = (ly + lh).min(ry + rh) - ly.max(ry);
            if overlap_w > 4.0 && overlap_h > 4.0 {
                let overlap = overlap_w * overlap_h;
                let smaller = (lw * lh).min(rw * rh);
                if smaller > 0.0 && overlap / smaller >= 0.35 {
                    overlaps += 1;
                    add(
                        &mut out,
                        "overlapping-copy",
                        "P2",
                        "layout",
                        left["selector"].as_str().unwrap_or("?"),
                        "text is painted under another text run".to_string(),
                        json!({
                            "other": right["selector"],
                            "leftText": text(left),
                            "rightText": text(right),
                            "overlapRatio": (overlap / smaller * 100.0).round() / 100.0
                        }),
                    );
                    if overlaps >= 20 {
                        break;
                    }
                }
            }
        }
        if overlaps >= 20 {
            break;
        }
    }

    let mut columns: Vec<(f64, f64, &Value)> = Vec::new();
    for element in &visible {
        let (x, y, w, h) = rect(element);
        if y < 200.0
            && w >= 150.0
            && h >= 100.0
            && !element["interactive"].as_bool().unwrap_or(false)
        {
            columns.push((x, y + h, element));
        }
    }
    if columns.len() >= 2 {
        let min_bottom = columns.iter().map(|entry| entry.1).fold(f64::MAX, f64::min);
        let max_bottom = columns.iter().map(|entry| entry.1).fold(f64::MIN, f64::max);
        if viewport_height > 0.0
            && min_bottom <= viewport_height
            && max_bottom > viewport_height * 1.2
        {
            add(
                &mut out,
                "opening-column-overrun",
                "P2",
                "layout",
                "body",
                "one opening column runs far past the fold while its sibling fits".to_string(),
                json!({ "minBottom": min_bottom, "maxBottom": max_bottom, "viewportHeight": viewport_height }),
            );
        }
    }

    let edge = if viewport_width > 0.0 && viewport_width < 600.0 {
        12.0
    } else {
        16.0
    };
    for element in &visible_text_elements {
        if !["p", "li", "blockquote", "td"].contains(&tag(element)) {
            continue;
        }
        if text(element).len() < 40 {
            continue;
        }
        let (x, _, w, _) = rect(element);
        if w <= 0.0 {
            continue;
        }
        let right = viewport_width - (x + w);
        let padding = element["padding"].as_array().cloned().unwrap_or_default();
        let horizontal_padding = padding.get(1).and_then(Value::as_f64).unwrap_or(0.0)
            + padding.get(3).and_then(Value::as_f64).unwrap_or(0.0);
        if horizontal_padding < 4.0 && (x < edge || right < edge) {
            add(
                &mut out,
                "viewport-edge-text",
                "P2",
                "layout",
                element["selector"].as_str().unwrap_or("?"),
                format!("body text sits within {edge:.0}px of the viewport edge"),
                json!({ "left": x, "right": right, "limit": edge, "text": text(element) }),
            );
        }
    }

    for container in &visible {
        let overflow = container["overflow"].as_str().unwrap_or("");
        if !(overflow.contains("hidden")
            || overflow.contains("clip")
            || overflow.contains("auto")
            || overflow.contains("scroll")
            || container["overflowX"].as_i64().unwrap_or(0) > 0)
        {
            continue;
        }
        let (cx, cy, cw, ch) = rect(container);
        if cw <= 0.0 || ch <= 0.0 {
            continue;
        }
        let container_index = element_index(container);
        for child in elements.iter().skip(container_index + 1) {
            let position = child["position"].as_str().unwrap_or("static");
            if position != "absolute" && position != "fixed" {
                continue;
            }
            if !is_ancestor(&elements, container_index, element_index(child)) {
                continue;
            }
            let (x, y, w, h) = rect(child);
            let escape =
                x < cx - 8.0 || y < cy - 8.0 || x + w > cx + cw + 8.0 || y + h > cy + ch + 8.0;
            if escape {
                add(
                    &mut out,
                    "clipped-escape-child",
                    "P2",
                    "layout",
                    container["selector"].as_str().unwrap_or("?"),
                    "a clipping container wraps a positioned child that escapes it".to_string(),
                    json!({
                        "child": child["selector"],
                        "container": { "x": cx, "y": cy, "w": cw, "h": ch },
                        "childRect": { "x": x, "y": y, "w": w, "h": h }
                    }),
                );
            }
        }
    }

    for element in &visible_text_elements {
        let heading = ["h1", "h2", "h3"].contains(&tag(element));
        if !heading {
            continue;
        }
        let (_, _, _, height) = rect(element);
        if text(element).len() >= 40
            && (num(element, "fontSize") >= 48.0 || height > viewport_height * 0.3)
        {
            add(
                &mut out,
                "oversized-headline",
                "P2",
                "content",
                element["selector"].as_str().unwrap_or("?"),
                "a full headline is set at display size and dominates the viewport".to_string(),
                json!({ "fontSize": element["fontSize"], "height": height, "text": text(element) }),
            );
        }
    }

    let mut spacing_values = Vec::new();
    for element in &visible_text_elements {
        for key in ["margin", "padding"] {
            if let Some(values) = element[key].as_array() {
                for value in values.iter().filter_map(Value::as_f64) {
                    if value > 0.0 {
                        spacing_values.push((value * 10.0).round() / 10.0);
                    }
                }
            }
        }
        if let Some(gaps) = element["gap"].as_array() {
            for value in gaps.iter().filter_map(Value::as_f64) {
                if value > 0.0 {
                    spacing_values.push((value * 10.0).round() / 10.0);
                }
            }
        }
    }
    let unique_spacing: HashSet<String> = spacing_values
        .iter()
        .map(|value| format!("{value:.1}"))
        .collect();
    if spacing_values.len() >= 8 && unique_spacing.len() <= 2 {
        add(
            &mut out,
            "uniform-spacing",
            "P3",
            "layout",
            "body",
            "the same spacing value is repeated everywhere".to_string(),
            json!({ "values": unique_spacing, "samples": spacing_values.len() }),
        );
    }

    // Colour and material.
    let mut palette_hues = Vec::new();
    let mut palette_colors = Vec::new();
    for element in &visible {
        for key in ["color", "backgroundColor"] {
            if let Some(color) = element[key].as_str().and_then(color) {
                if is_chromatic(color) {
                    palette_hues.push(hue(color));
                    palette_colors.push(color);
                }
            }
        }
        for color in colors_in(element["backgroundImage"].as_str().unwrap_or("")) {
            if is_chromatic(color) {
                palette_hues.push(hue(color));
                palette_colors.push(color);
            }
        }
    }
    let purple = palette_hues
        .iter()
        .any(|value| *value >= 250.0 && *value <= 330.0);
    let cyan = palette_hues
        .iter()
        .any(|value| *value >= 175.0 && *value <= 210.0);
    if dark_page && (purple && cyan || palette_hues.len() >= 3) {
        add(
            &mut out,
            "synthetic-purple-palette",
            "P3",
            "layout",
            "body",
            "the palette leans on violet or cyan neon on a dark ground".to_string(),
            json!({ "hues": palette_hues, "colors": palette_colors.len(), "dark": dark_page }),
        );
    }
    if let Some(background) = background {
        if background.0 > 200.0
            && background.1 > 180.0
            && background.2 > 140.0
            && background.2 < background.0 - 8.0
            && chroma(background) < 80.0
        {
            add(
                &mut out,
                "cream-canvas",
                "P3",
                "layout",
                "body",
                "the page uses a warm cream or beige canvas".to_string(),
                json!({ "backgroundColor": [background.0, background.1, background.2] }),
            );
        }
    }

    for element in &visible {
        for shadow in element["focusStyle"]["boxShadow"]
            .as_str()
            .unwrap_or("none")
            .split("),")
        {
            let Some((x, y, blur, _, color, inset)) = shadow_parts(shadow) else {
                continue;
            };
            if inset || !is_chromatic(color) || blur <= 0.0 {
                continue;
            }
            let offset = (x * x + y * y).sqrt();
            if offset < 2.0 || dark_page {
                add(
                    &mut out,
                    "colored-halo-shadow",
                    "P3",
                    "sheet",
                    element["selector"].as_str().unwrap_or("?"),
                    "a chromatic blurred shadow reads as a halo".to_string(),
                    json!({ "boxShadow": shadow.trim(), "offset": offset, "blur": blur, "dark": dark_page }),
                );
            }
        }
    }

    for element in &visible {
        let image = element["backgroundImage"].as_str().unwrap_or("");
        if image.contains("radial-gradient") {
            let colored = colors_in(image).into_iter().any(is_chromatic);
            let (_, y, w, h) = rect(element);
            if colored && dark_page {
                add(
                    &mut out,
                    "radial-background-wash",
                    "P3",
                    "sheet",
                    element["selector"].as_str().unwrap_or("?"),
                    "a chromatic radial wash decorates a dark background".to_string(),
                    json!({ "backgroundImage": image }),
                );
            }
            if colored && y < viewport_height && w > viewport_width * 0.4 && h > 80.0 {
                add(
                    &mut out,
                    "radial-spotlight",
                    "P3",
                    "sheet",
                    element["selector"].as_str().unwrap_or("?"),
                    "a radial gradient sits behind a hero or section as a spotlight".to_string(),
                    json!({ "backgroundImage": image, "rect": element["rect"] }),
                );
            }
        }
        if image.contains("repeating-linear-gradient")
            || image.contains("repeating-radial-gradient")
        {
            add(
                &mut out,
                "repeating-stripe-surface",
                "P3",
                "sheet",
                element["selector"].as_str().unwrap_or("?"),
                "repeating gradient stripes decorate a surface".to_string(),
                json!({ "backgroundImage": image }),
            );
        }
        let grid_layers = image.matches("linear-gradient").count();
        if grid_layers >= 2
            && image.contains("1px")
            && element["backgroundSize"]
                .as_str()
                .map(|value| value.contains("px"))
                .unwrap_or(false)
        {
            add(
                &mut out,
                "grid-line-background",
                "P3",
                "sheet",
                element["selector"].as_str().unwrap_or("?"),
                "a decorative grid is tiled from hairline gradients".to_string(),
                json!({ "backgroundImage": image, "backgroundSize": element["backgroundSize"] }),
            );
        }
    }

    for element in &visible {
        let clip = element["clipPath"].as_str().unwrap_or("none");
        if clip.starts_with("path(")
            || (clip.starts_with("polygon(") && clip.matches(',').count() > 5)
        {
            add(
                &mut out,
                "irregular-clip-path",
                "P3",
                "sheet",
                element["selector"].as_str().unwrap_or("?"),
                "a complex clip path approximates a torn or blob edge".to_string(),
                json!({ "clipPath": clip }),
            );
        }
        let image = element["backgroundImage"].as_str().unwrap_or("");
        if image.contains("url(") {
            let overlay = element["backgroundColor"].as_str().and_then(color);
            let opacity = element["opacity"].as_f64().unwrap_or(1.0);
            if overlay.map(|value| value.3 >= 0.85).unwrap_or(false) || opacity <= 0.1 {
                add(
                    &mut out,
                    "washed-out-image",
                    "P2",
                    "sheet",
                    element["selector"].as_str().unwrap_or("?"),
                    "a raster sits under a near-opaque wash or at near-zero opacity".to_string(),
                    json!({
                        "backgroundImage": image,
                        "backgroundColor": element["backgroundColor"],
                        "opacity": opacity
                    }),
                );
            }
        }
    }

    for element in &visible {
        for shadow in element["focusStyle"]["boxShadow"]
            .as_str()
            .unwrap_or("none")
            .split("),")
        {
            let Some((x, y, blur, _, color, inset)) = shadow_parts(shadow) else {
                continue;
            };
            if inset || blur >= 1.0 || color.3 <= 0.15 {
                continue;
            }
            let offset = (x * x + y * y).sqrt();
            if offset >= 4.0 {
                add(
                    &mut out,
                    "unblurred-offset-shadow",
                    "P3",
                    "sheet",
                    element["selector"].as_str().unwrap_or("?"),
                    "an unblurred shadow is displaced by at least 4px".to_string(),
                    json!({ "boxShadow": shadow.trim(), "offset": offset }),
                );
            }
        }
    }

    for element in &visible_text_elements {
        let Some(foreground) = element["color"].as_str().and_then(color) else {
            continue;
        };
        let Some(backdrop) = element["backgroundColor"].as_str().and_then(color) else {
            continue;
        };
        if chroma(foreground) < 12.0 && chroma(backdrop) > 30.0 {
            add(
                &mut out,
                "washed-grey-on-tint",
                "P3",
                "layout",
                element["selector"].as_str().unwrap_or("?"),
                "grey text sits on a chromatic background and reads washed out".to_string(),
                json!({ "color": element["color"], "backgroundColor": element["backgroundColor"] }),
            );
        }
    }

    for element in &visible {
        if tag(element) == "svg"
            && element["svgShapes"].as_u64().unwrap_or(0) >= 12
            && rect(element).2 >= 320.0
            && rect(element).3 >= 200.0
        {
            add(
                &mut out,
                "primitive-svg-scene",
                "P3",
                "content",
                element["selector"].as_str().unwrap_or("?"),
                "a large inline SVG assembles a scene from primitive shapes".to_string(),
                json!({ "shapes": element["svgShapes"], "rect": element["rect"] }),
            );
        }
    }

    // Type and text.
    for element in &visible_text_elements {
        let font_size = num(element, "fontSize");
        let line_height = element["lineHeight"]
            .as_str()
            .and_then(|value| value.strip_suffix("px"))
            .and_then(|value| value.trim().parse::<f64>().ok());
        if let Some(line_height) = line_height {
            if line_height > 0.0
                && font_size > 0.0
                && element["lineCount"].as_u64().unwrap_or(0) >= 2
            {
                let ratio = line_height / font_size;
                if ratio < 1.3 {
                    add(
                        &mut out,
                        "tight-line-height",
                        "P2",
                        "content",
                        element["selector"].as_str().unwrap_or("?"),
                        format!("line height is {ratio:.2}, below 1.3"),
                        json!({ "ratio": (ratio * 100.0).round() / 100.0, "lineHeight": line_height, "fontSize": font_size }),
                    );
                }
            }
        }
        let align = element["textAlign"].as_str().unwrap_or("start");
        if align == "justify"
            && element["lineCount"].as_u64().unwrap_or(0) >= 2
            && rect(element).2 < 500.0
            && element["hyphens"].as_str().unwrap_or("manual") != "auto"
        {
            add(
                &mut out,
                "justified-narrow-text",
                "P3",
                "content",
                element["selector"].as_str().unwrap_or("?"),
                "justified text runs in a narrow column without hyphenation".to_string(),
                json!({ "width": rect(element).2, "hyphens": element["hyphens"] }),
            );
        }
        let last = element["lastLineWidth"].as_f64().unwrap_or(0.0);
        let max = element["maxLineWidth"].as_f64().unwrap_or(0.0);
        let words = text(element).split_whitespace().count();
        if ["p", "li", "blockquote"].contains(&tag(element))
            && element["lineCount"].as_u64().unwrap_or(0) >= 2
            && max > 0.0
            && last < max * 0.3
            && words >= 6
        {
            add(
                &mut out,
                "short-last-line",
                "P3",
                "content",
                element["selector"].as_str().unwrap_or("?"),
                "the last line holds a single short word".to_string(),
                json!({ "lastLineWidth": last, "maxLineWidth": max, "words": words }),
            );
        }
        if ["h1", "h2", "h3"].contains(&tag(element))
            && element["lineCount"].as_u64().unwrap_or(0) >= 2
            && max > 0.0
            && last < max * 0.45
        {
            add(
                &mut out,
                "short-heading-tail",
                "P3",
                "content",
                element["selector"].as_str().unwrap_or("?"),
                "a heading wraps with a much shorter last line".to_string(),
                json!({ "lastLineWidth": last, "maxLineWidth": max }),
            );
        }
        let interactive = element["interactive"].as_bool().unwrap_or(false);
        if !interactive && text(element).len() >= 20 && font_size < 12.0 {
            add(
                &mut out,
                "sub-12px-body",
                "P2",
                "content",
                element["selector"].as_str().unwrap_or("?"),
                format!("body text is {font_size:.1}px, below 12px"),
                json!({ "fontSize": font_size, "text": text(element) }),
            );
        }
        if (interactive && font_size < 11.0) || (!interactive && font_size < 10.0) {
            add(
                &mut out,
                "sub-11px-control-text",
                "P2",
                "content",
                element["selector"].as_str().unwrap_or("?"),
                format!("UI text is {font_size:.1}px below its floor"),
                json!({ "fontSize": font_size, "interactive": interactive, "text": text(element) }),
            );
        }
        if font_size >= 14.0 && text(element).len() >= 20 {
            if let Some(raw) = element["letterSpacing"].as_str() {
                if let Some(px) = raw
                    .strip_suffix("px")
                    .and_then(|value| value.trim().parse::<f64>().ok())
                {
                    let em = px / font_size;
                    if em > 0.05 {
                        add(
                            &mut out,
                            "excess-letter-spacing",
                            "P3",
                            "content",
                            element["selector"].as_str().unwrap_or("?"),
                            format!("letter spacing is {em:.3}em on body text"),
                            json!({ "em": (em * 1000.0).round() / 1000.0, "letterSpacing": raw }),
                        );
                    }
                }
            }
        }
    }

    let mut by_container: HashMap<(usize, String), Vec<&Value>> = HashMap::new();
    for element in &visible_text_elements {
        let mut current = parent_index(element);
        let mut card = None;
        while let Some(index) = current {
            let Some(parent) = elements.get(index) else {
                break;
            };
            if parent["cardLike"].as_bool().unwrap_or(false) {
                card = Some(index);
                break;
            }
            current = parent_index(parent);
        }
        if let Some(card) = card {
            by_container
                .entry((card, text(element)))
                .or_default()
                .push(element);
        }
    }
    for ((card, value), matches) in by_container {
        if matches.len() >= 3 {
            add(
                &mut out,
                "repeated-container-copy",
                "P2",
                "content",
                elements
                    .get(card)
                    .and_then(|element| element["selector"].as_str())
                    .unwrap_or("?"),
                format!(
                    "the same copy appears {} times in one container",
                    matches.len()
                ),
                json!({ "text": value, "count": matches.len() }),
            );
        }
    }

    let mut index_labels = Vec::new();
    for element in &visible_text_elements {
        let value = text(element);
        if value.len() <= 3
            && value
                .chars()
                .all(|character| character.is_ascii_digit() || character == '.' || character == ')')
            && num(element, "fontSize") < 14.0
            && element["nextIsHeading"].as_bool().unwrap_or(false)
        {
            index_labels.push(value);
        }
    }
    if index_labels.len() >= 2 {
        add(
            &mut out,
            "numeric-section-index",
            "P3",
            "content",
            "body",
            "small numeric indexes ride beside repeated section headings".to_string(),
            json!({ "labels": index_labels }),
        );
    }

    for element in &visible_text_elements {
        let heading = ["h1", "h2"].contains(&tag(element));
        let italic = element["fontStyle"].as_str() == Some("italic");
        let family = element["fontFamily"].as_str().unwrap_or("").to_lowercase();
        let serif =
            family.contains("serif") || family.contains("georgia") || family.contains("times");
        if heading && italic && serif && num(element, "fontSize") >= 40.0 {
            add(
                &mut out,
                "oversized-italic-serif",
                "P3",
                "content",
                element["selector"].as_str().unwrap_or("?"),
                "an oversized italic serif headline carries the display voice".to_string(),
                json!({ "fontSize": element["fontSize"], "fontFamily": element["fontFamily"], "text": text(element) }),
            );
        }
        let value = text(element);
        let letters: Vec<char> = value
            .chars()
            .filter(|character| character.is_alphabetic())
            .collect();
        let uppercase = letters
            .iter()
            .filter(|character| character.is_uppercase())
            .count();
        if value.len() >= 80
            && element["textTransform"].as_str().unwrap_or("none") != "uppercase"
            && !letters.is_empty()
            && uppercase as f64 / letters.len() as f64 >= 0.9
        {
            add(
                &mut out,
                "literal-uppercase-passage",
                "P3",
                "content",
                element["selector"].as_str().unwrap_or("?"),
                "a long passage is written in literal uppercase".to_string(),
                json!({ "characters": value.len(), "uppercaseRatio": (uppercase as f64 / letters.len() as f64 * 100.0).round() / 100.0 }),
            );
        }
    }

    // Motion and behaviour.
    for element in &visible {
        let width = rect(element).2;
        let height = rect(element).3;
        let radius = element["radiusPx"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(Value::as_f64)
            .fold(0.0, f64::max);
        let animation = element["animationName"].as_str().unwrap_or("none");
        let duration = element["animationDuration"].as_str().unwrap_or("0s");
        let animated = animation != "none" && duration != "0s";
        let infinite = element["animationIterationCount"]
            .as_str()
            .unwrap_or("1")
            .contains("infinite");
        let live = element["role"].as_str() == Some("status")
            || element["ariaLive"].as_str().is_some()
            || element["ariaBusy"].as_str() == Some("true");
        let live_text = text(element).to_lowercase();
        let genuine = live
            || ["running", "live", "online", "active"]
                .iter()
                .any(|needle| live_text.contains(needle));
        if width <= 16.0
            && height <= 16.0
            && radius >= width.min(height) * 0.5
            && animated
            && infinite
        {
            if !genuine {
                add(
                    &mut out,
                    "pulsing-status-dot",
                    "P3",
                    "sheet",
                    element["selector"].as_str().unwrap_or("?"),
                    "a small pulsing dot decorates the page without marking live state".to_string(),
                    json!({ "animation": animation, "duration": duration, "rect": element["rect"] }),
                );
            }
        }
        let lowered = animation.to_lowercase();
        if (lowered.contains("blink")
            || lowered.contains("caret")
            || lowered.contains("cursor")
            || (animated && width <= 3.0 && height >= 12.0))
            && rect(element).1 < viewport_height * 0.6
            && element["contentEditable"].as_str() != Some("true")
            && tag(element) != "input"
            && tag(element) != "textarea"
        {
            add(
                &mut out,
                "decorative-blinking-caret",
                "P3",
                "sheet",
                element["selector"].as_str().unwrap_or("?"),
                "a blinking caret is animated where no input exists".to_string(),
                json!({ "animation": animation, "rect": element["rect"] }),
            );
        }
        if lowered.contains("ticker")
            || (animated
                && infinite
                && element["overflow"]
                    .as_str()
                    .unwrap_or("")
                    .contains("hidden")
                && element["overflowX"].as_i64().unwrap_or(0) > 0)
        {
            add(
                &mut out,
                "continuous-scroller",
                "P3",
                "sheet",
                element["selector"].as_str().unwrap_or("?"),
                "content scrolls continuously and hides part of itself".to_string(),
                json!({ "animation": animation, "overflowX": element["overflowX"] }),
            );
        }
    }

    // Partial remainders.
    for element in &visible_text_elements {
        let padding = element["padding"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(Value::as_f64)
            .fold(f64::MAX, f64::min);
        let bordered = element["borderWidths"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(Value::as_f64)
            .any(|value| value > 0.0);
        let colored = element["backgroundColor"]
            .as_str()
            .and_then(color)
            .map(|value| value.3 > 0.1)
            .unwrap_or(false);
        if padding < 16.0 && (bordered || colored) && text(element).len() >= 10 {
            let severity = if padding < 8.0 { "P2" } else { "P3" };
            add(
                &mut out,
                "tight-container-inset",
                severity,
                "layout",
                element["selector"].as_str().unwrap_or("?"),
                format!("text sits {padding:.1}px from a visible container edge"),
                json!({ "padding": padding, "bordered": bordered, "colored": colored }),
            );
        }
    }

    if !design_resolved {
        let mut families: HashMap<String, u64> = HashMap::new();
        for element in &visible_text_elements {
            if !["p", "li", "blockquote", "body"].contains(&tag(element)) {
                continue;
            }
            if let Some(family) = element["fontFamily"].as_str() {
                let first = family
                    .split(',')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_lowercase();
                if !first.is_empty() {
                    *families.entry(first).or_insert(0) += 1;
                }
            }
        }
        let reflex = [
            "inter",
            "roboto",
            "fraunces",
            "geist",
            "plus jakarta sans",
            "space grotesk",
        ];
        if let Some((family, count)) = families
            .iter()
            .filter(|(family, count)| reflex.contains(&family.as_str()) && **count >= 3)
            .max_by_key(|(_, count)| *count)
        {
            add(
                &mut out,
                "reflex-font-choice",
                "P3",
                "content",
                "body",
                format!("the page leans on the reflex face {family}"),
                json!({ "family": family, "count": count, "designResolved": false }),
            );
        }
    }

    let mut sizes_by_parent: HashMap<usize, Vec<f64>> = HashMap::new();
    for element in &visible_text_elements {
        if ["h1", "h2", "h3", "p"].contains(&tag(element)) {
            let size = num(element, "fontSize");
            if size > 0.0 {
                if let Some(parent) = parent_index(element) {
                    sizes_by_parent
                        .entry(parent)
                        .or_default()
                        .push((size * 10.0).round() / 10.0);
                }
            }
        }
    }
    for (parent, mut sizes) in sizes_by_parent {
        sizes.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
        sizes.dedup();
        if sizes.len() >= 2 && sizes.windows(2).all(|pair| pair[1] / pair[0] < 1.25) {
            add(
                &mut out,
                "compressed-type-scale",
                "P3",
                "content",
                elements
                    .get(parent)
                    .and_then(|element| element["selector"].as_str())
                    .unwrap_or("body"),
                "heading and body sizes never separate by 1.25x".to_string(),
                json!({ "sizes": sizes }),
            );
        }
    }

    for element in &visible_text_elements {
        if element["nextIsHeading"].as_bool().unwrap_or(false) && text(element).len() <= 40 {
            let radius = element["radiusPx"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .iter()
                .filter_map(Value::as_f64)
                .fold(0.0, f64::max);
            let padding = element["padding"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .iter()
                .filter_map(Value::as_f64)
                .fold(0.0, f64::max);
            let filled = element["backgroundColor"]
                .as_str()
                .and_then(color)
                .map(|value| value.3 > 0.1)
                .unwrap_or(false);
            if radius >= 999.0 || (radius > 0.0 && padding > 0.0 && filled) {
                add(
                    &mut out,
                    "pill-eyebrow-label",
                    "P3",
                    "layout",
                    element["selector"].as_str().unwrap_or("?"),
                    "the eyebrow is a filled pill chip rather than a label".to_string(),
                    json!({ "text": text(element), "radius": radius, "padding": padding, "filled": filled }),
                );
            }
        }
    }

    let mut siblings: HashMap<usize, Vec<&Value>> = HashMap::new();
    for element in &visible_text_elements {
        if let Some(parent) = parent_index(element) {
            siblings.entry(parent).or_default().push(element);
        }
    }
    for group in siblings.values() {
        if group.len() < 2 {
            continue;
        }
        for (index, left) in group.iter().enumerate() {
            let (lx, ly, lw, lh) = rect(left);
            if lw <= 0.0 || lh <= 0.0 {
                continue;
            }
            for right in group.iter().skip(index + 1) {
                let (rx, ry, rw, rh) = rect(right);
                if rw <= 0.0 || rh <= 0.0 || text(left) == text(right) {
                    continue;
                }
                let overlap_w = (lx + lw).min(rx + rw) - lx.max(rx);
                let overlap_h = (ly + lh).min(ry + rh) - ly.max(ry);
                let block = left["display"].as_str().unwrap_or("block") != "inline"
                    || right["display"].as_str().unwrap_or("block") != "inline";
                if block && overlap_w > 4.0 && overlap_h > 4.0 {
                    add(
                        &mut out,
                        "sibling-text-collision",
                        "P2",
                        "layout",
                        left["selector"].as_str().unwrap_or("?"),
                        "text runs into a sibling box".to_string(),
                        json!({
                            "other": right["selector"],
                            "leftText": text(left),
                            "rightText": text(right),
                            "overlap": { "w": overlap_w, "h": overlap_h }
                        }),
                    );
                }
            }
        }
    }

    // Heading rhythm, relational law.
    for element in &visible_text_elements {
        if !["h1", "h2", "h3", "h4"].contains(&tag(element)) {
            continue;
        }
        let margins = element["margin"].as_array().cloned().unwrap_or_default();
        let top = margins.first().and_then(Value::as_f64).unwrap_or(0.0);
        let bottom = margins.get(2).and_then(Value::as_f64).unwrap_or(0.0);
        if top > 0.0 && bottom >= top {
            add(
                &mut out,
                "heading-spacing-relation",
                "P3",
                "layout",
                element["selector"].as_str().unwrap_or("?"),
                "space below a heading is not smaller than the space above".to_string(),
                json!({ "marginTop": top, "marginBottom": bottom }),
            );
        }
    }

    out
}
