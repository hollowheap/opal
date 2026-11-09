# Rule packs

`default-rules.json` is the generic advisory floor. Project packs overlay named rules by id and append structural rules. A pack that tightens a default keeps the same id so the overlay is explicit. A same-id entry is a deliberate tightening, not drift; the stricter pack must not loosen the default.

## Default and tightened pairs

These defaults are intentionally loose. A surface pack may tighten them; it must not loosen the default.

- `side-tab`: the default tolerance is the generic floor; a tightening pack narrows it to one pixel.
- `kicker-above-heading`: the default requires uppercase or tracking; a tightening pack makes the form unconditional.
- `em-dash-overuse`: the default allows eight; a tightening pack sets zero.
- `bounce-easing`: the default is advisory; a tightening pack raises it to error.
- `extreme-negative-tracking`: the default floor is `-0.04em`; a tightening pack uses `-0.02em`.
- `no-gradient-text`: the default is advisory; a tightening pack raises it to error.

## `heading-rhythm`

`heading-rhythm` keeps the fixed-range law on the surface pack: 28 to 40 pixels above a heading and 8 to 12 pixels below. The relational law lives in the engine as `heading-spacing-relation`: space above a heading must exceed space below it across the page.

The fixed range is a surface-specific tightening. The relational law is the generic page rule. A heading that passes the fixed range also passes the relational law, but not the reverse. Sharing one id would hide two laws, so the relational law gets its own name.

## Content patterns

`rebuttal-cadence` and `dismissive-copy` are parameterised content-pattern rules. Packs can override their patterns and thresholds without engine changes.

## Scrub state

The default pack does not ship `gpt-thin-border-wide-shadow` under any id. The house removed that rule from every pack copy on 2026-11-08; the engine may still know the id for explicitly supplied packs, but the shipped floor must not bring it back.
