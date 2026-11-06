---
name: opal
description: UI craft: DTCG tokens, wireframing, and rendered audits.
license: MIT
compatibility: Requires Chromium and ui-probe.
disable-model-invocation: true
---

# Opal

Use when the diff renders or changes a user interface.

## Authoring

- Decompose compound primitives into parts (`Root`, `Trigger`, `Portal`, `Content`, `Close`); use context instead of prop drilling.
- Model async state as a strict union: `idle`, `loading`, `success`, `empty`, `error`; cancel stale work with `AbortController`.
- Wire WAI-ARIA roles and state (`aria-expanded`, `aria-controls`), keyboard navigation, and visible focus.
- Support `asChild` composition and ref forwarding; merge classes with `cn()`.
- Animate only compositor properties (`transform`, `opacity`); respect `prefers-reduced-motion`.

## Design specificity

- Decide whether this composition could belong to an unrelated product unchanged; if yes, report it.
- Check hierarchy, information architecture, cognitive load, and whether the primary action is findable within seconds.
- Check brief coverage: every requirement in `artifacts/ui-design.md` is present and findable within seconds; list shipped features the brief never asked for.
- Compare against `artifacts/ui-design.md` when present; treat its tokens and constraints as authoritative.

## Anti-patterns

The rules pack reports slop patterns. Treat its findings as advisory, not bans: a documented brief can earn any pattern, and the finding must name the earning line.

## Heuristics

- Check visibility of system status, match to the user's world, user control, consistency, error prevention, recognition over recall, flexibility, minimalist design, error recovery, and help.
- Flag a crowd of primary choices, hidden state, working-memory load, and unchunked data.
- Report severity as blocking use, blocking the primary task, degrading trust or speed, or polish.

## Setup

Run `scripts/ui-audit.sh init` once per project. Fill `.opal/design.json`; add `.opal/rules.json` only to override the bundled Impeccable pack. Then run `scripts/ui-audit.sh audit <url> -- --theme both --viewport sweep`.

## Audit

Read `artifacts/ui-design.md` when present; use its tokens, density, typography, and constraints as the design source. If it is absent, infer them from the diff.

Run `scripts/ui-audit.sh source` for source patterns in changed UI files before reading the diff. Treat matches as evidence, not findings; verify each match in context.

Run `scripts/ui-audit.sh audit <url> -- [flags]` before judging. A rendered review without a probe run is incomplete. The probe renders desktop and mobile viewports in light and dark themes and samples hover and focus states; it reports findings derived from rendered geometry and computed colors, plus a candidate list and tab order, as JSON. Use `--expand` to open each ARIA disclosure one at a time and report per-state deltas. Read the script and the report for thresholds and evidence. Without the tool or a browser, inspect supplied screenshots and list every unrendered check under unverified.

Exercise preload, loaded, empty, error, disabled, collapsed, and expanded; snapshot only the states the diff touches.

- Every interactive control needs an accessible name, visible focus indicator, and keyboard trigger.
- Widget roles carry their states: `aria-expanded`, `aria-controls`, `aria-selected`, `aria-pressed`, `aria-current`. Async status uses a live region. No ARIA overrides native semantics without cause.
- Touch targets and text contrast meet the thresholds in the bundled rules pack and the report evidence.
- Body measure, tracking, and overflow stay within the report limits; no text overflows its container. Display sizes follow `artifacts/ui-design.md`, not a fixed ceiling.
- Compare computed padding, margin, and gap against the density profile.
- Declare elevation once, border or shadow. Flag a hairline border under a wide soft shadow as a ghost card. Match radii to `artifacts/ui-design.md`; without a spec, flag mixed radii on comparable surfaces, not a numeric range. Keep radius units: a percentage never equals the same number of pixels.
- Theme selection, caret, scrollbars, focus rings, underline offset, and tabular numerals come from the palette; browser defaults belong to no design system.
- Resolve computed colors across themes; report off-palette nodes.
- Never suppress focus rings without an equivalent visible focus ring.
- Never introduce raw color literals in components; fix token mappings.
- Cite source and rendered evidence: viewport, theme, input method, and what was not exercised.

## Rules

- Primitives stay headless and unstyled by default; styles merge via `cn()`.
- Never use disjoint flags (`isLoading && isError`); use a strict discriminated union.
- Write findings with `file:line` and a remediation diff to `artifacts/review/ui-<topic>.md`.
- Verify: zero off-palette colors, zero accessibility violations, and conformance to the target density profile.
