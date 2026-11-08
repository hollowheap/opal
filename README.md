# opal

UI craft skill: DTCG design tokens, wireframing, and rendered audits with `ui-probe`.

## Layout

- `skills/opal/SKILL.md` — the skill.
- `scripts/compile.sh` — build `ui-probe` into `~/.opal/bin` and install the default rules pack to `~/.opal/rules.json`.
- `skills/opal/references/impeccable-rules.json` — default advisory rules pack.
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

`init` creates `.opal/design.json` and `.opal/README.md`. Rules resolve from `--rules`, the nearest `.opal/rules.json`, then `~/.opal/rules.json`; named rules overlay by id and structural rules append. Design layers from `--design`, the nearest `.opal/design.json`, then `~/.opal/design.json`. The report lists the resolved files under `sources` and tags rule findings with `ruleSource`. `pack` prints the resolved rules files; `pack --json` includes their origins. `--write-baseline` seeds `.opal/baseline.json`, and later runs suppress known fingerprints. Inline suppressions use `opal-ignore` comments in source and `data-opal-ignore` in rendered pages. `--format sarif|markdown|github` changes report output; `doctor` checks the local setup.

Requires Chromium. `ui-probe` resolves it from `--chrome`, `UI_PROBE_CHROME`, or `PATH`.

## Build

```bash
cd ui-probe && cargo build --release
```
