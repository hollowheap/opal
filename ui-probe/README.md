# ui-probe

Rendered and source UI evidence as JSON. Exit codes: `0` clean, `2` findings, `1` error. Reports carry `schemaVersion: 1`.

## Commands

- `init [dir]` creates `.opal/design.json` and `.opal/README.md`.
- `audit <url> [flags]` samples a page in headless Chromium.
- `source [dir] [flags]` scans changed UI files without a browser. `--source-all` scans every UI file.
- `pack [--rules <path>] [--json]` prints the resolved rules files, one per line, highest precedence first.
- `doctor [--json]` checks Chromium, project config, rules, design, and baseline.

The legacy `--url` and `--source` flags remain accepted.

## Formats

`--format json|sarif|markdown|github` controls `audit` and `source` output. SARIF is version 2.1.0. The GitHub format emits workflow annotations.

## Capture

The browser capture records every element, including hidden ones, with `visible`, `parentIndex`, and a stable `data-ui-probe` marker. Colours are normalized through a canvas `fillStyle` round-trip, so modern syntaxes such as `oklch()` and `color-mix()` resolve to rgba. A colour that cannot be normalized is listed under `unverified`. Iframe and closed-shadow content is not sampled; both are reported under `unverified`.

Capture runs through the playwright-rs driver over the Chromium DevTools endpoint. Settling waits for DOM quiet, fonts, and images, then pins CSS animations to their starting phase before sampling; findings are sorted by severity and stable evidence keys. That is what makes the same page produce the same JSON across runs.

## Expanded checks

The engine also runs fixed advisory checks for page health, geometry, material, typography, motion, and content cadence. Findings carry the measured values in `evidence`. Use `--allow`, inline suppressions, or a baseline to settle an accepted pattern.

## Runtime diagnostics

The probe captures console errors and warnings, uncaught page errors, and failed or 4xx/5xx network requests as `console-error`, `console-warning`, `page-error`, and `network-error` findings. Capture waits for fonts and images. `--freeze-animations` disables animations and transitions before sampling.

## Rules

Rules resolve from `--rules`, the nearest `.opal/rules.json` found from the working directory to the git root, then `~/.opal/rules.json`. Every existing file is loaded. Named rules overlay by id: the first source that names an id wins. Structural rules append from every source. The report lists the loaded files under `sources.rules` and tags each rule finding with `ruleSource`.

Structural rules use CSS property names, `ancestors`/`descendants` constraints, and selector or design-parameter comparisons. Named rules use the registry ids below. Invalid keys, rule ids, properties, selectors, and malformed `param:` references exit `1`.

A pack can list `extends` as a path or array of paths. Relative paths resolve from the referring pack. Extended packs load after the pack that names them. Cycles and missing files exit `1`. Packs may carry `$schema: "./rules.schema.json"` for editor validation; the schema lives beside the default pack.

Registry ids: `nested-cards`, `kicker-above-heading`, `line-length`, `extreme-negative-tracking`, `gpt-thin-border-wide-shadow`, `side-tab`, `em-dash-overuse`, `marketing-buzzword`, `bounce-easing`. The shipped default pack omits `gpt-thin-border-wide-shadow` intentionally; an explicit project pack may still select the engine id.

Every state is evaluated against the pack. A finding carries `state`, `layer`, and, when merged across runs, `states`, `themes`, and `viewports`.

## Design file

The design source layers from `--design`, the nearest `.opal/design.json`, then `~/.opal/design.json`. Higher layers override matching token paths. The report lists the chosen files under `sources.design` and their origins under `sources.designOrigin`.

`--design <path>` reads the DTCG schema: groups `color`, `font`, `typography`, `spacing`, `radius`, and `surface`, with `$value`, `$type`, `$description`, and `$extensions`. `$type: gradient` tokens are accepted and excluded from colour checks. The earlier flat shape (`colors`, `spacingPx`, `radiiPx`, `fontFamilies`) remains a fallback.

`--design auto` infers colours, font families, spacing, radii, and typography clusters, then writes `.ui-witness/design.json` in the DTCG shape with provisional keys such as `color-1`, `font-1`, and `type-1`. Counts and confidence stay in the report under `inferredDesign`. An existing file is not overwritten without `--force`. `--infer-missing` fills only absent keys.

## Allowlist

`--allow <check-id>[,<check-id>]` suppresses named checks globally. Use it when the brief earns a generic pattern, for example `--allow nested-cards,kicker-above-heading`.

## Baseline

Each finding carries a stable `fingerprint`. `--write-baseline` writes the current fingerprints to `.opal/baseline.json` and exits `0`. `--update-baseline` adds new fingerprints to an existing baseline. Later runs suppress matching findings and report the count under `baseline.suppressed`.

The default baseline is the nearest `.opal/baseline.json`. `--baseline <path>` selects an explicit file; `--no-baseline` disables discovery. Baselines apply to `audit` and `source`.

## Suppressions

Source files accept `opal-ignore <id>` on the matching line, `opal-ignore-next-line <id>`, and `opal-ignore-file <id>`. Rendered pages accept `data-opal-ignore="<id>"` on the element or an ancestor. Use `*` to suppress every rule finding for that element or line.

## Doctor

`ui-probe doctor` validates Chromium, project config, rules, design, and baseline. It exits `1` when a required check fails. `--json` emits machine-readable checks.

## Chromium

`--chrome <path>` or `UI_PROBE_CHROME` selects the browser. `--launch-timeout <ms>` bounds startup; a failed launch reports the browser's own stderr and exit status.
