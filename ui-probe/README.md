# ui-probe

Rendered and source UI evidence as JSON. Exit codes: `0` clean, `2` findings, `1` error.

## Modes

- `--url <url>` samples a page in headless Chromium.
- `--source <dir>` scans changed UI files without a browser. `--source-all` scans every UI file.

## Capture

The browser capture records every element, including hidden ones, with `visible`, `parentIndex`, and a stable `data-ui-probe` marker. Colours are normalized through a canvas `fillStyle` round-trip. A colour that cannot be normalized is listed under `unverified`.

## Rules

`--rules <path>` loads a versioned rules pack. Structural rules use CSS property names, `ancestors`/`descendants` constraints, and selector or design-parameter comparisons. Named rules use the registry ids below. Invalid keys, rule ids, properties, selectors, and malformed `param:` references exit `1`.

Registry ids: `nested-cards`, `kicker-above-heading`, `line-length`, `extreme-negative-tracking`, `gpt-thin-border-wide-shadow`, `side-tab`, `em-dash-overuse`, `marketing-buzzword`, `bounce-easing`.

Every state is evaluated against the pack. A finding carries `state`, `layer`, and, when merged across runs, `states`, `themes`, and `viewports`.

## Design file

`--design <path>` reads the DTCG schema: groups `color`, `font`, `typography`, `spacing`, `radius`, and `surface`, with `$value`, `$type`, `$description`, and `$extensions`. `$type: gradient` tokens are accepted and excluded from colour checks. The earlier flat shape (`colors`, `spacingPx`, `radiiPx`, `fontFamilies`) remains a fallback.

`--design auto` infers colours, font families, spacing, radii, and typography clusters, then writes `.ui-witness/design.json` in the DTCG shape with provisional keys such as `color-1`, `font-1`, and `type-1`. Counts and confidence stay in the report under `inferredDesign`. An existing file is not overwritten without `--force`. `--infer-missing` fills only absent keys.

## Allowlist

`--allow <check-id>[,<check-id>]` suppresses named checks. Use it when the brief earns a generic pattern, for example `--allow nested-cards,kicker-above-heading`.

## Chromium

`--chrome <path>` or `UI_PROBE_CHROME` selects the browser. `--launch-timeout <ms>` bounds startup; a failed launch reports the browser's own stderr and exit status.
