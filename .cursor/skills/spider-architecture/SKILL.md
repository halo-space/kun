---
name: spider-architecture
description: Defines the spider library's external architecture and naming. Use when designing or updating Spider, parse callbacks, Request/Response APIs, Rules, Runtime, or overall framework boundaries for this project.
---

# Spider Architecture

## Use This Skill When

- Designing or refactoring the library's top-level API
- Deciding how `Spider`, `parse`, callbacks, `rules`, `runtime`, and `MIDDLEWARES` fit together
- Reviewing docs or code for architecture drift from the agreed design

## Core Rules

- The external primary entry point is `Spider`.
- The only default code entry is `parse`.
- Other callback names are fully user-defined.
- `start_urls` defaults to `parse`.
- `response.follow(url)` defaults to `parse`.
- Code-mode callback arguments should prefer function references, not strings.
- `Crawler`, `Registry`, and `TaskConfig` are concept/internal concepts, not primary user-facing APIs.

## Dual Mode Model

- Code callbacks and JSON DSL rules must coexist in the same spider.
- Favor DSL for stable, structured pages.
- Favor code callbacks for login, anti-bot, signing, CAPTCHA, and complex detail pages.
- Do not split the system into two unrelated engines.

## Request/Response API Shape

- `Response` must expose `url`, `status`, `headers`, `body`, `text`, `meta`, `request`, `flags`, `certificate`, `ip_address`, and `protocol`.
- `Request` must expose `url`, `method`, `headers`, `body`, `meta`, `callback`, `dont_filter`, and optional local `runtime`.
- Text-returning helpers trim by default.
- `html()` and structured values do not trim by default.

## Default Routing Rules

- No top-level DSL `entry` block.
- The default entry name is always `parse`.
- If DSL should take over the default entry, a DSL step with `id = "parse"` must exist.
- Do not auto-guess callback names from step ids.

## Keep These Boundaries

- `callback`: who handles next
- `middleware`: how request/response/exception are intercepted
- `event`: what happened and who observes it

## Naming Rules

- Event names do not use `on_`; use names like `request_started`.
- Middleware methods use verb names: `process_request`, `process_response`, `process_exception`.

## Review Checklist

- Is `parse` still the only default entry?
- Is any new concept being exposed to users when it should stay internal?
- Are code and DSL still sharing one execution model?
- Is any design trying to reintroduce top-level DSL `entry`?
- Are callback names explicit rather than inferred?
