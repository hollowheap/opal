# Audit reference

Computed evidence channel for every action.

Use after `scripts/opal audit <url> --theme both --viewport sweep`.

- Treat findings as advisory prompts, not bans.
- Exercise preload, loaded, empty, error, disabled, collapsed, and expanded states.
- Cite viewport, theme, input method, and unexercised states.
- Treat `console-error`, `page-error`, and `network-error` as real defects; warnings are advisory.
- Use `--freeze-animations` when motion makes geometry unstable; rerun without it for motion checks.
- Use `--expand` to open each ARIA disclosure one at a time and report per-state deltas.

Design checks:
- Read `DESIGN.md` when present; use its tokens, density, typography, and constraints as authoritative.
- Compare computed padding, margin, and gap against the density profile.
- Match radii to `DESIGN.md`; without a spec, flag mixed radii on comparable surfaces.
- Display sizes follow `DESIGN.md`, not a fixed ceiling.
- Resolve computed colors across themes; report off-palette nodes.
- Never suppress focus rings without an equivalent visible focus ring.
- Keep radius units: a percentage never equals the same number of pixels.
- Theme selection, caret, scrollbars, focus rings, underline offset, and tabular numerals come from the palette; browser defaults belong to no design system.
- Verify: zero off-palette colors, zero accessibility violations, conformance to target density.