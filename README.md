# opal

UI craft skill: DTCG design tokens, wireframing, and rendered audits with `opal`.

## Layout

- `skills/opal/SKILL.md` — the skill.
- `scripts/compile.sh` — build `opal` into `$OPAL_SKILL_DIR/scripts/opal`; rules are created per workspace by `opal init`.
- `skills/opal/references/default-rules.json` — default advisory rules pack.
- `skills/opal/references/optional-rules.json` — optional non-default behaviour pack.
- `skills/opal/references/rules.schema.json` — version 2 pack schema.
- `docs/rules-format.md` — the whole rule format, verbs, shapes, and worked examples.
- `opal/` — Rust capture and audit binary.

## Use

Copy `skills/opal` into a harness skills directory, or install the repo as a skill source.

```bash
OPAL_SKILL_DIR=<skill-dir> scripts/compile.sh
"$OPAL_SKILL_DIR/scripts/opal" init
"$OPAL_SKILL_DIR/scripts/opal" doctor
"$OPAL_SKILL_DIR/scripts/opal" audit http://localhost:3000 --theme both --viewport sweep
"$OPAL_SKILL_DIR/scripts/opal" source .
```

`init` creates `.opal/design.json`, `.opal/rules.json`, and `.opal/README.md`. Rules resolve from `--rules` or the nearest `.opal/rules.json`; no rules source is an error. Project named entries overlay ids in the chosen pack; project declarative rules replace matching ids and append when new. Design layers from `--design`, the nearest `.opal/design.json`, then `~/.opal/design.json`. The report lists the resolved files under `sources` and tags rule findings with `ruleSource` and `packSource`. The rule format is documented in `docs/rules-format.md`. `pack` prints the resolved rules files; `pack --json` includes their origins. `--write-baseline` seeds `.opal/baseline.json`, and later runs suppress known fingerprints. Inline suppressions use `opal-ignore` comments in source and `data-opal-ignore` in rendered pages. `--format sarif|markdown|github` changes report output; `doctor` checks the local setup. Runtime console, page, and network errors become findings; `--freeze-animations` stabilizes geometry.

Requires Chromium. `opal` resolves it from `--chrome`, `UI_PROBE_CHROME`, or `PATH`.

## Build

```bash
cd ui-probe && cargo build --release
```
