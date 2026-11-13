# opal

UI craft skill: DTCG design tokens, wireframing, and rendered audits with `ui-probe`.

## Layout

- `skills/opal/SKILL.md` — the skill.
- `scripts/compile.sh` — build `ui-probe` into `~/.opal/bin` and install the default rules pack to `~/.opal/rules.json`.
- `skills/opal/references/default-rules.json` — default advisory rules pack.
- `skills/opal/references/rules.schema.json` — version 2 pack schema.
- `docs/rules-format.md` — the whole rule format, verbs, shapes, and worked examples.
- `docs/rules-decoupling.md` — the decoupling ledger for compiled behaviours.
- `docs/architecture.md` — the current modular slices and migration boundary.
- `ui-probe/` — Rust capture and audit binary.

## Use

Copy `skills/opal` into a harness skills directory, or install the repo as a skill source.

```bash
opal/scripts/compile.sh
~/.opal/bin/ui-probe init
~/.opal/bin/ui-probe doctor
~/.opal/bin/ui-probe audit http://localhost:3000 --theme both --viewport sweep
~/.opal/bin/ui-probe source .
```

`init` creates `.opal/design.json` and `.opal/README.md`. Rules resolve from `--rules`, the nearest `.opal/rules.json`, then `~/.opal/rules.json`; if none resolves, the shipped version 2 default floor is used. Project named entries overlay default ids; project declarative rules replace matching ids and append when new. Design layers from `--design`, the nearest `.opal/design.json`, then `~/.opal/design.json`. The report lists the resolved files under `sources` and tags rule findings with `ruleSource` and `packSource`. The rule format is documented in `docs/rules-format.md`. `pack` prints the resolved rules files; `pack --json` includes their origins. `--write-baseline` seeds `.opal/baseline.json`, and later runs suppress known fingerprints. Inline suppressions use `opal-ignore` comments in source and `data-opal-ignore` in rendered pages. `--format sarif|markdown|github` changes report output; `doctor` checks the local setup. Runtime console, page, and network errors become findings; `--freeze-animations` stabilizes geometry.

Requires Chromium. `ui-probe` resolves it from `--chrome`, `UI_PROBE_CHROME`, or `PATH`.

## Build

```bash
cd ui-probe && cargo build --release
```
