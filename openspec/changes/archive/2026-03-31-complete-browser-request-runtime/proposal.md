# 变更提案

## 为什么做

- 当前 `browser` 下载链路已经能打开页面、应用 `headless / viewport / wait_for / timeout / headers / proxy`，并且已经补上了最小 `session` 复用能力。
- 但它还没有真正接住统一 `Request` 模型里的 `method` 与 `body`。这会让 `http` 与 `browser` 两种下载形式在同一套请求模型下出现语义裂缝。
- 用户侧对 `browser` 的理解是“同一个请求能力模型，不同的执行方式”；因此本轮需要把这部分补成正式的 OpenSpec change，避免遗漏边界与验证项。

## 范围

- 会影响哪些 capability specs：
  - `openspec/specs/spider-api/spec.md`
- 会影响哪些模块 / 文档：
  - `src/download/browser.rs`
  - `README.md`
  - `TODO.md`
- 预期带来哪些用户可见结果：
  - browser request 可以沿用统一 `Request` 上的 `method` 与 `body`
  - 相同 `session` id 的 browser request 继续复用同一个 Playwright user data dir
  - `stealth`、`fingerprint_profile` 等仍未实现项继续显式报错，不做静默忽略

## 非目标

- 这次 change 不处理 DSL / `rules` 接线。
- 这次 change 不实现 `stealth` 或 `fingerprint_profile`。
- 这次 change 不扩展 HTML XPath、OCR 或其它 parser 能力。
- 这次 change 不把 browser 单独做成另一套 runtime 或 backend 抽象。

## 风险

- Playwright 的导航 API 本身以 `goto` 为核心，非 `GET`/带 body 的导航需要依赖 route interception 覆写首个主文档请求；如果处理过宽，可能误伤页面内的资源请求。
- browser `session` 当前只完成“稳定 user data dir 复用”，并没有专门处理同一 session 的并发协调；这一点需要继续保留为已知边界。
