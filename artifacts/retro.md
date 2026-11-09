# Retro: opal merge and playwright-rs port

Source: the active pi session transcript for this opal session (1,094 records, 832 messages).

## Tool Efficiency & Errors

- 585 tool results, 92 errors: **15.7% error rate**.
- Tool calls: `bash` 526, `edit` 26, `write` 18, `task` 8, `read` 6, `codemode` 3, `create_todos` 1 (no such tool).
- Error split: 57 pi-guard denials, 8 timeouts, 12 permission-denied paths, 3 missing-file reads, 2 missing `cargo` on `PATH`, 2 suite failures, 2 fatal git probes, 3 other command-not-found. Categories overlap.
- Largest sink: proving the described pre-merge state was absent. Session stores, opencode snapshots, Nix store, undo history, and editor state were searched after `git cat-file` and `git stash list` already showed the local objects were gone. That was necessary to avoid inventing a side, but the forensic phase should have ended at the wall.
- Guard friction: compound commands with `&&`/`;`, output redirection, heredocs, and paths outside the repo were the main denial sources. Simple single-purpose commands and the `read` tool worked reliably.
- One false test failure came from running the suite against a stale `target/debug/ui-probe` after source changes. The suite defaults to the debug binary, so debug must be rebuilt before `run.sh`.
- Late packaging discovery: the Nix package built but would have needed a user cache for the Playwright Node driver. Pinning the driver in the flake fixed the clean-home runtime.

## Winning Patterns

- Locate symbols with `rg '^fn|^struct|...'`, then use `read` with offsets instead of dumping huge files.
- Recover deleted upstream decision text with `git show <commit>:<path>` rather than reconstructing prose from memory.
- Apply small exact `edit` blocks, then run `cargo fmt`, `cargo clippy -D warnings`, and a debug build in one verification batch.
- Test both binaries: debug via `ui-probe/tests/run.sh`, release via `UI_PROBE_BIN=~/.opal/bin/ui-probe`.
- Prove determinism with `artifacts/scripts/determinism-check.sh` on a fixed page and a fixed viewport/theme matrix; use a live surface for the smoke check, not for a long sweep where app data can change.
- Keep repeatable checks in `artifacts/scripts/` instead of ad-hoc `/tmp` scripts.

## Actionable Improvements

- In `AGENTS.md`, require a capture-change verification order: format, clippy, debug build, full suite, release install, determinism diff.
- In `AGENTS.md`, require the Nix package to stay self-contained by pinning the Playwright Node driver; do not rely on a developer cache.
- On future merge briefs, run one fixed state check first (`HEAD`, expected local commits, `stash list`, `MERGE_HEAD`, `fsck --unreachable`). If the local side is absent, stop and report the missing objects before searching session archives.
- Prefer simple single-purpose shell commands in this harness. Avoid compound pipelines, redirects, and heredocs when a script file or `read`/`write` tool can do the job.
