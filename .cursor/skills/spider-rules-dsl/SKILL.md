---
name: spider-rules-dsl
description: Defines the JSON DSL, Rules loading model, and parsing schema for this spider project. Use when creating or modifying `rules`, DSL steps, `parse.fields`, `parse.links`, fetch modes, or Request/Response parsing behavior.
---

# Spider Rules DSL

## Use This Skill When

- Editing the DSL schema
- Adding or reviewing `rules = { type, options }`
- Defining step fields like `fetch`, `parse`, `route`, `output`, and `runtime`
- Adjusting `Response` parsing semantics that must match DSL behavior

## Rules Source Model

- Spider exposes exactly one rules entry: `rules`.
- `rules` is a rules-source descriptor, not raw DSL by default.
- Shape:

```json
{
  "type": "local",
  "options": {
    "path": "rules/myspider.json"
  }
}
```

- `type` decides where rules come from.
- `options` carries source-specific parameters.
- The loader must normalize the result into one standard JSON DSL document.

## P0 Rules Types

- `local`
- `inline`

P1 and later:

- `redis`
- `db`
- `http`
- `custom`

## Step Model

- Internal DSL unit is still `step`.
- A step shape is:
  - `id`
  - `impl`
  - `fetch`
  - `parse`
  - `route`
  - `output`
  - `runtime`
  - optional `MIDDLEWARES`

## Step Impl Rules

- `impl = "dsl"` means the step is rule-driven.
- `impl = "code"` means the step must explicitly declare `callback`.
- Do not allow both full DSL parsing config and `callback` on the same step.

## Fetch Rules

- `fetch.mode` must support:
  - `http`
  - `browser`

- Browser mode should reserve config for:
  - Playwright
  - Chromium
  - Google Chrome
  - stealth
  - fingerprint
  - challenge-page handling hooks

- Do not promise "bypass all protections" in docs or code comments.

## Parse Rules

- `parse` contains:
  - `fields`
  - `links`

### Field Rule Shape

```json
{
  "name": "title",
  "source": "html",
  "selector_type": "css",
  "selector": ["h1.title"],
  "attribute": "text",
  "required": true,
  "default": null,
  "multiple": false,
  "options": {}
}
```

### Link Rule Shape

```json
{
  "name": "detail_links",
  "source": "html",
  "selector_type": "css",
  "selector": [".article-list a.title"],
  "attribute": "attr:href",
  "required": false,
  "default": [],
  "multiple": true,
  "allow": ["^https://example\\.com/detail/\\d+$"],
  "deny": [],
  "to": {
    "next_step": "detail",
    "meta_patch": {}
  },
  "options": {}
}
```

## Parsing Constraints

- Supported `source` for P0:
  - `html`
  - `text`
  - `json`
  - `xml`
  - `headers`
  - `final_url`
  - `meta.xxx`

- Supported `selector_type` for P0:
  - `css`
  - `xpath`
  - `json`
  - `xml`
  - `regex`
  - `ai`

- Supported text/value extraction:
  - `text`
  - `html`
  - `value`
  - `raw`
  - `attr:<name>`
  - `group:<n>`

## Response Semantics That Must Match DSL

- Text outputs trim by default.
- `html()` does not trim by default.
- Structured values do not trim by default.
- `response.meta` must come from the originating request.
- `follow(..., meta=...)` merges or overrides current meta.

## P0 Additions

- `xml` is part of P0, not P1.
- `ai` is part of P0 and should be modeled through `selector_type = "ai"` plus `options`.
- `ocr` stays out of P0 unless explicitly requested.

## Avoid

- Reintroducing top-level `entry`
- Adding a separate `transforms` DSL block
- Auto-inferring callback names from step ids
- Adding too many parser-specific top-level fields instead of using `options`
