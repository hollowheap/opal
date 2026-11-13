# Rule format

`ui-probe` rules are data. A pack carries a version, rules, structural rules, and kind aliases. It carries no token values and no behaviour: design files supply token sets, and the binary supplies the evaluation machinery.

Version 2 is the current pack version. A version 1 pack still loads whole through the legacy reader; a pack never loads in part. A version newer than 2 fails loudly. Packs may use `extends` for a string or a list of paths; relative paths resolve from the referring file, and cycles or missing files fail the run.

## Pack shape

```json
{
  "$schema": "./rules.schema.json",
  "version": 2,
  "pack": "example",
  "aliases": {
    "button": [".btn", ".button"]
  },
  "structural": [],
  "rules": []
}
```

Both `rules` and `structural` accept the same rule shape. `rules` may also name a compiled behaviour by `id` or `name`; the name must resolve through the total resolution rule below.

Each finding carries `ruleSource` and `packSource`, the pack file the rule stands in. Unresolved names become explicit coverage gaps in `unverified` and `coverageGaps`, never silent skips.

## Rule shape

```json
{
  "name": "button-fill",
  "selector": ".button",
  "ancestors": { "any": ".toolbar" },
  "operator": "all",
  "exceptions": [".button.is-ghost"],
  "cardinality": { "min": 1, "max": 4, "region": ".toolbar" },
  "lifecycle": "first-paint",
  "properties": {
    "background-color": [
      { "present": true },
      { "notIn": "colors" }
    ]
  },
  "severity": "advisory",
  "layer": "layout"
}
```

- `name` or `id` names the rule. `selector` is required for a declarative rule. `selectors` and `match` compose several selector constraints.
- `ancestors` and `descendants` accept one constraint or a list. A constraint is `{ "any": "selector" }` or `{ "none": "selector" }`.
- `constraints` may hold extra selector constraints or property tests. `operator` is `all`, `any`, or `one` (`exactly one`).
- `exceptions` names selectors. A matched element inside an exception is removed from the rule's set before any test runs.
- `cardinality` states `min` and/or `max` over the matched set, per page or inside `region`, and fires once with `count` in evidence.
- `lifecycle` names the evidence point. `first-paint`, `domcontentloaded`, and `staged` read a staged capture; `settled` and `load` read the normal settled capture.
- `severity` and `layer` are presentation data. They carry no logic.

A selector with no property test, no cardinality, and no other constraint produces no finding. Rules beside each other only add findings; a conjunction belongs in one rule.

## Verbs

A property test is one verb and one operand. The verb set is closed.

| Verb | The test states | Operand |
| --- | --- | --- |
| `in` | the value stands in the named set or listed values | set name, literal, or list |
| `notIn` | the value stands outside the named set or listed values | set name, literal, or list |
| `equals` | the parsed value equals the operand | literal |
| `notEquals` | the parsed value does not equal the operand | literal |
| `present` | the property is not the empty value for that property | `true` |
| `absent` | the property is the empty value for that property | `true` |

The tests state the condition the rule looks for. A finding appears when the rule's `operator` over the tests is true. A rule with `equals` fires when the value equals the operand; use `notEquals` for the opposite. The old one-entry `deny` is therefore `equals`, and the old `allowFrom` is `notIn`.

`transparent` is the empty value for a fill; `none` is the empty value for a shadow. Colours compare as parsed colours, so `#ff0000`, `rgb(255, 0, 0)`, and `rgba(255, 0, 0, 1)` agree. Dimensions compare by number and unit.

A composed test is a list of verbs under one property. The rule's operator joins the tests into one verdict.

```json
{
  "background-color": [
    { "present": true },
    { "notIn": "colors" }
  ]
}
```

A multi-value property may be tested component by component:

```json
{
  "padding": {
    "components": {
      "top": { "notIn": "spacing" },
      "right": { "notIn": "spacing" },
      "bottom": { "notIn": "spacing" },
      "left": { "notIn": "spacing" }
    }
  }
}
```

An attribute test uses `attributes`, or a property name beginning with `@`, `attr:`, or `attribute:`:

```json
{
  "attributes": {
    "data-component": { "notIn": ["card", "panel", "dialog"] }
  }
}
```

## Expressions

A rule may carry `when`, `condition`, or `expr`. The expression is evaluated per matched element. `field` paths read element evidence, `page.*` reads the page evidence, and `sheet.text` reads the stylesheet text.

```json
{
  "name": "small-body-text",
  "selector": "*",
  "when": {
    "all": [
      { "equals": [{ "field": "interactive" }, false] },
      { "gte": [{ "length": { "field": "text" } }, 20] },
      { "lt": [{ "field": "fontSize" }, 12] }
    ]
  },
  "message": "body text is below 12px",
  "severity": "P2",
  "layer": "content"
}
```

Operators: `all`/`and`, `any`/`or`, `not`, `if`, `gt`, `gte`, `lt`, `lte`, `equals`, `notEquals`, `contains`, `matches`, `countMatches`, `in`, `between`, `number`, `headingLevel`, `length`, `words`, `min`, `max`, `nthLargest`, `add`, `sub`, `mul`, and `div`. Aggregate operators: `anyOf`, `allOf`, `countOf`, `sumOf`, `firstOf`, `map`, `flatMap`, `filter`, `concat`, `unique`, `uniqueCount`, `sortNumbers`, `pairAny`, `groupAny`, `groupAll`, `ancestor`, `byIndex`, `adjacentAny`, and `adjacentAll`. Value/colour operators: `get`, `contrast`, `alpha`, `chroma`, `hue`, `luminance`, `chromatic`, `dark`, `red`/`r`, `green`/`g`, `blue`/`b`, `shadowInset`, `shadowBlur`, `shadowOffset`, `shadowAlpha`, and `shadowChromatic`. Text/identity operators: `lower`, `join`, `nearestCard`, `blocks`, `lastSentence`, `containsAny`, `overshooting`, and `param`. A rule with `when` fires once per matching element whose expression is true. `scope: "page"` or `scope: "sheet"` evaluates the expression once against the page evidence or stylesheet text; page rules need no selector. `message` supplies the finding text.

## The ten functions

### 1. Constraint composition

A rule carries a list of selector or property constraints and one operator over the same matched elements. The example fires only when one element is both a skeleton and a painted card, and its radius is outside the token set.

```json
{
  "name": "skeleton-card",
  "selectors": [".skeleton", ".card"],
  "operator": "all",
  "matchOperator": "all",
  "properties": {
    "border-top-left-radius": { "notIn": "radii" }
  }
}
```

### 2. Composed property tests

One property carries a presence test, a set test, and a value denial together. The verdict is true when the fill is present, outside the colour set, and not transparent, so a transparent fill stays lawful and an off-token fill fires.

```json
{
  "name": "button-fill",
  "selector": ".button",
  "operator": "all",
  "properties": {
    "background-color": [
      { "present": true },
      { "notIn": "colors" },
      { "notEquals": "transparent" }
    ]
  }
}
```

### 3. Component tests

Each component is tested against its own set. `8px 16px 8px 16px` passes; `8px 13px 8px 16px` fires on the right component. `operator: "any"` means one bad component is enough.

```json
{
  "name": "padding-scale",
  "selector": ".card, .panel",
  "operator": "any",
  "properties": {
    "padding": {
      "components": {
        "top": { "notIn": "spacing" },
        "right": { "notIn": "spacing" },
        "bottom": { "notIn": "spacing" },
        "left": { "notIn": "spacing" }
      }
    }
  }
}
```

### 4. Cardinality

A minimum and maximum over the matched set, per page or inside a region. The finding appears once and carries the count.

```json
{
  "name": "one-primary-action",
  "selector": ".button-primary",
  "cardinality": { "min": 1, "max": 1, "region": "main" }
}
```

### 5. Value membership

The pack carries the allowed attribute values, so the test never reaches outside the pack for a catalogue. The rule fires when the value stands outside the list.

```json
{
  "name": "component-name",
  "selector": "[data-component]",
  "attributes": {
    "data-component": { "notIn": ["card", "panel", "dialog"] }
  }
}
```

### 6. Exception sets

Exceptions remove matched elements before the test. The remaining cards may not use a fill outside the colour set.

```json
{
  "name": "card-fill",
  "selector": ".card",
  "exceptions": [".card.is-exempt"],
  "properties": {
    "background-color": { "notIn": "colors" }
  }
}
```

### 7. Lifecycle evidence

A staged capture records first paint before the page settles. A rule may name that point; the normal settled evidence is not substituted for it.

```json
{
  "name": "skeleton-at-first-paint",
  "selector": ".skeleton",
  "lifecycle": "first-paint",
  "properties": {
    "display": { "equals": "block" }
  }
}
```

### 8. Provenance in output

Every finding and skip note names its pack file. This is output data, not a pack field. A finding from an extended pack names the extended file, not the top pack.

```json
{
  "check": "pad-set",
  "ruleSource": "/packs/base/padding.json",
  "packSource": "/packs/base/padding.json",
  "lifecycle": "settled"
}
```

### 9. Kind aliases

A pack declares once that two class dialects name one kind. Every selector in the pack, including ancestors and exceptions, reads through the declaration.

```json
{
  "version": 2,
  "aliases": {
    "button": ["btn", "button"]
  },
  "rules": [
    {
      "name": "button-radius",
      "selector": ".button",
      "properties": {
        "border-top-left-radius": { "notIn": "radii" }
      }
    }
  ]
}
```

### 10. Total name resolution

Every name resolves to a compiled behaviour, to a baseline through a declared alias, or to a stated coverage gap. This pack reports `legacy-contrast` through its alias and reports `missing-behaviour` as an uncovered entry.

```json
{
  "version": 2,
  "pack": "resolution",
  "ruleAliases": {
    "legacy-contrast": "contrast"
  },
  "rules": [
    { "id": "legacy-contrast" },
    { "id": "missing-behaviour" }
  ]
}
```

The run includes a note such as:

```json
{
  "check": "missing-behaviour",
  "reason": "coverage gap: no compiled behaviour, alias, or rule body",
  "coverageGap": true,
  "source": "/packs/resolution.json"
}
```

## Composition and evidence boundaries

The capture module produces evidence sets at named lifecycle points. The design module reads design files into token sets and catalogue lists. The pack module parses packs and resolves names. The evaluation module applies resolved rules to evidence sets and is the only place a finding is born. The output module renders findings with provenance.

Compiled behaviours stand behind the same resolution surface as pack rules. The decoupling ledger names the behaviours that remain compiled and the fault that keeps each one there; see `docs/rules-decoupling.md`.
