# opal

Rust/Tokio CLI (`ui-probe`) and declarative v2 rules pack that render pages in headless Chromium via `playwright-rs` and emit deterministic JSON UI audits.

## Essential Commands

- Build / check: `nix develop --command bash -c 'cargo fmt --manifest-path opal/Cargo.toml --check && cargo clippy --manifest-path opal/Cargo.toml --all-targets -- -D warnings && cargo build --manifest-path opal/Cargo.toml'`
- Test suite: `nix develop --command bash -c 'cargo test --manifest-path opal/Cargo.toml && bash opal/tests/run.sh'`
- Packaging: `nix develop --command bash scripts/compile.sh`
- Single check / targeted feedback: `nix develop --command bash opal/tests/run.sh`

## Conventions

- Output is JSON always; errors write to stderr and stdout stays a single JSON document.
- Deterministic output: same page yields identical JSON; verify capture changes in order (`cargo fmt`, `cargo clippy --all-targets -- -D warnings`, debug build, `opal/tests/run.sh`, `scripts/compile.sh`, then a two-run diff on a fixed page).
- `skills/opal/references/default-rules.json` is the generic v2 advisory floor. Its 78 `.rules` entries and 10 `.sourceRules` entries are declarative.
- The default pack carries generic behaviour ids. The optional pack carries non-default behaviours; a project may extend it.
- `heading-rhythm` and `heading-spacing-relation` are separate laws and keep separate ids.
- Capture settling waits for DOM quiet, fonts, and pending images, then pins animations to phase `0`; never replace settling with a fixed sleep alone.
- Never make the Nix package depend on a user cache; keep the Playwright Node driver pinned in Nix.
- Keep repeatable check scripts in the workspace scripts directory rather than ad-hoc `/tmp` scripts.
- UI audit workflow runbook: see `skills/opal/SKILL.md`.
