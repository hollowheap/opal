# opal

UI craft skill: DTCG design tokens, wireframing, and rendered audits with `ui-probe`.

## Layout

- `skills/opal/SKILL.md` — the skill.
- `skills/opal/scripts/ui-audit.sh` — init, audit, source, pack.
- `skills/opal/references/impeccable-rules.json` — default advisory rules pack.
- `ui-probe/` — Rust capture and audit binary.

## Use

Copy `skills/opal` into a harness skills directory, or install the repo as a skill source.

```bash
opal/skills/opal/scripts/ui-audit.sh init
opal/skills/opal/scripts/ui-audit.sh audit http://localhost:3000 --theme both --viewport sweep
```

Requires Chromium. `ui-probe` resolves it from `--chrome`, `UI_PROBE_CHROME`, or `PATH`.

## Build

```bash
cd ui-probe && cargo build --release
```
