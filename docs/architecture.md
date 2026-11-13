# Architecture

The rule engine, design, output, source, and capture capabilities are separate slices. `main.rs` is the composition root: it wires the audit flow but does not own rule parsing/evaluation, design parsing, report rendering, source scanning, or browser lifecycle code.

## Rules slice

`ui-probe/src/rules/` owns the rule domain:

- the pack model and versioned schema shape;
- pack loading, extends folding, and alias resolution;
- selector and property-test parsing;
- rule evaluation over evidence sets;
- provenance and coverage resolution;
- the compiled fallback checks in `compiled.rs`.

The slice exposes a narrow surface to the composition root:

- `load_rules` and `expand_rules_sources` for pack resolution;
- `RuleSet` with methods for selectors, properties, declared names, first-paint needs, disabled checks, and coverage;
- `rule_findings` and `apply_ruleset` for evaluation;
- `check_keys` for the small shared config validation used by baseline files.

`compiled.rs` is a legacy fallback, not a permanent module boundary. Its checks are grouped under the rules slice because they are rule behaviours. The decoupling pass dissolves them into default-pack rules and reusable evidence primitives; when that is complete, the file disappears.

## Design slice

`ui-probe/src/design/` owns the design model, DTCG and flat design parsing, token grouping, token-set membership helpers, design drift checks, and inferred design clustering. `main.rs` still owns design source resolution and writing inferred design files.

## Output slice

`ui-probe/src/output/` owns `OutputFormat` and the JSON, SARIF, Markdown, and GitHub renderers. It takes a finished report value and writes it out. It owns no judgement and does not read rules, design files, or browser state.

## Source slice

`ui-probe/src/source/` owns source-file discovery, UI-file filtering, `opal-ignore` parsing, the source regex rules, and the source scan result. `main.rs` still owns baseline application and report assembly for source mode.

## Capture slice

`ui-probe/src/capture/` owns browser launch and shutdown, Chromium discovery, page settling, navigation, page evaluation, the evidence JavaScript, and the staged first-paint capture. `main.rs` still owns the audit orchestration, state expansion, and report assembly around those captures.

## Composition root

`main.rs` wires the CLI, audit orchestration, baseline application, rule/design/source loading, and report assembly. It consumes the rule, design, output, source, and capture slices rather than owning their internals.

The rule slice must not depend on browser or output modules. It consumes evidence values and design token sets, and returns findings. The output slice renders finished report data. The source slice scans files and returns findings. The design slice parses design data and returns token sets. The capture slice produces evidence at named lifecycle points. Dependency direction stays inward.

## Migration rule

Do not move hardcoded check bodies from `compiled.rs` into `main.rs`. Move their reusable evidence calculations into the rules domain or a shared evidence module, express the checks as default-pack rules, prove parity, then delete the compiled bodies.
