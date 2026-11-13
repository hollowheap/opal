use crate::Args;
use anyhow::{Context, Result};
use playwright_rs::{Browser, ConnectOverCdpOptions, Page, Playwright};
use serde_json::Value;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

pub(crate) const JS: &str = r##"
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
  canvas.width = 1;
  canvas.height = 1;
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
      if (match) {
        const parts = match[1].split(/[ ,\/]+/).filter(Boolean).map(Number);
        if (parts.length < 3 || !parts.slice(0, 3).every((channel) => Number.isFinite(channel))) return null;
        r = parts[0];
        g = parts[1];
        b = parts[2];
        if (parts.length > 3 && Number.isFinite(parts[3])) a = clamp(parts[3], 0, 1);
      } else {
        context.clearRect(0, 0, 1, 1);
        context.fillStyle = value;
        context.fillRect(0, 0, 1, 1);
        const pixel = context.getImageData(0, 0, 1, 1).data;
        r = pixel[0];
        g = pixel[1];
        b = pixel[2];
        a = pixel[3] / 255;
      }
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
    const lineMetrics = (el) => {
      const text = (el.innerText || "").trim();
      if (!text || text.length > 2000) return { lineCount: 0, lastLineWidth: 0, maxLineWidth: 0 };
      const range = document.createRange();
      range.selectNodeContents(el);
      const rects = Array.from(range.getClientRects()).filter((rect) => rect.width > 1 && rect.height > 1);
      const lines = [];
      for (const rect of rects) {
        const line = lines.find((entry) => Math.abs(entry.top - rect.top) < Math.max(4, rect.height * 0.5));
        if (line) {
          line.left = Math.min(line.left, rect.left);
          line.right = Math.max(line.right, rect.right);
        } else {
          lines.push({ top: rect.top, left: rect.left, right: rect.right });
        }
      }
      const widths = lines.map((line) => Math.max(0, line.right - line.left));
      return {
        lineCount: widths.length,
        lastLineWidth: widths.length > 0 ? widths[widths.length - 1] : 0,
        maxLineWidth: widths.length > 0 ? Math.max(...widths) : 0
      };
    };
    const metrics = hasText
      ? lineMetrics(el)
      : { lineCount: 0, lastLineWidth: 0, maxLineWidth: 0 };
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
      textAlign: cs.textAlign,
      hyphens: cs.hyphens,
      opacity: parseFloat(cs.opacity),
      visibility: cs.visibility,
      display: cs.display,
      position: cs.position,
      clipPath: cs.clipPath,
      zIndex: cs.zIndex,
      caretColor,
      color,
      background: compositedBg(el),
      backgroundColor,
      backgroundImage: cs.backgroundImage,
      backgroundSize: cs.backgroundSize,
      fontSize: parseFloat(cs.fontSize) || 0,
      fontWeight: parseInt(cs.fontWeight, 10) || 400,
      fontStyle: cs.fontStyle,
      fontFamily: cs.fontFamily,
      padding: [cs.paddingTop, cs.paddingRight, cs.paddingBottom, cs.paddingLeft].map((v) => parseFloat(v) || 0),
      margin: [cs.marginTop, cs.marginRight, cs.marginBottom, cs.marginLeft].map((v) => parseFloat(v) || 0),
      gap: [cs.rowGap, cs.columnGap].filter((v) => v && v !== "normal").map((v) => parseFloat(v) || 0),
      radius: [cs.borderTopLeftRadius, cs.borderTopRightRadius, cs.borderBottomRightRadius, cs.borderBottomLeftRadius],
      radiusPx: [cs.borderTopLeftRadius, cs.borderTopRightRadius, cs.borderBottomRightRadius, cs.borderBottomLeftRadius].map((v) => parseFloat(v) || 0),
      borderColors,
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
      src: tag === "img" ? (el.currentSrc || el.src || null) : null,
      naturalWidth: tag === "img" ? el.naturalWidth : 0,
      naturalHeight: tag === "img" ? el.naturalHeight : 0,
      complete: tag === "img" ? el.complete : null,
      svgShapes: tag === "svg"
        ? el.querySelectorAll("path,circle,rect,ellipse,polygon,polyline,line").length
        : 0,
      lineCount: metrics.lineCount,
      lastLineWidth: metrics.lastLineWidth,
      maxLineWidth: metrics.maxLineWidth,
      borderWidths: [cs.borderTopWidth, cs.borderRightWidth, cs.borderBottomWidth, cs.borderLeftWidth].map((v) => parseFloat(v) || 0),
      transitionDuration: cs.transitionDuration,
      transitionTimingFunction: cs.transitionTimingFunction,
      animationDuration: cs.animationDuration,
      animationTimingFunction: cs.animationTimingFunction,
      animationName: cs.animationName,
      animationIterationCount: cs.animationIterationCount,
      ariaLive: el.getAttribute("aria-live"),
      ariaBusy: el.getAttribute("aria-busy"),
      contentEditable: el.getAttribute("contenteditable"),
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
    frames: document.querySelectorAll("iframe, frame").length,
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

pub(crate) const STYLES_JS: &str = r#" (async () => {
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

pub(crate) const FOCUS_JS: &str = r##"(() => {
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

pub(crate) const DISCOVER_JS: &str = r##"(() => {
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

pub(crate) const ORACLE_JS: &str = r##"((selectors, properties) => {
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
          if (!el) continue;
          if (property.charAt(0) === "@") {
            const attribute = property.slice(1);
            map[String(index)] = el.getAttribute(attribute) || "";
          } else {
            map[String(index)] = getComputedStyle(el).getPropertyValue(property);
          }
        }
      }
      values[property] = map;
    }
    result[selector] = { matches, values, error };
  }
  return JSON.stringify({ selectors: result });
})"##;

pub(crate) fn first_paint_script(selectors: &[String], properties: &[String]) -> String {
    let oracle = oracle_js(selectors, properties);
    let mut script = String::from(
        "(() => { let attempts = 0; const capture = () => { attempts += 1; if (!document.body || attempts > 120) { if (attempts > 120) window.__opal_first_paint = JSON.stringify({ error: 'timeout', href: location.href, count: document.querySelectorAll('*').length, body: !!document.body }); return; } try { const baseline = JSON.parse(",
    );
    script.push_str(JS);
    script.push_str("); const oracle = JSON.parse(");
    script.push_str(&oracle);
    script.push_str(
        "); if (!baseline.elements || baseline.elements.length <= 2) { setTimeout(capture, 0); return; } window.__opal_first_paint = JSON.stringify({ baseline, oracle }); } catch (error) { window.__opal_first_paint = JSON.stringify({ error: String(error) }); } }; const arm = () => { if (document.readyState === 'loading') { document.addEventListener('DOMContentLoaded', () => setTimeout(capture, 0), { once: true }); } else { setTimeout(capture, 0); } }; arm(); })()",
    );
    script
}

pub(crate) fn oracle_js(selectors: &[String], properties: &[String]) -> String {
    format!(
        "({})({}, {})",
        ORACLE_JS,
        serde_json::to_string(selectors).unwrap_or_else(|_| "[]".to_string()),
        serde_json::to_string(properties).unwrap_or_else(|_| "[]".to_string())
    )
}

pub(crate) fn validate_js(selectors: &[String]) -> String {
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

pub(crate) fn activate_js(index: usize) -> String {
    format!(
        r#"(() => {{ const el = document.querySelector('[data-ui-probe-target="{index}"]'); if (!el) return false; el.click(); return true; }})()"#
    )
}

pub(crate) fn activated_js(index: usize) -> String {
    format!(
        r#"(() => {{ const el = document.querySelector('[data-ui-probe-target="{index}"]'); if (!el) return false; return el.getAttribute("aria-expanded") === "true" || el.getAttribute("aria-selected") === "true" || (el.tagName === "SUMMARY" && el.parentElement && el.parentElement.open === true); }})()"#
    )
}

pub(crate) struct BrowserSession {
    runtime: tokio::runtime::Runtime,
    _playwright: Playwright,
    browser: Browser,
    child: std::process::Child,
    profile: PathBuf,
}

impl BrowserSession {
    pub(crate) fn runtime(&self) -> &tokio::runtime::Runtime {
        &self.runtime
    }

    pub(crate) fn browser(&self) -> &Browser {
        &self.browser
    }
}

impl Drop for BrowserSession {
    fn drop(&mut self) {
        let _ = self.runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(2), self.browser.close()).await
        });
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.profile);
    }
}

pub(crate) fn discover_chrome(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path.to_path_buf());
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

pub(crate) fn find_chrome(args: &Args) -> Result<PathBuf> {
    discover_chrome(args.chrome.as_deref())
}

pub(crate) fn which(name: &str) -> Result<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    for directory in std::env::split_paths(&path) {
        let candidate = directory.join(name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    anyhow::bail!("{name} is not on PATH")
}

pub(crate) fn launch_browser(args: &Args) -> Result<BrowserSession> {
    let chrome = find_chrome(args)?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or(0);
    let profile = std::env::temp_dir().join(format!("ui-probe-{}-{stamp}", std::process::id()));
    std::fs::create_dir_all(&profile).context("create chromium profile directory")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&profile, std::fs::Permissions::from_mode(0o700));
    }

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

    let endpoint = endpoint.context("chromium gave no DevTools endpoint")?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("create async runtime")?;
    let connected = runtime.block_on(async {
        let playwright = Playwright::launch()
            .await
            .context("launch playwright driver")?;
        let options = ConnectOverCdpOptions::new().timeout(args.launch_timeout.as_millis() as f64);
        let browser = playwright
            .chromium()
            .connect_over_cdp(&endpoint, Some(options))
            .await
            .context("connect to chromium")?;
        Ok::<_, anyhow::Error>((playwright, browser))
    });
    let (playwright, browser) = match connected {
        Ok(value) => value,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_dir_all(&profile);
            return Err(error);
        }
    };
    Ok(BrowserSession {
        runtime,
        _playwright: playwright,
        browser,
        child,
        profile,
    })
}

pub(crate) async fn settle(tab: &Page, args: &Args) -> Result<()> {
    let wait = r#"(async () => {
  await new Promise((resolve) => {
    let timer;
    let finished = false;
    const finish = () => {
      if (finished) return;
      finished = true;
      observer.disconnect();
      clearTimeout(limit);
      clearTimeout(timer);
      resolve();
    };
    const reset = () => {
      clearTimeout(timer);
      timer = setTimeout(finish, 250);
    };
    const observer = new MutationObserver(reset);
    observer.observe(document.documentElement, {
      subtree: true,
      childList: true,
      attributes: true,
      characterData: true,
    });
    const limit = setTimeout(finish, 2000);
    reset();
  });
  if (document.fonts && document.fonts.ready) await document.fonts.ready;
  const images = Array.from(document.images || []).filter((image) => !image.complete);
  if (images.length > 0) {
    await Promise.race([
      Promise.all(images.map((image) => new Promise((resolve) => {
        image.addEventListener("load", resolve, { once: true });
        image.addEventListener("error", resolve, { once: true });
      }))),
      new Promise((resolve) => setTimeout(resolve, 2000))
    ]);
  }
  return true;
})()"#;
    let _ = evaluate_js(tab, wait).await;
    let stabilize = r#"(() => {
  if (!document.getAnimations) return true;
  for (const animation of document.getAnimations()) {
    try {
      animation.pause();
      animation.currentTime = 0;
    } catch (_) {}
  }
  return true;
})()"#;
    let _ = evaluate_js(tab, stabilize).await;
    if args.freeze_animations {
        let freeze = r#"(() => {
  const style = document.createElement("style");
  style.setAttribute("data-ui-probe-freeze", "");
  style.textContent = "*, *::before, *::after { animation: none !important; transition: none !important; scroll-behavior: auto !important; }";
  (document.head || document.documentElement).appendChild(style);
  return true;
})()"#;
        let _ = evaluate_js(tab, freeze).await;
    }
    Ok(())
}

pub(crate) async fn load(tab: &Page, args: &Args) -> Result<()> {
    let url = args.url.as_deref().context("--url is required")?;
    tab.goto(url, None)
        .await
        .with_context(|| format!("navigate to {url}"))?;
    tab.wait_for_load_state(None)
        .await
        .context("wait for navigation")?;
    if let Some(selector) = &args.wait_for {
        tab.locator(selector)
            .wait_for(None)
            .await
            .with_context(|| format!("wait for {selector}"))?;
    }
    tokio::time::sleep(Duration::from_millis(400)).await;
    settle(tab, args).await?;
    Ok(())
}

pub(crate) async fn evaluate(tab: &Page) -> Result<Value> {
    evaluate_js(tab, JS).await
}

pub(crate) async fn evaluate_js(tab: &Page, js: &str) -> Result<Value> {
    let raw: Value = tab
        .evaluate(js, None::<&()>)
        .await
        .context("evaluate page")?;
    match raw {
        Value::String(text) => serde_json::from_str(&text).context("parse page payload"),
        other => Ok(other),
    }
}
