---
name: spider-middlewares-plugins
description: Defines middleware configuration, plugin loading, and override rules for this spider project. Use when editing `MIDDLEWARES`, runtime-to-middleware compilation, `plugins.toml`, plugin names, override behavior, or middleware/plugin discovery.
---

# Spider Middlewares And Plugins

## Use This Skill When

- Designing or changing middleware config
- Adding built-in or custom middleware
- Working on plugin loading or plugin manifests
- Reviewing override behavior or middleware naming

## Middleware Config Shape

The external config must stay unified under one block:

```json
{
  "MIDDLEWARES": {
    "retry_by_status": {
      "enabled": true,
      "type": "download",
      "order": 200,
      "options": {}
    }
  }
}
```

## Required Middleware Fields

- `enabled`
- `type`
- `order`
- `options`

## Middleware Type Rules

P0 supports only:

- `download`
- `spider`

### `download`

Use for:

- request interception
- response handling
- exception handling
- retry
- dedup
- cookies
- proxy
- rate limiting

### `spider`

Use for:

- response preprocessing before parse
- parse-result handling after callback/DSL execution

## Naming Rules

- Prefer short logical keys like `retry_by_status`, `cookies`, `proxy`, `custom_signature`.
- Do not make fully qualified class paths the default config key format.
- The middleware config key is the unique middleware identifier at the config layer.

## Runtime Relationship

- `runtime` is high-level execution policy.
- `MIDDLEWARES` is explicit middleware config.
- `runtime` compiles into default middleware entries.
- Explicit `MIDDLEWARES` may override those generated defaults.

Examples:

- `retry` -> `retry_by_status`, `retry_by_error`
- `dedup` -> `dedup`
- `schedule.interval_ms` -> `interval_gate`
- `schedule.rate_per_minute` -> `rate_limit`

## Middleware Method Names

Middleware interfaces must use:

- `process_request`
- `process_response`
- `process_exception`

## Plugin Loading Model

Use plugin manifest plus auto-loading.

Recommended manifest:

- `plugins.toml`

Example:

```toml
[[plugins]]
name = "custom_signature"
kind = "middleware"
entry = "myproject.plugins.custom_signature:Plugin"
override = false

[[plugins]]
name = "local"
kind = "rules"
entry = "myproject.plugins.local_rules:Plugin"
override = false
```

## Plugin Kinds

P0/P1 project vocabulary:

- `middleware`
- `rules`
- `provider`
- `storage`

Do not rename `rules` back to `rule_source`.

## Identity And Conflicts

- Plugins are uniquely identified by `(kind, name)`.
- `middleware.redis` and `rules.redis` are allowed together.

### Override Rule

This is a hard rule:

- Same-kind same-name conflicts must fail unless `override = true` is explicitly set.

Do not silently replace built-ins.

## End-User Principle

- Final config users only enable configured names.
- They do not manually register middleware.
- Project/plugin authors provide implementations through the plugin manifest.

## Validation Checklist

- Does every middleware use `enabled/type/order/options`?
- Is a new middleware using `download` or `spider` only?
- Is plugin identity checked using `(kind, name)`?
- Does same-name replacement require `override = true`?
- Is the design still "manifest + auto-load" instead of "manual registration by config users"?
