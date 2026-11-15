# Rule packs

`default-rules.json` is the version 2 generic advisory floor. Project packs overlay named rules by id and append declarative rules. A pack that tightens a default keeps the same id so the overlay is explicit. A same-id entry is a deliberate tightening, not drift; the stricter pack must not loosen the default. The whole rule format is documented in `docs/rules-format.md`; the schema is `rules.schema.json`.

A version 2 rule carries a selector, optional ancestor and exception conditions, a list of property tests, an optional operator, optional cardinality, and an optional lifecycle point. The closed verb set is `in`, `notIn`, `equals`, `notEquals`, `present`, and `absent`; multi-value properties may use `components`, attributes may use `attributes`, and `when` expressions cover derived measurements.

Names that resolve to no default rule, alias baseline, or declarative rule body are reported as coverage gaps, never skipped. The three former dead names `aphoristic-cadence`, `edge-flush-cards`, and `script-error` have no implementation and are not part of the floor.

## Default and optional packs

All 78 `.rules` entries and 10 `.sourceRules` entries in `default-rules.json` are declarative. The structural `no-gradient-text` rule is declarative too. A project can see the whole floor, disable a rule, or override its severity and layer.

`optional-rules.json` carries generic non-default behaviours. A project can extend that pack and select its rule ids. `gpt-thin-border-wide-shadow` is not shipped under that id; the optional behaviour is `hairline-border-wide-shadow`.

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
