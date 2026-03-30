# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [0.0.1] - 2026-03-18

### Added

- **Spider trait** — name, start_urls, allowed_domains, parse, callback dispatch, DSL/code 双模式执行。
- **`cb!` / `spider_callbacks!` 宏** — 回调分发，消除手写 match 样板。
- **Request** — url, method, headers, body, meta, callback, dont_filter, runtime, mode (http/browser), with_cookie/with_cookies。
- **Response** — url, status, headers, body, text, meta, request, flags, certificate, ip_address, protocol。
- **解析器** — CSS, XPath, JSON, XML, Regex, AI, Feed，统一 `one()`/`all()` 查询 API。
- **Rules DSL** — local / inline 规则源，step 模型 (id, impl, fetch, parse, route, output, runtime, MIDDLEWARES)，parse.fields + parse.links。
- **Runtime** — schedule / retry / dedup 配置，编译为默认中间件。
- **MIDDLEWARES** — enabled / type / order / options，merge / override 语义。
- **内建中间件** — retry_by_status, retry_by_error, dedup, interval_gate, rate_limit, cookies, proxy。
- **自定义中间件** — `add_middleware()` 直接注册实例，`register_middleware()` 工厂注册。
- **Settings** — 引擎级集中配置：download_delay, concurrent_requests, retry_times, dedup_enabled, idle_timeout, with_middleware。
- **Pipeline trait** — open / process / close，`()` 空实现，`(A, B)` 元组组合。
- **Engine** — 持久 run() 循环，队列空不退出，仅 stop() 或 Ctrl+C 退出。
- **ShutdownHandle** — 可 Clone 跨线程停止句柄。
- **Memory Scheduler** — enqueue, lease, ack, nack, has_pending，延时任务支持。
- **HTTP Downloader** — reqwest + rustls-tls。
- **Browser Downloader** — headless_chrome（feature-gated），无 feature 时 stub 响应。
- **插件系统** — plugins.toml 加载，(kind, name) 注册，override 冲突检查。
- **域名过滤** — allowed_domains，引擎入队时过滤。

### Examples

- `quotes_code` — Scrapy 风格递归爬虫，parse/parse_author 回调，LogPipeline，Ctrl+C 退出。
- `quotes_dsl` — DSL 驱动爬虫，JSON 规则文件。
- `custom_middleware` — 自定义中间件两种注册方式演示。
