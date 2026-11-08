use anyhow::{Context, Result};
use headless_chrome::browser::tab::Tab;
use headless_chrome::protocol::cdp::Emulation;
use headless_chrome::Browser;
use regex::Regex;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const JS: &str = r##"
(() => {
  const esc = (s) => (window.CSS && CSS.escape ? CSS.escape(s) : s.replace(/[^a-zA-Z0-9_-]/g, "\\$&"));
  const selector = (el) => {
    const parts = [];
    let n = el;
    while (n && n.nodeType === 1 && parts.length < 6) {
      if (n.id) { parts.unshift(n.tagName.toLowerCase() + "#" + esc(n.id)); break; }
      let p = n.tagName.toLowerCase();
      const parent = n.parentElement;
      if (parent) {
        const sibs = Array.from(parent.children).filter((c) => c.tagName === n.tagName);
        if (sibs.length > 1) p += ":nth-of-type(" + (sibs.indexOf(n) + 1) + ")";
      }
      parts.unshift(p);
      n = n.parentElement;
    }
    return parts.join(" > ");
  };
  const canvas = document.createElement("canvas");
  const context = canvas.getContext("2d");
  const clamp = (value, min, max) => Math.min(max, Math.max(min, value));
  const paint = (raw) => {
    if (raw === null || raw === undefined) return null;
    const value = String(raw).trim();
    if (value === "" || value === "none") return null;
    if (!context) return null;
    context.fillStyle = "#000000";
    context.fillStyle = value;
    const first = context.fillStyle;
    context.fillStyle = "#ffffff";
    context.fillStyle = value;
    const second = context.fillStyle;
    if (first !== second) return null;
    let r = 0;
    let g = 0;
    let b = 0;
    let a = 1;
    if (first.charAt(0) === "#") {
      let hex = first.slice(1);
      if (hex.length === 3 || hex.length === 4) {
        hex = hex.split("").map((digit) => digit + digit).join("");
      }
      if (hex.length !== 6 && hex.length !== 8) return null;
      r = parseInt(hex.slice(0, 2), 16);
      g = parseInt(hex.slice(2, 4), 16);
      b = parseInt(hex.slice(4, 6), 16);
      if (hex.length === 8) a = parseInt(hex.slice(6, 8), 16) / 255;
      if (![r, g, b].every((channel) => Number.isFinite(channel))) return null;
    } else {
      const match = /^rgba?\(([^)]+)\)$/.exec(first);
      if (!match) return null;
      const parts = match[1].split(/[ ,\/]+/).filter(Boolean).map(Number);
      if (parts.length < 3 || !parts.slice(0, 3).every((channel) => Number.isFinite(channel))) return null;
      r = parts[0];
      g = parts[1];
      b = parts[2];
      if (parts.length > 3 && Number.isFinite(parts[3])) a = clamp(parts[3], 0, 1);
    }
    r = Math.round(clamp(r, 0, 255) * 1000) / 1000;
    g = Math.round(clamp(g, 0, 255) * 1000) / 1000;
    b = Math.round(clamp(b, 0, 255) * 1000) / 1000;
    a = Math.round(clamp(a, 0, 1) * 1000) / 1000;
    return { r, g, b, a, css: "rgba(" + r + ", " + g + ", " + b + ", " + a + ")" };
  };
  const compositedBg = (el) => {
    const layers = [];
    for (let n = el; n && n.nodeType === 1; n = n.parentElement) {
      const cs = getComputedStyle(n);
      if (cs.backgroundImage && cs.backgroundImage !== "none") return { unknown: "background-image" };
      const color = paint(cs.backgroundColor);
      if (color === null) return { unknown: "colour-conversion-failed", value: cs.backgroundColor };
      if (color.a > 0) {
        layers.push(color);
        if (color.a >= 1) break;
      }
    }
    let out = { r: 255, g: 255, b: 255 };
    for (let index = layers.length - 1; index >= 0; index -= 1) {
      const color = layers[index];
      out = {
        r: color.r * color.a + out.r * (1 - color.a),
        g: color.g * color.a + out.g * (1 - color.a),
        b: color.b * color.a + out.b * (1 - color.a)
      };
    }
    return out;
  };
  const visible = (el) => {
    for (let n = el; n && n.nodeType === 1; n = n.parentElement) {
      const cs = getComputedStyle(n);
      if (cs.display === "none" || cs.visibility === "hidden") return false;
      if (cs.contentVisibility === "hidden") return false;
      if (parseFloat(cs.opacity) === 0) return false;
      if (n.tagName === "DETAILS" && !n.open) {
        const summary = n.querySelector(":scope > summary");
        if (el !== summary && !(summary && summary.contains(el))) return false;
      }
    }
    const r = el.getBoundingClientRect();
    return r.width > 0 && r.height > 0;
  };
  const accessibleName = (el) => {
    const aria = el.getAttribute("aria-label");
    if (aria && aria.trim()) return aria.trim();
    const by = el.getAttribute("aria-labelledby");
    if (by) {
      const t = by.split(/\s+/).map((id) => {
        const e = document.getElementById(id);
        return e ? (e.innerText || e.textContent || "") : "";
      }).join(" ").trim();
      if (t) return t;
    }
    const alt = el.getAttribute("alt");
    if (alt && alt.trim()) return alt.trim();
    const title = el.getAttribute("title");
    if (title && title.trim()) return title.trim();
    return (el.innerText || el.value || el.textContent || "").trim().slice(0, 120);
  };
  const interactiveSel = "a[href],button,input,select,textarea,summary,[role=button],[role=link],[role=tab],[role=menuitem],[role=switch],[role=checkbox],[role=radio],[tabindex]";
  const cardCache = new Map();
  const isCard = (el) => {
    if (cardCache.has(el)) return cardCache.get(el);
    const cs = getComputedStyle(el);
    const widths = [cs.borderTopWidth, cs.borderRightWidth, cs.borderBottomWidth, cs.borderLeftWidth].map((value) => parseFloat(value) || 0);
    const radii = [cs.borderTopLeftRadius, cs.borderTopRightRadius, cs.borderBottomRightRadius, cs.borderBottomLeftRadius].map((value) => parseFloat(value) || 0);
    const border = widths.some((value) => value > 0);
    const radius = radii.some((value) => value > 0);
    const shadow = cs.boxShadow && cs.boxShadow !== "none";
    const image = cs.backgroundImage && cs.backgroundImage !== "none";
    const own = paint(cs.backgroundColor);
    const opaque = own !== null && own.a >= 1;
    const interactive = el.matches(interactiveSel);
    const card = !interactive && (border || radius || shadow || image) && (opaque || image);
    cardCache.set(el, card);
    return card;
  };
  const rawEntries = [];
  let marker = 0;
  const walkRoot = (root, host) => {
    for (const el of Array.from(root.querySelectorAll("*"))) {
      el.setAttribute("data-ui-probe", String(marker));
      rawEntries.push({ el, host, index: marker });
      marker += 1;
      if (el.shadowRoot) walkRoot(el.shadowRoot, el);
    }
  };
  walkRoot(document.documentElement, null);
  const position = new Map(rawEntries.map((entry, index) => [entry.el, index]));
  const ignoreRules = (el) => {
    const out = [];
    for (let n = el; n && n.nodeType === 1; n = n.parentElement) {
      const raw = n.getAttribute("data-opal-ignore");
      if (raw) out.push(...raw.toLowerCase().split(/[\s,]+/).filter(Boolean));
    }
    return out;
  };
  const elements = rawEntries.map((entry, index) => {
    const el = entry.el;
    const cs = getComputedStyle(el);
    const r = el.getBoundingClientRect();
    const directText = Array.from(el.childNodes)
      .filter((n) => n.nodeType === 3)
      .map((n) => n.textContent || "")
      .join("")
      .trim();
    const hasText = directText.length > 0;
    const tag = el.tagName.toLowerCase();
    const colourFailures = [];
    const sample = (property, raw) => {
      const color = paint(raw);
      if (color === null && raw && String(raw).trim() !== "none") {
        colourFailures.push({ property: String(property), value: String(raw).slice(0, 80) });
      }
      return color ? color.css : null;
    };
    const color = sample("color", cs.color);
    const borderColors = [cs.borderTopColor, cs.borderRightColor, cs.borderBottomColor, cs.borderLeftColor]
      .map((value) => sample("border-color", value));
    const caretColor = sample("caret-color", cs.caretColor);
    const backgroundColor = sample("background-color", cs.backgroundColor);
    let chWidth = 0;
    if (context) {
      context.font = cs.font;
      chWidth = context.measureText("0").width;
    }
    let cardDepth = 0;
    for (let parent = el.parentElement; parent && parent.nodeType === 1; parent = parent.parentElement) {
      if (isCard(parent)) cardDepth += 1;
    }
    const nextSibling = el.nextElementSibling;
    const nextIsHeading = nextSibling !== null && /^h[1-6]$/i.test(nextSibling.tagName);
    const nextSize = nextIsHeading ? parseFloat(getComputedStyle(nextSibling).fontSize) || 0 : 0;
    const letterSpacing = parseFloat(cs.letterSpacing) || 0;
    const eyebrow = nextIsHeading
      && hasText
      && directText.length <= 40
      && parseFloat(cs.fontSize) < nextSize
      && (cs.textTransform === "uppercase" || letterSpacing > parseFloat(cs.fontSize) * 0.04);
    const parent = el.parentElement || entry.host;
    const parentIndex = parent && position.has(parent) ? position.get(parent) : null;
    const shadowHost = entry.host && position.has(entry.host) ? position.get(entry.host) : null;
    const iconTile = Math.abs(r.width - r.height) <= 4
      && r.width > 0
      && r.width <= 64
      && el.querySelector("svg,img") !== null
      && nextIsHeading;
    const numeric = directText.length > 0
      && /^[\s$€£¥+\-.,%0-9]+$/.test(directText)
      && (directText.match(/[0-9]/g) || []).length >= 2;
    const heroMetric = numeric && parseFloat(cs.fontSize) >= 40;
    return {
      index,
      selector: selector(el),
      id: el.id || null,
      tag,
      role: el.getAttribute("role"),
      name: accessibleName(el),
      hasText,
      disabled: el.disabled === true || el.getAttribute("aria-disabled") === "true",
      text: (el.innerText || "").trim(),
      classList: el.getAttribute("class"),
      parentIndex,
      shadowHost,
      visible: visible(el),
      textTransform: cs.textTransform,
      caretColor,
      color,
      background: compositedBg(el),
      backgroundColor,
      fontSize: parseFloat(cs.fontSize) || 0,
      fontWeight: parseInt(cs.fontWeight, 10) || 400,
      padding: [cs.paddingTop, cs.paddingRight, cs.paddingBottom, cs.paddingLeft].map((v) => parseFloat(v) || 0),
      margin: [cs.marginTop, cs.marginRight, cs.marginBottom, cs.marginLeft].map((v) => parseFloat(v) || 0),
      gap: [cs.rowGap, cs.columnGap].filter((v) => v && v !== "normal").map((v) => parseFloat(v) || 0),
      radius: [cs.borderTopLeftRadius, cs.borderTopRightRadius, cs.borderBottomRightRadius, cs.borderBottomLeftRadius],
      radiusPx: [cs.borderTopLeftRadius, cs.borderTopRightRadius, cs.borderBottomRightRadius, cs.borderBottomLeftRadius].map((v) => parseFloat(v) || 0),
      fontFamily: cs.fontFamily,
      borderColors,
      backgroundImage: cs.backgroundImage,
      backgroundClip: cs.webkitBackgroundClip || cs.backgroundClip,
      letterSpacing: cs.letterSpacing,
      lineHeight: cs.lineHeight,
      textLength: directText.length,
      chWidth,
      cardLike: isCard(el),
      cardDepth,
      nextIsHeading,
      nextSize,
      eyebrow,
      iconTile,
      heroMetric,
      borderWidths: [cs.borderTopWidth, cs.borderRightWidth, cs.borderBottomWidth, cs.borderLeftWidth].map((v) => parseFloat(v) || 0),
      transitionDuration: cs.transitionDuration,
      transitionTimingFunction: cs.transitionTimingFunction,
      animationDuration: cs.animationDuration,
      animationTimingFunction: cs.animationTimingFunction,
      animationName: cs.animationName,
      focusStyle: {
        outline: [cs.outlineStyle, cs.outlineWidth, cs.outlineColor].join(" "),
        boxShadow: cs.boxShadow,
        border: [cs.borderTopWidth, cs.borderRightWidth, cs.borderBottomWidth, cs.borderLeftWidth, cs.borderTopStyle, cs.borderTopColor].join(" ")
      },
      rect: { x: r.x + window.scrollX, y: r.y + window.scrollY, w: r.width, h: r.height },
      overflowX: el.scrollWidth - el.clientWidth,
      overflowY: el.scrollHeight - el.clientHeight,
      overflow: cs.overflowX + "/" + cs.overflowY,
      whiteSpace: cs.whiteSpace,
      textOverflow: cs.textOverflow,
      tabindex: el.getAttribute("tabindex"),
      interactive: el.matches(interactiveSel),
      expanded: el.getAttribute("aria-expanded"),
      hasPopup: el.getAttribute("aria-haspopup"),
      href: tag === "a" ? el.href : null,
      colourFailures,
      ignoreRules: ignoreRules(el)
    };
  });
  const focusableSel = "a[href],button,input,select,textarea,summary,[tabindex]";
  const hiddenAncestor = (el) => {
    for (let n = el; n && n.nodeType === 1; n = n.parentElement) {
      if (n.getAttribute("aria-hidden") === "true") return "aria-hidden";
      if (n.hasAttribute("inert")) return "inert";
    }
    return null;
  };
  const focusableElements = Array.from(document.querySelectorAll(focusableSel))
    .filter((el) => !el.disabled && el.tabIndex >= 0 && visible(el));
  const orderedFocusables = [
    ...focusableElements.filter((el) => el.tabIndex > 0).sort((a, b) => a.tabIndex - b.tabIndex),
    ...focusableElements.filter((el) => el.tabIndex === 0)
  ];
  const focusables = orderedFocusables.map((el, index) => ({
    order: index + 1,
    selector: selector(el),
    name: accessibleName(el),
    tabIndex: el.tabIndex,
    hiddenBy: hiddenAncestor(el)
  }));
  const candidates = elements.filter((e) => e.visible && e.interactive && !e.disabled).map((e) => {
    let kind = "action";
    if (e.expanded !== null || e.hasPopup !== null || e.tag === "summary") kind = "disclosure";
    else if (e.tag === "a" && e.href) kind = "navigation";
    return { selector: e.selector, name: e.name, tag: e.tag, kind };
  });
  return JSON.stringify({
    title: document.title,
    url: location.href,
    lang: document.documentElement.lang || null,
    viewport: {
      width: window.innerWidth,
      height: window.innerHeight,
      documentWidth: document.documentElement.scrollWidth
    },
    scheme: window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light",
    reducedMotion: window.matchMedia("(prefers-reduced-motion: reduce)").matches,
    headings: Array.from(document.querySelectorAll("h1,h2,h3,h4,h5,h6")).map((h) => h.tagName.toLowerCase()),
    closedShadowSuspects: Array.from(document.querySelectorAll("*")).filter((el) => el.tagName.includes("-") && el.shadowRoot === null).length,
    pageText: (document.body.innerText || "").slice(0, 1_000_000),
    counts: {
      interactive: candidates.length,
      focusables: focusables.length,
      formControls: document.querySelectorAll("input,select,textarea,button").length,
      textInputs: Array.from(document.querySelectorAll("input,textarea")).filter((el) => {
        if (el.tagName.toLowerCase() === "textarea") return true;
        const type = (el.getAttribute("type") || "text").toLowerCase();
        return ["text", "search", "email", "url", "tel", "password", "number"].includes(type);
      }).length
    },
    elements,
    focusables,
    candidates
  });
})()
"##;

const STYLES_JS: &str = r#" (async () => {
  const sheets = [];
  const unreadable = [];
  for (const sheet of Array.from(document.styleSheets)) {
    const href = sheet.href || null;
    let rules = null;
    try {
      rules = [];
      const flatten = (list, condition) => {
        for (const rule of Array.from(list)) {
          if (rule.type === 7) {
            rules.push({ selector: null, text: rule.cssText, condition: condition || null, keyframes: rule.name });
          } else if (rule.cssRules && rule.cssRules.length > 0 && rule.conditionText) {
            flatten(rule.cssRules, condition ? condition + " and " + rule.conditionText : rule.conditionText);
          } else if (rule.cssRules && rule.cssRules.length > 0) {
            flatten(rule.cssRules, condition);
          } else {
            rules.push({ selector: rule.selectorText || null, text: rule.cssText, condition: condition || null });
          }
        }
      };
      flatten(sheet.cssRules, null);
    } catch (error) {
      rules = null;
    }
    if (rules === null && href) {
      try {
        const response = await fetch(href, { credentials: "omit" });
        if (response.ok) {
          const text = await response.text();
          rules = [{ selector: null, text: text.slice(0, 2_000_000), condition: null }];
        }
      } catch (error) {
        rules = null;
      }
    }
    if (rules === null) {
      unreadable.push(href || "inline");
      continue;
    }
    sheets.push({ href, rules: rules.slice(0, 5000) });
  }
  return JSON.stringify({ sheets, unreadable });
})()"#;

const FOCUS_JS: &str = r##"(() => {
  const esc = (s) => (window.CSS && CSS.escape ? CSS.escape(s) : s.replace(/[^a-zA-Z0-9_-]/g, "\\$&"));
  const selector = (el) => {
    const parts = [];
    let n = el;
    while (n && n.nodeType === 1 && parts.length < 6) {
      if (n.id) { parts.unshift(n.tagName.toLowerCase() + "#" + esc(n.id)); break; }
      let p = n.tagName.toLowerCase();
      const parent = n.parentElement;
      if (parent) {
        const sibs = Array.from(parent.children).filter((c) => c.tagName === n.tagName);
        if (sibs.length > 1) p += ":nth-of-type(" + (sibs.indexOf(n) + 1) + ")";
      }
      parts.unshift(p);
      n = n.parentElement;
    }
    return parts.join(" > ");
  };
  const name = (el) => (el.getAttribute("aria-label") || el.innerText || el.value || el.getAttribute("title") || "").trim().slice(0, 120);
  const style = (el) => {
    const cs = getComputedStyle(el);
    return {
      outline: [cs.outlineStyle, cs.outlineWidth, cs.outlineColor].join(" "),
      boxShadow: cs.boxShadow,
      border: [cs.borderTopWidth, cs.borderRightWidth, cs.borderBottomWidth, cs.borderLeftWidth, cs.borderTopStyle, cs.borderTopColor].join(" ")
    };
  };
  const focusableSel = "a[href],button,input,select,textarea,summary,[tabindex]";
  const els = Array.from(document.querySelectorAll(focusableSel)).filter((el) => {
    const r = el.getBoundingClientRect();
    const cs = getComputedStyle(el);
    return !el.disabled && el.tabIndex >= 0 && r.width > 0 && r.height > 0 && cs.visibility !== "hidden";
  });
  els.forEach((el, index) => el.setAttribute("data-ui-probe-focus", String(index)));
  const out = [];
  for (let index = 0; index < els.length; index++) {
    const el = document.querySelector('[data-ui-probe-focus="' + index + '"]');
    if (!el) continue;
    el.blur();
    const before = style(el);
    el.focus();
    const after = style(el);
    out.push({
      selector: selector(el),
      name: name(el),
      changed: before.outline !== after.outline || before.boxShadow !== after.boxShadow || before.border !== after.border,
      before,
      after
    });
    el.blur();
  }
  if (document.activeElement && document.activeElement.blur) document.activeElement.blur();
  return JSON.stringify(out);
})()"##;

const DISCOVER_JS: &str = r##"(() => {
  const out = [];
  const seen = new Set();
  const add = (el, kind) => {
    if (!el || seen.has(el)) return;
    seen.add(el);
    el.setAttribute("data-ui-probe-target", String(out.length));
    const name = (el.getAttribute("aria-label") || el.innerText || el.getAttribute("title") || "").trim().slice(0, 80);
    const selector = el.id ? "#" + el.id : el.tagName.toLowerCase();
    out.push({ index: out.length, name, kind, selector });
  };
  document.querySelectorAll("details:not([open]) > summary").forEach((el) => add(el, "details"));
  document.querySelectorAll('[aria-expanded="false"]').forEach((el) => add(el, "aria"));
  document.querySelectorAll('[role="tab"][aria-selected="false"]').forEach((el) => add(el, "tab"));
  return JSON.stringify(out);
})()"##;

const ORACLE_JS: &str = r##"((selectors, properties) => {
  const roots = [document];
  const scan = (root) => {
    for (const el of root.querySelectorAll("*")) {
      if (el.shadowRoot) {
        roots.push(el.shadowRoot);
        scan(el.shadowRoot);
      }
    }
  };
  scan(document);
  const byIndex = new Map();
  for (const root of roots) {
    for (const el of root.querySelectorAll("*")) {
      const raw = el.getAttribute("data-ui-probe");
      if (raw !== null) byIndex.set(Number(raw), el);
    }
  }
  const result = {};
  for (const selector of selectors) {
    const matches = [];
    let error = null;
    for (const root of roots) {
      let list;
      try {
        list = root.querySelectorAll(selector);
      } catch (exception) {
        error = String(exception);
        break;
      }
      for (const el of list) {
        const raw = el.getAttribute("data-ui-probe");
        if (raw === null) continue;
        const index = Number(raw);
        if (!matches.includes(index)) matches.push(index);
      }
    }
    const values = {};
    for (const property of properties) {
      const map = {};
      if (error === null) {
        for (const index of matches) {
          const el = byIndex.get(index);
          if (el) map[String(index)] = getComputedStyle(el).getPropertyValue(property);
        }
      }
      values[property] = map;
    }
    result[selector] = { matches, values, error };
  }
  return JSON.stringify({ selectors: result });
})"##;

fn oracle_js(selectors: &[String], properties: &[String]) -> String {
    format!(
        "({})({}, {})",
        ORACLE_JS,
        serde_json::to_string(selectors).unwrap_or_else(|_| "[]".to_string()),
        serde_json::to_string(properties).unwrap_or_else(|_| "[]".to_string())
    )
}

fn validate_js(selectors: &[String]) -> String {
    format!(
        r#"((selectors) => {{
  const errors = [];
  for (const selector of selectors) {{
    try {{ document.querySelector(selector); }} catch (error) {{ errors.push({{ selector, error: String(error) }}); }}
  }}
  return JSON.stringify(errors);
}})({})"#,
        serde_json::to_string(selectors).unwrap_or_else(|_| "[]".to_string())
    )
}

fn activate_js(index: usize) -> String {
    format!(
        r#"(() => {{ const el = document.querySelector('[data-ui-probe-target="{index}"]'); if (!el) return false; el.click(); return true; }})()"#
    )
}

fn activated_js(index: usize) -> String {
    format!(
        r#"(() => {{ const el = document.querySelector('[data-ui-probe-target="{index}"]'); if (!el) return false; return el.getAttribute("aria-expanded") === "true" || el.getAttribute("aria-selected") === "true" || (el.tagName === "SUMMARY" && el.parentElement && el.parentElement.open === true); }})()"#
    )
}

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
                    "usage: ui-probe init [DIR]\n       ui-probe audit URL [options]\n       ui-probe source [DIR] [options]\n       ui-probe pack [--rules PATH] [--json]\n       ui-probe --url URL [options]\n       ui-probe --source DIR [--pretty]\n\noptions: --wait-for SELECTOR --viewport WxH --timeout MS --launch-timeout MS\n         --state NAME --theme light|dark --viewport WxH|sweep --chrome PATH\n         --no-sandbox --expand --max-states N --design PATH|auto --infer-missing\n         --force --sample-states --allow id,id --source-all --rules PATH\n         --baseline PATH --write-baseline --update-baseline --no-baseline\n\nrules resolve from --rules, the nearest .opal/rules.json, then ~/.opal/rules.json;\nnamed rules overlay by id and structural rules append.\ndesign layers --design, the nearest .opal/design.json, then ~/.opal/design.json."
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
    })
}

fn run() -> Result<i32> {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let command = raw.first().cloned();
    match command.as_deref() {
        Some("init") => init_mode(&raw[1..]),
        Some("pack") => pack_mode(&raw[1..]),
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

fn resolve_baseline_path(args: &Args) -> Option<PathBuf> {
    if args.no_baseline {
        return None;
    }
    if let Some(path) = &args.baseline {
        return Some(path.clone());
    }
    let cwd = std::env::current_dir().ok()?;
    if let Some(config) = project_config_dir(&cwd) {
        let path = config.join("baseline.json");
        if path.is_file() || args.write_baseline || args.update_baseline {
            return Some(path);
        }
    }
    if args.write_baseline || args.update_baseline {
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
        let Some(path) = resolve_baseline_path(args) else {
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
        let Some(path) = resolve_baseline_path(args) else {
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
- `rules.json` is optional. Named rules overlay by id; structural rules append.
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

    let baseline_path = resolve_baseline_path(&args);
    let baseline = load_baseline(
        baseline_path.as_deref(),
        args.write_baseline || args.update_baseline,
    )?;

    let mut rule_notes = Vec::new();
    let rule_sources = resolve_rules_sources(args.rules.as_deref());
    let rule_source_labels: Vec<String> = rule_sources
        .iter()
        .map(|(path, _)| path.display().to_string())
        .collect();
    let rule_origins: Vec<String> = rule_sources
        .iter()
        .map(|(_, origin)| (*origin).to_string())
        .collect();
    let rule_paths: Vec<PathBuf> = rule_sources.iter().map(|(path, _)| path.clone()).collect();
    let ruleset = if rule_paths.is_empty() {
        None
    } else {
        let (rules, notes) = load_rules(&rule_paths, design_spec)?;
        rule_notes = notes;
        Some(rules)
    };

    let session = launch_browser(&args)?;
    let browser = &session.browser;
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
                        browser,
                        &args,
                        Some(theme),
                        *viewport,
                        ruleset.as_ref(),
                        design_spec,
                        &design_notes,
                        &rule_notes,
                    )?,
                });
            }
        } else {
            let theme = args.theme.clone().filter(|value| value != "both");
            runs.push(Run {
                theme: theme.clone(),
                viewport: label,
                scrape: scrape_once(
                    browser,
                    &args,
                    theme.as_deref(),
                    *viewport,
                    ruleset.as_ref(),
                    design_spec,
                    &design_notes,
                    &rule_notes,
                )?,
            });
        }
    }

    let mut report = build_report(
        &args,
        &runs,
        both_themes,
        sweep,
        &rule_source_labels,
        &rule_origins,
        &design_source_labels,
        &design_origins,
    );
    if let Some(path) = write_design_if_needed(&args, runs[0].scrape.inferred.as_ref())? {
        report["designFile"] = json!(path);
    }
    apply_audit_baseline(&mut report, &args, baseline, &runs[0].scrape)?;
    drop(session);

    if args.pretty {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("{}", serde_json::to_string(&report)?);
    }

    if args.write_baseline || args.update_baseline {
        return Ok(0);
    }

    let has_error = report["findings"]
        .as_array()
        .map(|list| {
            list.iter()
                .any(|finding| match finding["severity"].as_str() {
                    Some("P0" | "P1" | "P2" | "error") => true,
                    _ => false,
                })
        })
        .unwrap_or(false);
    Ok(if has_error { 2 } else { 0 })
}

struct BrowserSession {
    browser: Browser,
    child: std::process::Child,
    profile: PathBuf,
}

impl Drop for BrowserSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.profile);
    }
}

fn find_chrome(args: &Args) -> Result<PathBuf> {
    if let Some(path) = &args.chrome {
        return Ok(path.clone());
    }
    for name in [
        "chromium",
        "chromium-browser",
        "google-chrome",
        "google-chrome-stable",
    ] {
        if let Ok(path) = which(name) {
            return Ok(path);
        }
    }
    anyhow::bail!("chromium not found; pass --chrome or set UI_PROBE_CHROME")
}

fn which(name: &str) -> Result<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    for directory in std::env::split_paths(&path) {
        let candidate = directory.join(name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    anyhow::bail!("{name} is not on PATH")
}

fn launch_browser(args: &Args) -> Result<BrowserSession> {
    let chrome = find_chrome(args)?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or(0);
    let profile = std::env::temp_dir().join(format!("ui-probe-{}-{stamp}", std::process::id()));
    std::fs::create_dir_all(&profile).context("create chromium profile directory")?;

    let mut command = Command::new(&chrome);
    command
        .arg("--headless=new")
        .arg("--remote-debugging-port=0")
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg("--remote-allow-origins=*")
        .arg("--disable-background-networking")
        .arg("--disable-dev-shm-usage")
        .arg("--disable-extensions")
        .arg("--disable-gpu")
        .arg("--hide-scrollbars")
        .arg("--force-color-profile=srgb")
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg(format!("--window-size={},{}", args.width, args.height))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());
    if args.no_sandbox {
        command.arg("--no-sandbox");
    }
    let mut child = command
        .spawn()
        .with_context(|| format!("launch chromium at {}", chrome.display()))?;
    let stderr = child.stderr.take().context("capture chromium stderr")?;
    let (sender, receiver) = std::sync::mpsc::channel::<Option<String>>();
    std::thread::spawn(move || {
        let reader = std::io::BufReader::new(stderr);
        for line in reader.lines() {
            match line {
                Ok(line) => {
                    if sender.send(Some(line)).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        let _ = sender.send(None);
    });

    let deadline = std::time::Instant::now() + args.launch_timeout;
    let mut output = Vec::new();
    let mut endpoint: Option<String> = None;
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            let _ = child.kill();
            let status = child.wait();
            let _ = std::fs::remove_dir_all(&profile);
            anyhow::bail!(
                "chromium did not report a DevTools endpoint within {}ms (status: {}):\n{}",
                args.launch_timeout.as_millis(),
                status
                    .map(|value| value.to_string())
                    .unwrap_or_else(|_| "unknown".to_string()),
                output.join("\n")
            );
        }
        match receiver.recv_timeout(remaining) {
            Ok(Some(line)) => {
                if let Some(value) = line.split("DevTools listening on ").nth(1) {
                    endpoint = Some(value.trim().to_string());
                }
                output.push(line);
                if output.len() > 200 {
                    output.remove(0);
                }
                if endpoint.is_some() {
                    break;
                }
            }
            Ok(None) => {
                let status = child.wait();
                let _ = std::fs::remove_dir_all(&profile);
                anyhow::bail!(
                    "chromium exited before reporting a DevTools endpoint (status: {}):\n{}",
                    status
                        .map(|value| value.to_string())
                        .unwrap_or_else(|_| "unknown".to_string()),
                    output.join("\n")
                );
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                let status = child.wait();
                let _ = std::fs::remove_dir_all(&profile);
                anyhow::bail!(
                    "chromium stderr closed before a DevTools endpoint (status: {}):\n{}",
                    status
                        .map(|value| value.to_string())
                        .unwrap_or_else(|_| "unknown".to_string()),
                    output.join("\n")
                );
            }
        }
    }

    let browser = match Browser::connect(endpoint.context("chromium gave no DevTools endpoint")?) {
        Ok(browser) => browser,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_dir_all(&profile);
            return Err(error).context("connect to chromium");
        }
    };
    Ok(BrowserSession {
        browser,
        child,
        profile,
    })
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
}

fn scrape_once(
    browser: &Browser,
    args: &Args,
    theme: Option<&str>,
    viewport: (u32, u32),
    ruleset: Option<&RuleSet>,
    design: Option<&DesignSpec>,
    design_notes: &[Value],
    rule_notes: &[Value],
) -> Result<Scrape> {
    let tab = browser.new_tab().context("open tab")?;
    tab.set_default_timeout(args.timeout);
    tab.call_method(Emulation::SetDeviceMetricsOverride {
        width: viewport.0,
        height: viewport.1,
        device_scale_factor: 1.0,
        mobile: viewport.0 < 600,
        scale: None,
        screen_width: Some(viewport.0),
        screen_height: Some(viewport.1),
        position_x: None,
        position_y: None,
        dont_set_visible_size: None,
        screen_orientation: None,
        viewport: None,
        display_feature: None,
        device_posture: None,
    })
    .context("set device metrics")?;

    if let Some(theme) = theme {
        tab.call_method(Emulation::SetEmulatedMedia {
            media: None,
            features: Some(vec![Emulation::MediaFeature {
                name: "prefers-color-scheme".to_string(),
                value: theme.to_string(),
            }]),
        })
        .context("set emulated media")?;
    }

    load(&tab, args)?;
    if let Some(rules) = ruleset {
        validate_selectors(&tab, rules)?;
    }
    let baseline = evaluate(&tab)?;
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
    let sheets = evaluate_js(&tab, STYLES_JS).unwrap_or_else(|error| {
        json!({
            "sheets": [],
            "unreadable": [],
            "error": format!("{error:#}")
        })
    });
    let sheet_text = sheets_text(&sheets);

    let mut findings = with_state(check(&baseline), &args.state);
    if let Some(rules) = ruleset {
        let oracle = load_oracle(&tab, rules)?;
        findings.extend(with_state(
            rule_findings(rules, &oracle, &baseline, design, &sheet_text),
            &args.state,
        ));
    }
    let baseline_keys: HashSet<String> = findings.iter().map(finding_key).collect();
    let mut expanded: Vec<Value> = Vec::new();
    let mut skipped: Vec<Value> = Vec::new();

    if args.expand {
        let discovered = evaluate_js(&tab, DISCOVER_JS)?;
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

            load(&tab, args)?;
            let fresh = evaluate_js(&tab, DISCOVER_JS)?;
            let available = fresh.as_array().map(|list| list.len()).unwrap_or(0);
            if index >= available {
                skipped.push(json!({ "name": name, "reason": "not-found" }));
                continue;
            }
            if !evaluate_js(&tab, &activate_js(index))?
                .as_bool()
                .unwrap_or(false)
            {
                skipped.push(json!({ "name": name, "reason": "not-clickable" }));
                continue;
            }

            let mut activated = false;
            for _ in 0..20 {
                if evaluate_js(&tab, &activated_js(index))?
                    .as_bool()
                    .unwrap_or(false)
                {
                    activated = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            if !activated {
                skipped.push(json!({
                    "name": if name.is_empty() { "unnamed".to_string() } else { name.clone() },
                    "reason": "postcondition-not-met",
                    "selector": candidate["selector"]
                }));
                continue;
            }
            std::thread::sleep(Duration::from_millis(250));

            let state_data = evaluate(&tab)?;
            let state_findings = check(&state_data);
            let generic_delta: Vec<Value> = state_findings
                .into_iter()
                .filter(|finding| !baseline_keys.contains(&finding_key(finding)))
                .collect();
            let mut state_rules: Vec<Value> = Vec::new();
            if let Some(rules) = ruleset {
                let oracle = load_oracle(&tab, rules)?;
                state_rules = rule_findings(rules, &oracle, &state_data, design, &sheet_text);
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

    let (motion, motion_findings) = motion_evidence(&baseline, &sheet_text, ruleset.is_none());
    extra.extend(motion_findings);
    let mut design_unverified: Vec<Value> = Vec::new();
    if let Some(spec) = design {
        let (design_findings, notes) = design_checks(&baseline, spec, &sheet_text);
        extra.extend(design_findings);
        design_unverified.extend(notes);
    }

    if ruleset.is_none() {
        extra.extend(typography_checks(&baseline));
        extra.extend(ghost_card_checks(&baseline));
        extra.extend(structure_checks(&baseline));
    }

    let focus_states = if args.sample_states {
        let sampled = evaluate_js(&tab, FOCUS_JS)?;
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

    findings.extend(with_state(extra, &args.state));

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
    unverified_entries.extend(design_unverified);
    unverified_entries.extend(colour_unverified(&baseline));

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
    })
}

fn validate_selectors(tab: &Tab, rules: &RuleSet) -> Result<()> {
    let errors = evaluate_js(tab, &validate_js(&rules.selectors))?;
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

fn load_oracle(tab: &Tab, rules: &RuleSet) -> Result<Oracle> {
    let data = evaluate_js(tab, &oracle_js(&rules.selectors, &rules.properties))?;
    let mut oracle = Oracle::default();
    for selector in &rules.selectors {
        let Some(entry) = data["selectors"].get(selector) else {
            continue;
        };
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
        if generic && finding["severity"].is_null() {
            finding["severity"] = json!("advisory");
        }
        finding["fingerprint"] = json!(finding_fingerprint(finding));
    }
}

fn build_report(
    args: &Args,
    runs: &[Run],
    both_themes: bool,
    sweep: bool,
    rule_sources: &[String],
    rule_origins: &[String],
    design_sources: &[String],
    design_origins: &[String],
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
    let unverified_list = dedupe_values(unverified_list);

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
            "rules": rule_sources,
            "rulesOrigin": rule_origins,
            "design": design_sources,
            "designOrigin": design_origins
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

fn visible_elements(data: &Value) -> Vec<Value> {
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
                        "evidence": { "property": property, "value": value }
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
        let property = Regex::new(r"([a-z-]+)\s*:").unwrap();
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

fn typography_checks(baseline: &Value) -> Vec<Value> {
    let mut findings = Vec::new();
    for element in visible_elements(baseline) {
        let selector = element["selector"].as_str().unwrap_or("?").to_string();
        let font_size = element["fontSize"].as_f64().unwrap_or(0.0);
        let has_text = element["hasText"].as_bool().unwrap_or(false);
        if !has_text {
            continue;
        }

        let text_length = element["textLength"].as_u64().unwrap_or(0);
        let ch_width = element["chWidth"].as_f64().unwrap_or(0.0);
        let padding = element["padding"].as_array().cloned().unwrap_or_default();
        let padding_x = padding.get(1).and_then(Value::as_f64).unwrap_or(0.0)
            + padding.get(3).and_then(Value::as_f64).unwrap_or(0.0);
        let width = element["rect"]["w"].as_f64().unwrap_or(0.0) - padding_x;
        if text_length >= 120 && ch_width > 0.0 && width > 0.0 {
            let measure = width / ch_width;
            if measure > 75.0 {
                findings.push(json!({
                    "check": "line-length",
                    "severity": "P3",
                    "selector": selector,
                    "message": "body text measure exceeds 75 characters",
                    "evidence": { "measure": (measure * 10.0).round() / 10.0, "characters": text_length }
                }));
            }
        }

        if font_size >= 16.0 {
            if let Some(raw) = element["letterSpacing"].as_str() {
                if raw != "normal" {
                    if let Some(px) = raw
                        .strip_suffix("px")
                        .and_then(|value| value.trim().parse::<f64>().ok())
                    {
                        let em = px / font_size;
                        if em < -0.0401 {
                            findings.push(json!({
                                "check": "extreme-negative-tracking",
                                "severity": "P3",
                                "selector": selector,
                                "message": "letter spacing is tighter than -0.04em",
                                "evidence": { "em": (em * 1000.0).round() / 1000.0, "letterSpacing": raw }
                            }));
                        }
                    }
                }
            }
        }
    }
    findings
}

fn ghost_card_checks(baseline: &Value) -> Vec<Value> {
    let mut findings = Vec::new();
    let length = Regex::new(r"(-?\d*\.?\d+)px").unwrap();
    for element in visible_elements(baseline) {
        let shadow = element["focusStyle"]["boxShadow"]
            .as_str()
            .unwrap_or("none");
        if shadow == "none" || shadow.contains("inset") {
            continue;
        }
        let blur = length
            .captures_iter(shadow)
            .filter_map(|capture| capture[1].parse::<f64>().ok())
            .nth(2);
        let Some(blur) = blur else {
            continue;
        };
        if blur < 8.0 {
            continue;
        }
        let widths = element["borderWidths"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let bordered = widths
            .iter()
            .filter_map(Value::as_f64)
            .any(|width| width > 0.0 && width <= 1.5);
        if bordered {
            findings.push(json!({
                "check": "gpt-thin-border-wide-shadow",
                "severity": "P3",
                "selector": element["selector"],
                "message": "hairline border plus a wide soft shadow leaves elevation ambiguous",
                "evidence": { "borderWidths": widths, "boxShadow": shadow, "blur": blur }
            }));
        }
    }
    findings
}

fn structure_checks(baseline: &Value) -> Vec<Value> {
    let mut findings = Vec::new();
    for element in visible_elements(baseline) {
        let selector = element["selector"].as_str().unwrap_or("?").to_string();
        if element["cardLike"].as_bool().unwrap_or(false)
            && element["cardDepth"].as_u64().unwrap_or(0) > 0
        {
            findings.push(json!({
                "check": "nested-cards",
                "severity": "P3",
                "selector": selector,
                "message": "a card sits inside another card",
                "evidence": { "cardDepth": element["cardDepth"] }
            }));
        }
        if element["eyebrow"].as_bool().unwrap_or(false) {
            findings.push(json!({
                "check": "kicker-above-heading",
                "severity": "P3",
                "selector": selector,
                "message": "a small label sits above a heading",
                "evidence": { "text": element["text"] }
            }));
        }
        if element["iconTile"].as_bool().unwrap_or(false) {
            findings.push(json!({
                "check": "icon-tile-stack",
                "severity": "P3",
                "selector": selector,
                "message": "an icon tile is stacked above a heading",
                "evidence": { "rect": element["rect"] }
            }));
        }
        if element["heroMetric"].as_bool().unwrap_or(false) {
            findings.push(json!({
                "check": "hero-metric",
                "severity": "P3",
                "selector": selector,
                "message": "a large standalone number is used as the visual anchor",
                "evidence": { "text": element["text"], "fontSize": element["fontSize"] }
            }));
        }
    }
    findings
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

struct DesignSpec {
    colors: Option<Vec<(i64, i64, i64)>>,
    spacing: Option<Vec<f64>>,
    radii: Option<Vec<String>>,
    fonts: Option<Vec<String>>,
    typography: Option<Vec<TypographyStep>>,
    surfaces: Option<Value>,
    motion: Option<Value>,
    params: Value,
}

struct TypographyStep {
    size: f64,
    weight: u64,
}

struct DesignToken {
    path: Vec<String>,
    value: Value,
    kind: Option<String>,
    extensions: Value,
}

fn has_token(value: &Value) -> bool {
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

fn is_dtcg(data: &Value) -> bool {
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

fn collect_tokens(
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

fn group_tokens(data: &Value, group: &str) -> Vec<DesignToken> {
    let Some(value) = data.get(group) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut path = vec![group.to_string()];
    let no_extensions = Value::Null;
    collect_tokens(value, &mut path, None, &no_extensions, &mut out);
    out
}

fn clean_family(raw: &str) -> String {
    raw.trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim()
        .to_lowercase()
}

fn generic_family(family: &str) -> bool {
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

fn dimension_px(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(raw) => raw
            .trim()
            .strip_suffix("px")
            .and_then(|number| number.trim().parse::<f64>().ok()),
        _ => None,
    }
}

fn dimension_string(value: &Value) -> Option<String> {
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

fn merge_json(base: &mut Value, overlay: &Value) {
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

fn load_design_sources(sources: &[(PathBuf, &'static str)]) -> Result<(DesignSpec, Vec<Value>)> {
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

fn parse_design(data: &Value) -> (DesignSpec, Vec<Value>) {
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

fn load_dtcg_design(data: &Value, notes: &mut Vec<Value>) -> DesignSpec {
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

fn load_flat_design(data: &Value, notes: &mut Vec<Value>) -> DesignSpec {
    let colors = data["colors"].as_array().map(|list| {
        list.iter()
            .filter_map(|value| {
                let raw = value.as_str()?;
                match parse_color_rgb(raw) {
                    Some(color) => Some(color),
                    None => {
                        notes.push(json!({
                            "check": "design",
                            "reason": "unsupported colour in design file",
                            "value": raw
                        }));
                        None
                    }
                }
            })
            .collect::<Vec<_>>()
    });
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

fn parse_dimension(raw: &str) -> Option<(f64, String)> {
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

fn radius_allowed(actual: &str, allowed: &[String]) -> bool {
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

fn font_families(stack: &str) -> Vec<String> {
    split_top_level(stack)
        .iter()
        .map(|value| clean_family(value))
        .filter(|value| !value.is_empty())
        .collect()
}

fn sheet_contains_color(text: &str, expected: &str) -> bool {
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

fn opaque_sampled(raw: &str) -> bool {
    parse_css_color(raw)
        .map(|(_, _, _, alpha)| alpha > 0.0)
        .unwrap_or(false)
}

fn design_checks(
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
                    "property": property,
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

fn parse_color_rgb(css: &str) -> Option<(i64, i64, i64)> {
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
        let channel = |value: &str| i64::from_str_radix(value, 16).ok().map(|number| number);
        return Some((
            channel(&pairs[0])?,
            channel(&pairs[1])?,
            channel(&pairs[2])?,
        ));
    }
    parse_css_color(&css).map(|(r, g, b, _)| (r.round() as i64, g.round() as i64, b.round() as i64))
}

fn primary_font(stack: &str) -> String {
    font_families(stack).into_iter().next().unwrap_or_default()
}

fn confidence(count: u64, total: u64) -> f64 {
    if total == 0 {
        return 0.0;
    }
    ((count as f64 / total as f64) * 1000.0).round() / 1000.0
}

fn infer_design(baseline: &Value) -> (Value, Vec<Value>) {
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
        off.sort_by(|left, right| right.1.cmp(&left.1));
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
    palette_entries.sort_by(|left, right| right["count"].as_u64().cmp(&left["count"].as_u64()));

    let mut spacing_entries: Vec<Value> = spacing
        .iter()
        .map(|(value, count)| json!({ "value": value, "count": count, "confidence": confidence(*count, spacing_total) }))
        .collect();
    spacing_entries.sort_by(|left, right| right["count"].as_u64().cmp(&left["count"].as_u64()));

    let mut radius_entries: Vec<Value> = radii
        .iter()
        .map(|(value, count)| json!({ "value": value, "count": count, "confidence": confidence(*count, radii.values().sum::<u64>()) }))
        .collect();
    radius_entries.sort_by(|left, right| right["count"].as_u64().cmp(&left["count"].as_u64()));

    let mut font_entries: Vec<Value> = fonts
        .iter()
        .map(|(family, count)| json!({ "family": family, "count": count, "confidence": confidence(*count, font_total) }))
        .collect();
    font_entries.sort_by(|left, right| right["count"].as_u64().cmp(&left["count"].as_u64()));

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
    typography_entries.sort_by(|left, right| right["count"].as_u64().cmp(&left["count"].as_u64()));

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

fn color_distance_f(color: &(i64, i64, i64), cluster: &(f64, f64, f64, u64)) -> f64 {
    let dr = color.0 as f64 - cluster.0;
    let dg = color.1 as f64 - cluster.1;
    let db = color.2 as f64 - cluster.2;
    (dr * dr + dg * dg + db * db).sqrt()
}

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
            "next" => lines
                .entry(index + 2)
                .or_insert_with(HashSet::new)
                .extend(ids),
            _ => lines
                .entry(index + 1)
                .or_insert_with(HashSet::new)
                .extend(ids),
        }
    }
    (lines, file)
}

fn source_mode(root: &Path, args: &Args) -> Result<i32> {
    if !root.is_dir() {
        anyhow::bail!("source directory does not exist: {}", root.display());
    }
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
    let baseline_path = resolve_baseline_path(args);
    let baseline = load_baseline(
        baseline_path.as_deref(),
        args.write_baseline || args.update_baseline,
    )?;

    let mut findings = Vec::new();
    let mut suppressed = 0usize;
    let mut scanned = 0usize;
    for path in source_files(root, args.source_all)
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

    if args.pretty {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("{}", serde_json::to_string(&report)?);
    }

    if args.write_baseline || args.update_baseline {
        return Ok(0);
    }

    let has_error = report["findings"]
        .as_array()
        .map(|list| {
            list.iter()
                .any(|finding| match finding["severity"].as_str() {
                    Some("P0" | "P1" | "P2" | "error") => true,
                    _ => false,
                })
        })
        .unwrap_or(false);
    Ok(if has_error { 2 } else { 0 })
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

struct RuleSet {
    structural: Vec<StructuralRule>,
    named: Vec<NamedRule>,
    selectors: Vec<String>,
    properties: Vec<String>,
}

struct StructuralRule {
    name: String,
    source: String,
    selector: String,
    ancestors: Option<Constraint>,
    descendants: Option<Constraint>,
    properties: Vec<(String, PropertyRule)>,
    severity: String,
    layer: String,
}

struct Constraint {
    any: bool,
    selector: String,
}

enum PropertyRule {
    Deny(Vec<String>),
    AllowFrom(String),
}

struct NamedRule {
    id: String,
    source: String,
    severity: String,
    layer: String,
    params: Value,
}

const NAMED_RULES: &[&str] = &[
    "nested-cards",
    "kicker-above-heading",
    "line-length",
    "extreme-negative-tracking",
    "gpt-thin-border-wide-shadow",
    "side-tab",
    "em-dash-overuse",
    "marketing-buzzword",
    "bounce-easing",
];

const CSS_PROPERTIES: &[&str] = &[
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

fn check_keys(value: &Value, allowed: &[&str], context: &str) -> Result<()> {
    if let Some(object) = value.as_object() {
        for key in object.keys() {
            if !allowed.contains(&key.as_str()) {
                anyhow::bail!("rules: unknown key {key} in {context}");
            }
        }
    }
    Ok(())
}

fn parse_constraint(value: &Value, context: &str) -> Result<Option<Constraint>> {
    if value.is_null() {
        return Ok(None);
    }
    check_keys(value, &["any", "none"], context)?;
    let any = value.get("any");
    let none = value.get("none");
    let (matches, selector) = match (any, none) {
        (Some(selector), None) => (true, selector),
        (None, Some(selector)) => (false, selector),
        _ => anyhow::bail!("rules: {context} needs exactly one of any or none"),
    };
    let selector = selector
        .as_str()
        .context(format!("rules: {context} needs a selector"))?
        .trim()
        .to_string();
    if selector.is_empty() {
        anyhow::bail!("rules: {context} selector is empty");
    }
    Ok(Some(Constraint {
        any: matches,
        selector,
    }))
}

fn resolve_param(design: Option<&DesignSpec>, path: &str) -> Option<Value> {
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

fn dangling_param(value: &Value) -> Option<String> {
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

fn resolve_params(
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

fn read_rules(path: &Path) -> Result<Value> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("read rules file {}", path.display()))?;
    let data: Value = serde_json::from_str(&text)
        .with_context(|| format!("parse rules file {}", path.display()))?;
    check_keys(
        &data,
        &["version", "pack", "structural", "rules"],
        "rules file",
    )?;
    if data["version"].as_u64() != Some(1) {
        anyhow::bail!("rules: version must be 1");
    }
    Ok(data)
}

fn merge_rules(sources: &[PathBuf]) -> Result<Option<Value>> {
    if sources.is_empty() {
        return Ok(None);
    }
    let mut pack = None;
    let mut structural = Vec::new();
    let mut named = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();
    for source in sources {
        let data = read_rules(source)?;
        let label = source.display().to_string();
        if pack.is_none() {
            pack = data["pack"].as_str().map(str::to_string);
        }
        for mut rule in data["structural"].as_array().cloned().unwrap_or_default() {
            if let Some(object) = rule.as_object_mut() {
                object.insert("_source".to_string(), json!(label));
            }
            structural.push(rule);
        }
        for mut rule in data["rules"].as_array().cloned().unwrap_or_default() {
            let duplicate = rule["id"]
                .as_str()
                .map(|id| !seen_ids.insert(id.to_string()))
                .unwrap_or(false);
            if duplicate {
                continue;
            }
            if let Some(object) = rule.as_object_mut() {
                object.insert("_source".to_string(), json!(label));
            }
            named.push(rule);
        }
    }
    Ok(Some(json!({
        "version": 1,
        "pack": pack.unwrap_or_else(|| "custom".to_string()),
        "structural": structural,
        "rules": named
    })))
}

fn load_rules(sources: &[PathBuf], design: Option<&DesignSpec>) -> Result<(RuleSet, Vec<Value>)> {
    let Some(data) = merge_rules(sources)? else {
        anyhow::bail!("rules: no sources");
    };
    parse_rules(&data, design)
}

fn parse_rules(data: &Value, design: Option<&DesignSpec>) -> Result<(RuleSet, Vec<Value>)> {
    let mut notes = Vec::new();
    let mut selectors: Vec<String> = Vec::new();
    let mut properties: Vec<String> = Vec::new();
    let mut structural = Vec::new();
    for rule in data["structural"].as_array().cloned().unwrap_or_default() {
        check_keys(
            &rule,
            &[
                "name",
                "selector",
                "ancestors",
                "descendants",
                "properties",
                "severity",
                "layer",
                "_source",
            ],
            "structural rule",
        )?;
        let name = rule["name"]
            .as_str()
            .context("structural rule needs a name")?
            .to_string();
        let rule_source = rule["_source"].as_str().unwrap_or("").to_string();
        let selector = rule["selector"]
            .as_str()
            .context("structural rule needs a selector")?
            .trim()
            .to_string();
        if selector.is_empty() {
            anyhow::bail!("rules: structural rule {name} has an empty selector");
        }
        let severity = rule["severity"].as_str().unwrap_or("advisory").to_string();
        let layer = rule["layer"].as_str().unwrap_or("layout").to_string();
        validate_severity_layer(&severity, &layer)?;
        let ancestors = parse_constraint(&rule["ancestors"], "ancestors")?;
        let descendants = parse_constraint(&rule["descendants"], "descendants")?;
        if !selectors.contains(&selector) {
            selectors.push(selector.clone());
        }
        if let Some(constraint) = &ancestors {
            if !selectors.contains(&constraint.selector) {
                selectors.push(constraint.selector.clone());
            }
        }
        if let Some(constraint) = &descendants {
            if !selectors.contains(&constraint.selector) {
                selectors.push(constraint.selector.clone());
            }
        }

        let mut rule_properties = Vec::new();
        for (property, constraint) in rule["properties"].as_object().cloned().unwrap_or_default() {
            if !CSS_PROPERTIES.contains(&property.as_str()) {
                anyhow::bail!("rules: unknown CSS property {property} in rule {name}");
            }
            check_keys(&constraint, &["deny", "allowFrom"], "property constraint")?;
            if constraint.get("deny").is_some() == constraint.get("allowFrom").is_some() {
                anyhow::bail!("rules: property {property} needs exactly one of deny or allowFrom");
            }
            if !properties.contains(&property) {
                properties.push(property.clone());
            }
            if let Some(values) = constraint["deny"].as_array() {
                for value in values {
                    if let Some(reference) = dangling_param(value) {
                        anyhow::bail!("rules: dangling param reference {reference} in rule {name}");
                    }
                }
                let deny: Vec<String> = values
                    .iter()
                    .map(|value| resolve_params(value.clone(), design, &name, &mut notes))
                    .filter_map(|value| match value {
                        Value::String(raw) => Some(raw.to_lowercase()),
                        Value::Number(number) => Some(number.to_string()),
                        _ => None,
                    })
                    .collect();
                if deny.is_empty() {
                    anyhow::bail!("rules: property {property} deny list is empty");
                }
                rule_properties.push((property.clone(), PropertyRule::Deny(deny)));
            } else if let Some(source) = constraint["allowFrom"].as_str() {
                if !["colors", "fonts", "spacing", "radii", "typography"].contains(&source) {
                    anyhow::bail!("rules: unknown allowFrom source {source}");
                }
                if design.is_none() {
                    notes.push(json!({
                        "check": name,
                        "reason": "missing design parameters",
                        "rule": name,
                        "source": source
                    }));
                    continue;
                }
                rule_properties.push((
                    property.clone(),
                    PropertyRule::AllowFrom(source.to_string()),
                ));
            } else {
                anyhow::bail!("rules: property {property} needs deny or allowFrom");
            }
        }

        structural.push(StructuralRule {
            name,
            source: rule_source,
            selector,
            ancestors,
            descendants,
            properties: rule_properties,
            severity,
            layer,
        });
    }

    let mut named = Vec::new();
    for rule in data["rules"].as_array().cloned().unwrap_or_default() {
        check_keys(
            &rule,
            &["id", "enabled", "severity", "layer", "params", "_source"],
            "named rule",
        )?;
        let id = rule["id"]
            .as_str()
            .context("named rule needs an id")?
            .to_string();
        let rule_source = rule["_source"].as_str().unwrap_or("").to_string();
        if !NAMED_RULES.contains(&id.as_str()) {
            anyhow::bail!("rules: unknown rule id {id}");
        }
        if !rule["enabled"].as_bool().unwrap_or(true) {
            continue;
        }
        let severity = rule["severity"].as_str().unwrap_or("advisory").to_string();
        let layer = rule["layer"].as_str().unwrap_or("layout").to_string();
        validate_severity_layer(&severity, &layer)?;
        if let Some(reference) = dangling_param(&rule["params"]) {
            anyhow::bail!("rules: dangling param reference {reference} in rule {id}");
        }
        let params = resolve_params(rule["params"].clone(), design, &id, &mut notes);
        named.push(NamedRule {
            id,
            source: rule_source,
            severity,
            layer,
            params,
        });
    }

    Ok((
        RuleSet {
            structural,
            named,
            selectors,
            properties,
        },
        notes,
    ))
}

fn validate_severity_layer(severity: &str, layer: &str) -> Result<()> {
    if !["error", "advisory"].contains(&severity) {
        anyhow::bail!("rules: severity must be error or advisory");
    }
    if !["layout", "sheet", "content", "source"].contains(&layer) {
        anyhow::bail!("rules: unknown layer {layer}");
    }
    Ok(())
}

fn param_f64(params: &Value, key: &str, fallback: f64) -> f64 {
    match params.get(key) {
        Some(Value::Number(number)) => number.as_f64().unwrap_or(fallback),
        Some(Value::String(raw)) => raw
            .trim()
            .strip_suffix("px")
            .unwrap_or(raw)
            .trim()
            .parse::<f64>()
            .unwrap_or(fallback),
        _ => fallback,
    }
}

fn param_u64(params: &Value, key: &str, fallback: u64) -> u64 {
    param_f64(params, key, fallback as f64).round().max(0.0) as u64
}

fn font_stack_allowed(stack: &str, allowed: &[String]) -> bool {
    font_families(stack)
        .iter()
        .all(|family| generic_family(family) || allowed.iter().any(|entry| entry == family))
}

fn property_allowed(source: &str, actual: &str, design: &DesignSpec) -> bool {
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

fn parse_color_alpha(css: &str) -> Option<(f64, f64, f64, f64)> {
    if let Some(values) = parse_css_color(css) {
        return Some(values);
    }
    let value = css.trim().to_lowercase();
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

fn color_matches(actual: &str, expected: &str) -> bool {
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

fn deny_value_matches(property: &str, actual: &str, expected: &str) -> bool {
    if property == "color" || property.ends_with("-color") {
        color_matches(actual, expected)
    } else {
        actual.eq_ignore_ascii_case(expected)
    }
}

fn ancestor_any(elements: &[Value], index: usize, matches: &HashSet<usize>) -> bool {
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

fn descendant_any(elements: &[Value], index: usize, matches: &HashSet<usize>) -> bool {
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

fn element_ignored(element: &Value, rule: &str) -> bool {
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

fn rule_findings(
    rules: &RuleSet,
    oracle: &Oracle,
    baseline: &Value,
    design: Option<&DesignSpec>,
    sheet_text: &str,
) -> Vec<Value> {
    let elements = baseline["elements"].as_array().cloned().unwrap_or_default();
    let mut findings = Vec::new();

    for rule in &rules.structural {
        let matches: HashSet<usize> = oracle.matches(&rule.selector).iter().copied().collect();
        let ancestor_matches: HashSet<usize> = rule
            .ancestors
            .as_ref()
            .map(|constraint| {
                oracle
                    .matches(&constraint.selector)
                    .iter()
                    .copied()
                    .collect()
            })
            .unwrap_or_default();
        let descendant_matches: HashSet<usize> = rule
            .descendants
            .as_ref()
            .map(|constraint| {
                oracle
                    .matches(&constraint.selector)
                    .iter()
                    .copied()
                    .collect()
            })
            .unwrap_or_default();
        for &index in &matches {
            let Some(element) = elements.get(index) else {
                continue;
            };
            if !element["visible"].as_bool().unwrap_or(false) {
                continue;
            }
            if element_ignored(element, &rule.name) {
                continue;
            }
            let mut violated: Option<(String, String, String)> = None;
            if let Some(constraint) = &rule.ancestors {
                let found = ancestor_any(&elements, index, &ancestor_matches);
                if found != constraint.any {
                    violated = Some((
                        "ancestors".to_string(),
                        constraint.selector.clone(),
                        String::new(),
                    ));
                }
            }
            if violated.is_none() {
                if let Some(constraint) = &rule.descendants {
                    let found = descendant_any(&elements, index, &descendant_matches);
                    if found != constraint.any {
                        violated = Some((
                            "descendants".to_string(),
                            constraint.selector.clone(),
                            String::new(),
                        ));
                    }
                }
            }
            if violated.is_none() {
                for (property, constraint) in &rule.properties {
                    let actual = oracle
                        .value(&rule.selector, property, index)
                        .unwrap_or("")
                        .trim();
                    if actual.is_empty() {
                        continue;
                    }
                    match constraint {
                        PropertyRule::Deny(deny) => {
                            if deny
                                .iter()
                                .any(|entry| deny_value_matches(property, actual, entry))
                            {
                                violated =
                                    Some((property.clone(), deny.join(","), actual.to_string()));
                                break;
                            }
                        }
                        PropertyRule::AllowFrom(source) => {
                            if let Some(design) = design {
                                if !property_allowed(source, actual, design) {
                                    violated = Some((
                                        property.clone(),
                                        "parameter set".to_string(),
                                        actual.to_string(),
                                    ));
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            if let Some((property, expected, actual)) = violated {
                findings.push(json!({
                    "check": rule.name,
                    "ruleSource": rule.source,
                    "severity": rule.severity,
                    "layer": rule.layer,
                    "selector": element["selector"],
                    "property": property,
                    "message": format!("{} violated", rule.name),
                    "evidence": { "expected": expected, "actual": actual }
                }));
            }
        }
    }

    for rule in &rules.named {
        let mut push = |selector: &str, property: &str, message: &str, evidence: Value| {
            findings.push(json!({
                "check": rule.id,
                "ruleSource": rule.source,
                "severity": rule.severity,
                "layer": rule.layer,
                "selector": selector,
                "property": property,
                "message": message,
                "evidence": evidence
            }));
        };
        match rule.id.as_str() {
            "nested-cards" => {
                for element in elements
                    .iter()
                    .filter(|element| !element_ignored(element, &rule.id))
                {
                    if !element["visible"].as_bool().unwrap_or(false) {
                        continue;
                    }
                    if element["cardLike"].as_bool().unwrap_or(false)
                        && element["cardDepth"].as_u64().unwrap_or(0) > 0
                    {
                        push(
                            element["selector"].as_str().unwrap_or("?"),
                            "card",
                            "card inside card",
                            json!({"cardDepth": element["cardDepth"]}),
                        );
                    }
                }
            }
            "kicker-above-heading" => {
                let max = param_u64(&rule.params, "maxTextLength", 40) as usize;
                let require = rule.params["requireUppercaseOrTracking"]
                    .as_bool()
                    .unwrap_or(true);
                for element in elements
                    .iter()
                    .filter(|element| !element_ignored(element, &rule.id))
                {
                    if !element["visible"].as_bool().unwrap_or(false)
                        || !element["hasText"].as_bool().unwrap_or(false)
                    {
                        continue;
                    }
                    let size = element["fontSize"].as_f64().unwrap_or(0.0);
                    let next_size = element["nextSize"].as_f64().unwrap_or(0.0);
                    let tracking = element["letterSpacing"]
                        .as_str()
                        .and_then(|value| value.strip_suffix("px"))
                        .and_then(|value| value.trim().parse::<f64>().ok())
                        .unwrap_or(0.0);
                    let styled = element["textTransform"].as_str() == Some("uppercase")
                        || tracking > size * 0.04;
                    if element["nextIsHeading"].as_bool().unwrap_or(false)
                        && element["textLength"].as_u64().unwrap_or(0) as usize <= max
                        && size < next_size
                        && (!require || styled)
                    {
                        push(
                            element["selector"].as_str().unwrap_or("?"),
                            "text",
                            "kicker above heading",
                            json!({"text": element["text"]}),
                        );
                    }
                }
            }
            "line-length" => {
                let max = param_f64(&rule.params, "maxCh", 75.0);
                for element in elements
                    .iter()
                    .filter(|element| !element_ignored(element, &rule.id))
                {
                    if !element["visible"].as_bool().unwrap_or(false)
                        || !element["hasText"].as_bool().unwrap_or(false)
                        || element["textLength"].as_u64().unwrap_or(0) < 120
                    {
                        continue;
                    }
                    let ch = element["chWidth"].as_f64().unwrap_or(0.0);
                    let padding = element["padding"].as_array().cloned().unwrap_or_default();
                    let width = element["rect"]["w"].as_f64().unwrap_or(0.0)
                        - padding.get(1).and_then(Value::as_f64).unwrap_or(0.0)
                        - padding.get(3).and_then(Value::as_f64).unwrap_or(0.0);
                    if ch > 0.0 && width / ch > max {
                        push(
                            element["selector"].as_str().unwrap_or("?"),
                            "measure",
                            "line length above the pack limit",
                            json!({"measure": width / ch, "max": max}),
                        );
                    }
                }
            }
            "extreme-negative-tracking" => {
                let min = param_f64(&rule.params, "minEm", -0.04);
                for element in elements
                    .iter()
                    .filter(|element| !element_ignored(element, &rule.id))
                {
                    if !element["visible"].as_bool().unwrap_or(false) {
                        continue;
                    }
                    let size = element["fontSize"].as_f64().unwrap_or(0.0);
                    if size < 16.0 {
                        continue;
                    }
                    if let Some(px) = element["letterSpacing"]
                        .as_str()
                        .and_then(|value| value.strip_suffix("px"))
                        .and_then(|value| value.trim().parse::<f64>().ok())
                    {
                        if px / size < min {
                            push(
                                element["selector"].as_str().unwrap_or("?"),
                                "letter-spacing",
                                "tracking below the pack floor",
                                json!({"em": px / size, "min": min}),
                            );
                        }
                    }
                }
            }
            "gpt-thin-border-wide-shadow" => {
                let min_blur = param_f64(&rule.params, "blurPx", 8.0);
                let length = Regex::new(r"(-?\d*\.?\d+)px").unwrap();
                for element in elements
                    .iter()
                    .filter(|element| !element_ignored(element, &rule.id))
                {
                    if !element["visible"].as_bool().unwrap_or(false) {
                        continue;
                    }
                    let shadow = element["focusStyle"]["boxShadow"]
                        .as_str()
                        .unwrap_or("none");
                    if shadow == "none" || shadow.contains("inset") {
                        continue;
                    }
                    let blur = length
                        .captures_iter(shadow)
                        .filter_map(|c| c[1].parse::<f64>().ok())
                        .nth(2);
                    let bordered = element["borderWidths"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default()
                        .iter()
                        .filter_map(Value::as_f64)
                        .any(|width| width > 0.0 && width <= 1.5);
                    if bordered && blur.map(|blur| blur >= min_blur).unwrap_or(false) {
                        push(
                            element["selector"].as_str().unwrap_or("?"),
                            "box-shadow",
                            "hairline border with wide shadow",
                            json!({"blur": blur}),
                        );
                    }
                }
            }
            "side-tab" => {
                for element in elements
                    .iter()
                    .filter(|element| !element_ignored(element, &rule.id))
                {
                    if !element["visible"].as_bool().unwrap_or(false) {
                        continue;
                    }
                    let widths: Vec<f64> = element["borderWidths"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default()
                        .iter()
                        .filter_map(Value::as_f64)
                        .collect();
                    if widths.len() != 4 {
                        continue;
                    }
                    let mut sides = widths.clone();
                    sides.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
                    if sides[0] >= 2.0 && sides[0] >= sides[1] * 2.0 {
                        let radius = element["radiusPx"]
                            .as_array()
                            .cloned()
                            .unwrap_or_default()
                            .iter()
                            .filter_map(Value::as_f64)
                            .fold(0.0, f64::max);
                        if radius > 0.0 {
                            push(
                                element["selector"].as_str().unwrap_or("?"),
                                "border",
                                "side tab accent",
                                json!({"widths": widths}),
                            );
                        }
                    }
                }
            }
            "em-dash-overuse" => {
                let max = param_u64(&rule.params, "max", 8) as usize;
                let count = baseline["pageText"]
                    .as_str()
                    .unwrap_or("")
                    .matches('—')
                    .count();
                if count >= max {
                    push(
                        ":root",
                        "text",
                        "em dash overuse",
                        json!({"count": count, "max": max}),
                    );
                }
            }
            "marketing-buzzword" => {
                let text = baseline["pageText"].as_str().unwrap_or("").to_lowercase();
                for word in rule.params["words"].as_array().cloned().unwrap_or_default() {
                    if let Some(word) = word.as_str() {
                        if text.contains(&word.to_lowercase()) {
                            push(":root", "text", "marketing buzzword", json!({"word": word}));
                        }
                    }
                }
            }
            "bounce-easing" => {
                for element in elements
                    .iter()
                    .filter(|element| !element_ignored(element, &rule.id))
                {
                    if !element["visible"].as_bool().unwrap_or(false) {
                        continue;
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
                            if is_overshooting(&value) {
                                push(
                                    element["selector"].as_str().unwrap_or("?"),
                                    "easing",
                                    "overshooting easing curve",
                                    json!({"property": property, "value": value}),
                                );
                            }
                        }
                    }
                }
                let bounce = Regex::new(r"(?i)\b(bounce|elastic)\b").unwrap();
                if bounce.is_match(sheet_text) {
                    push(
                        ":root",
                        "easing",
                        "stylesheet declares a bounce or elastic timing keyword",
                        json!({"sheet": true}),
                    );
                }
                let curve = Regex::new(r"cubic-bezier\(([^)]*)\)").unwrap();
                for captures in curve.captures_iter(sheet_text) {
                    let value = format!("cubic-bezier({})", &captures[1]);
                    if is_overshooting(&value) {
                        push(
                            ":root",
                            "easing",
                            "stylesheet declares an overshooting cubic-bezier",
                            json!({"value": value, "sheet": true}),
                        );
                    }
                }
            }
            _ => {}
        }
    }
    findings
}

fn load(tab: &Tab, args: &Args) -> Result<()> {
    let url = args.url.as_deref().context("--url is required")?;
    tab.navigate_to(url)
        .with_context(|| format!("navigate to {url}"))?;
    tab.wait_until_navigated().context("wait for navigation")?;
    if let Some(selector) = &args.wait_for {
        tab.wait_for_element_with_custom_timeout(selector, args.timeout)
            .with_context(|| format!("wait for {selector}"))?;
    }
    std::thread::sleep(Duration::from_millis(400));
    Ok(())
}

fn evaluate(tab: &Tab) -> Result<Value> {
    evaluate_js(tab, JS)
}

fn evaluate_js(tab: &Tab, js: &str) -> Result<Value> {
    let remote = tab.evaluate(js, true).context("evaluate page")?;
    let raw = remote.value.context("page returned no value")?;
    match raw {
        Value::String(text) => serde_json::from_str(&text).context("parse page payload"),
        other => Ok(other),
    }
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
    out
}

fn check(data: &Value) -> Vec<Value> {
    let mut findings: Vec<Value> = Vec::new();

    if let Some(viewport) = data["viewport"].as_object() {
        let width = viewport.get("width").and_then(Value::as_f64).unwrap_or(0.0);
        let document_width = viewport
            .get("documentWidth")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        if document_width > width + 1.0 {
            findings.push(json!({
                "check": "page-overflow",
                "severity": "P1",
                "selector": "html",
                "message": format!("document is {document_width:.0}px wide in a {width:.0}px viewport"),
                "evidence": { "documentWidth": document_width, "viewportWidth": width }
            }));
        }
    }

    if data["lang"].is_null() {
        findings.push(json!({
            "check": "missing-lang",
            "severity": "P2",
            "selector": "html",
            "message": "html element has no lang attribute",
            "evidence": {}
        }));
    }

    let mut previous_heading = 0u32;
    for heading in data["headings"].as_array().cloned().unwrap_or_default() {
        let level = heading
            .as_str()
            .and_then(|h| h.strip_prefix('h'))
            .and_then(|n| n.parse::<u32>().ok())
            .unwrap_or(0);
        if level == 0 {
            continue;
        }
        if previous_heading > 0 && level > previous_heading + 1 {
            findings.push(json!({
                "check": "heading-skip",
                "severity": "P3",
                "selector": format!("h{level}"),
                "message": format!("heading level jumps from h{previous_heading} to h{level}"),
                "evidence": { "from": previous_heading, "to": level }
            }));
        }
        previous_heading = level;
    }

    for focusable in data["focusables"].as_array().cloned().unwrap_or_default() {
        if let Some(hidden) = focusable["hiddenBy"].as_str() {
            findings.push(json!({
                "check": "focus-hidden",
                "severity": "P2",
                "selector": focusable["selector"],
                "message": format!("focusable element is hidden by {hidden}"),
                "evidence": {
                    "tabIndex": focusable["tabIndex"],
                    "name": focusable["name"]
                }
            }));
        }
    }

    for element in visible_elements(data) {
        let selector = element["selector"].as_str().unwrap_or("?");
        let tag = element["tag"].as_str().unwrap_or("?");
        let interactive = element["interactive"].as_bool().unwrap_or(false);
        let disabled = element["disabled"].as_bool().unwrap_or(false);
        let has_text = element["hasText"].as_bool().unwrap_or(false);
        let name = element["name"].as_str().unwrap_or("").trim().to_string();
        let rect_w = element["rect"]["w"].as_f64().unwrap_or(0.0);
        let rect_h = element["rect"]["h"].as_f64().unwrap_or(0.0);

        if interactive && !disabled && name.is_empty() {
            findings.push(json!({
                "check": "missing-name",
                "severity": "P1",
                "selector": selector,
                "message": format!("{tag} has no accessible name"),
                "evidence": { "tag": tag }
            }));
        }

        if interactive && !disabled && (rect_w < 44.0 || rect_h < 44.0) {
            findings.push(json!({
                "check": "target-size",
                "severity": "P2",
                "selector": selector,
                "message": format!("target is {rect_w:.0}x{rect_h:.0}px, below 44x44"),
                "evidence": { "width": rect_w, "height": rect_h }
            }));
        }

        if let Some(raw) = element["tabindex"].as_str() {
            if raw.parse::<i32>().map(|n| n > 0).unwrap_or(false) {
                findings.push(json!({
                    "check": "positive-tabindex",
                    "severity": "P3",
                    "selector": selector,
                    "message": format!("tabindex={raw} overrides document order"),
                    "evidence": { "tabindex": raw }
                }));
            }
        }

        if has_text {
            if let Some(ratio) = contrast_ratio(&element) {
                let font_size = element["fontSize"].as_f64().unwrap_or(16.0);
                let font_weight = element["fontWeight"].as_u64().unwrap_or(400);
                let large = font_size >= 24.0 || (font_size >= 18.66 && font_weight >= 700);
                let threshold = if large { 3.0 } else { 4.5 };
                if ratio < threshold {
                    findings.push(json!({
                        "check": "contrast",
                        "severity": "P2",
                        "selector": selector,
                        "message": format!("text contrast {ratio:.2}:1 is below {threshold:.1}:1"),
                        "evidence": {
                            "ratio": (ratio * 100.0).round() / 100.0,
                            "threshold": threshold,
                            "color": element["color"],
                            "fontSize": font_size,
                            "fontWeight": font_weight,
                            "text": element["text"]
                        }
                    }));
                }
            }
        }

        let overflow_x = element["overflowX"].as_i64().unwrap_or(0);
        let overflow_y = element["overflowY"].as_i64().unwrap_or(0);
        let overflow = element["overflow"].as_str().unwrap_or("");
        let clips = overflow.contains("hidden") || overflow.contains("clip");
        if clips && has_text && (overflow_x > 1 || overflow_y > 1) {
            findings.push(json!({
                "check": "clipped-text",
                "severity": "P2",
                "selector": selector,
                "message": format!("text is clipped by {overflow}"),
                "evidence": {
                    "overflowX": overflow_x,
                    "overflowY": overflow_y,
                    "textOverflow": element["textOverflow"],
                    "whiteSpace": element["whiteSpace"]
                }
            }));
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
    findings
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

fn parse_css_color(css: &str) -> Option<(f64, f64, f64, f64)> {
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

fn luminance(r: f64, g: f64, b: f64) -> f64 {
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
