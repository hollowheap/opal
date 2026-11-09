# opal repository rules

- Output is JSON always. Errors go to stderr; stdout stays a single JSON document.
- The binary is deterministic: the same page in gives the same JSON out. Any capture-layer change must be proven with a two-run diff on a real surface.
- The default rules pack is the generic advisory floor. A stricter pack entry that shares an id is a deliberate tightening, not drift. See `skills/opal/references/README.md`.
- `heading-rhythm` is the fixed-range law carried by a surface pack. The engine's relational law is `heading-spacing-relation`: space above a heading must exceed space below it. They are separate laws and keep separate ids.
- The default pack never carries `gpt-thin-border-wide-shadow`, under any id.
- The capture layer is the playwright-rs driver over Chromium. Page settling waits for DOM quiet, fonts, and pending images, then pins animations to their starting phase before sampling; do not replace that with a fixed sleep alone.

## History

The original local commits `218491f` and `9b2718e`, and the `pre-upstream-pull-2026-11-09` stash, were absent from this clone when the merge was attempted. This tree is a best-effort reconstruction on top of upstream `c12ea77`: the playwright-rs port, the settled rules documents, and the default-pack scrub were rebuilt from the brief and the upstream record. The original commit hashes, the 169-test local baseline, and the 29 stash fixtures could not be preserved. The reconstructed suite passes 233 checks; the unmodified upstream `c12ea77` suite passes 229. The installed binary produced byte-identical JSON for two runs against the pico-car surface at `http://127.0.0.1:8083/` and against a static multi-viewport fixture.
