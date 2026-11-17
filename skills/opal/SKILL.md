---
name: opal
description: UI craft: DTCG tokens, wireframing, and rendered audits.
license: MIT
compatibility: Requires Chromium and opal.
disable-model-invocation: true
---

# Opal

## Scope

- Covers: UI composition, design specificity, rendered audits, and motion, typography, and polish checks when a diff renders or changes a user interface.
- Excludes: backend-only changes, non-visual behavior, and unrendered visual claims.

## Design specificity

- Treat `DESIGN.md` as authoritative when present; infer tokens, density, typography, and constraints from the diff otherwise.
- Apply the critique and distill actions to judge product specificity and brief coverage.

## Anti-patterns

The rules pack reports slop patterns. Treat its findings as advisory, not bans: a documented brief can earn any pattern, and the finding must name the earning line.

## Setup

Run `scripts/compile.sh` once per machine. Run `scripts/opal init` once per project, fill `.opal/design.json`, use `.opal/rules.json` only to override named rules or add declarative rules, then run `scripts/opal audit <url> --theme both --viewport sweep`. Run `scripts/opal doctor` when setup fails. See `references/README.md` for rule and design resolution, baselines, and suppressions.

## Evidence

Every action checks both channels before it judges the work:

- Source: run `scripts/opal source .` and read `references/source.md`.
- Computed: run `scripts/opal audit <url> --theme both --viewport sweep` and read `references/audit.md`.

Without a browser, inspect supplied screenshots and list every computed check as unverified.

## Actions

Load actions by entry condition; each consumes both evidence channels and resumes at `## Rules`:

- Design clarity, hierarchy, or product specificity: `references/critique.md`.
- Motion change or motion finding: `references/animate.md`.
- Typography change or display-size finding: `references/typeset.md`.
- Scope creep or repeated copy: `references/distill.md`.
- Loading, cache, network, or health finding: `references/optimize.md`.
- Focus, hover, spacing, or theme finding: `references/polish.md`.

## Rules

- Primitives stay headless and unstyled by default; styles merge via `cn()`.
- Never use disjoint flags (`isLoading && isError`); use a strict discriminated union.
- Report severity as blocking use, blocking the primary task, degrading trust or speed, or polish.
- Write findings with `file:line` and a remediation diff to `artifacts/review/ui-<topic>.md`.
- Verify: zero off-palette colors, zero accessibility violations, and conformance to the target density profile.
