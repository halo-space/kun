---
name: spider-rust-implementation
description: Guides Rust implementation for this spider project using Rust 2024 conventions. Use when planning or writing Rust modules, traits, registries, plugin loaders, scheduler, engine, download, or P0/P1 implementation boundaries.
---

# Spider Rust Implementation

## Use This Skill When

- Translating the design into Rust
- Choosing module boundaries or traits
- Building the P0 implementation plan
- Reviewing code for architecture drift from the agreed design

## Language And Layout Rules

- Implementation target is Rust.
- Follow Rust 2024 conventions.
- Current Rust toolchain target is `1.94.0`.
- Do not use `mod.rs`.
- Prefer `foo.rs` and `foo/` submodule layouts instead.
- Inside a module directory, avoid repeating the module or file name in type and function names.
- Prefer concise names like `request::browser::Config` and `request::browser::Engine`, not `BrowserRequestConfig` or other redundant path-prefixed names.
- Apply the same rule in `rules/`, e.g. `rules::source::Source`, `rules::local::Source`, and `rules::inline::Source`.
- For parser query APIs, prefer `one` and `all` over `get` and `getall`.
- Prefer asynchronous implementations whenever the code path may perform I/O, waiting, scheduling, downloading, browser control, or pipeline/output work.
- Do not use the `async_trait` crate.
- Use Rust's native async support directly, and prefer designs that work with native async traits and futures on Rust `1.94.0`.

## P0 Scope

P0 must focus on the minimal working kernel:

- `Spider`
- default `parse`
- `Request` / `Response`
- `Request.mode = http | browser`
- `css/xpath/json/xml/regex/ai/follow`
- `rules` with `local` and `inline`
- DSL `steps`
- `runtime` with `schedule/retry/dedup`
- `MIDDLEWARES`
- built-in middleware
- `plugins.toml` loading
- memory scheduler
- HTTP download
- browser download minimal path
- engine main loop

Do not let P0 absorb OCR or advanced plugin systems.

## Recommended Module Boundaries

- `spider`
- `request`
- `response`
- `parser`
- `rules`
- `runtime`
- `middleware`
- `plugins`
- `scheduler`
- `download`
- `engine`
- `item`
- `error`

## Module Responsibilities

### `spider`

- Spider abstraction
- default `parse`
- callback lookup

### `request`

- request object
- request mode (`http` or `browser`)
- callback target
- meta
- dont_filter

### `response`

- response fields
- text/body/meta/request
- `follow`
- parse helpers delegating to parser layer

### `parser`

P0 implementations:

- css
- xpath
- json
- xml
- regex
- ai

### `rules`

- rules config
- local/inline loading
- schema structs
- validation
- compile to internal executable definitions

### `runtime`

- runtime structs
- compile `retry/dedup/schedule` into default middleware config

### `middleware`

- traits
- chain execution
- built-in middleware

### `plugins`

- parse project-level `plugins.toml`, not an internal library self-registration manifest
- `(kind, name)` registry
- override checks

### `scheduler`

- scheduler trait
- memory scheduler implementation

### `download`

- download trait
- HTTP implementation
- browser implementation placeholder or minimal adapter

### `engine`

- main request execution loop
- middleware invocation
- callback or DSL dispatch
- retry / ack / nack decisions
- Transitional entrypoints such as `execute_spider_once()` are acceptable while iterating, but they must converge later into a clearer `run()` shape or be folded into the final engine main loop instead of becoming permanent public architecture.

## Browser Planning Rule

The design must preserve `fetch.mode = http | browser`.

For browser mode, reserve config and abstractions for:

- Playwright
- Chromium
- Google Chrome
- stealth
- fingerprint
- challenge-page handling hooks

Browser is in P0, but keep it minimal:

- establish `Request.mode = http | browser`
- support a minimal Playwright path
- reserve config for Chromium / Google Chrome / stealth / fingerprint
- avoid overbuilding anti-bot capabilities in the first implementation pass

## Rust Interface Direction

Keep interfaces narrow and composable:

- scheduler trait
- download trait
- middleware trait with `process_request / process_response / process_exception`
- rules loader abstraction
- plugin registry keyed by `(kind, name)`

Avoid over-abstracting items, providers, or event systems in P0.

## Review Checklist

- Is the code still aligned with P0 instead of drifting into P1/P2?
- Are modules split by responsibility, not by premature micro-abstraction?
- Is Rust 2024 layout respected without `mod.rs`?
- Are plugin conflicts explicit and checked?
- Is browser support preserved in design but not overbuilt too early?
