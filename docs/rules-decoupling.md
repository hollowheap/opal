# Decoupling ledger

The default pack is now fully declarative. Every one of its 62 `.rules` entries carries a selector or page/sheet scope and a `when` expression. The additional structural rule, `no-gradient-text`, is also declarative.

The checks run from `skills/opal/references/default-rules.json`, not from a per-rule Rust branch.

## Migrated behaviour groups

The default pack covers:

- layout and geometry: card depth, side tabs, line measure, opening-column overflow, clipped text, clipped escape, text/sibling overlap, target size, spacing uniformity, tight containers, viewport-edge text, heading spacing, and page overflow;
- type and copy: kickers, tracking, line height, type scale, uppercase passages, short tails, repeated copy, numeric indexes, marketing copy, dismissive copy, rebuttal cadence, and numeric or textual labels;
- colour and material: contrast, cream canvas, purple palette, washed tint, washed images, radial washes, halos, unblurred shadows, and colour-token checks;
- motion and surfaces: bounce easing, blinking carets, pulsing dots, continuous scrollers, stripe and grid backgrounds;
- resolution: aliases, coverage gaps, provenance, and lifecycle-specific rules.

`missing-lang` is now a page rule too: the page evidence carries `lang`, so the root element does not need to be in the element array.

## Compiled fallback

`ui-probe/src/rules/compiled.rs` remains only for explicit legacy ids that are not part of the declarative default floor. The normal no-pack path loads the shipped default pack, and project packs overlay that declarative floor by id: a same-id named entry updates severity, layer, params, or enabled state, and a same-id declarative rule replaces the default rule.

The default pack never uses the fallback. Its names are all declared, so compiled findings are filtered whenever the pack is loaded.

The four names that had no implementation anywhere (`aphoristic-cadence`, `dark-glow`, `edge-flush-cards`, `script-error`) are deleted rather than rewritten; a pack that names them gets a coverage gap. `gpt-thin-border-wide-shadow` stays out of the default pack by the repository scrub rule; an explicit pack that selects it still reaches the legacy engine id.

## Removal condition

The compiled fallback now covers only non-default legacy ids such as the scrubbed engine id. Delete `compiled.rs` once those ids are retired or represented outside the default pack.
