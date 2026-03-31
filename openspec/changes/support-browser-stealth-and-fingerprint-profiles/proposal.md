# 变更提案

## 为什么做

- 当前 `halo-spider` 的 browser 下载链路已经接住了统一 `Request` 上的 `method`、`body`、`headers`、`timeout`、`proxy`、cookies 与 `session`，但 `BrowserConfig::stealth` 和 `BrowserConfig::fingerprint_profile` 仍然是显式报错。
- 这会导致 browser API 在“字段已经公开、用户已经能配置、但运行时完全不能消费”的状态停留过久。对于要抓取动态站点的用户来说，这两个字段正好又是最直觉会尝试的能力入口。
- 本地 `playwright-rs` 已经暴露 `BrowserContextOptions.user_agent / locale / timezone_id / extra_http_headers` 与 `BrowserContext::add_init_script()`，说明这次不是只能写占位设计，而是可以补一版最小、可验证、受约束的真实能力。

## 范围

- 会影响哪些 capability specs：
  - `openspec/specs/spider-api/spec.md`
- 会影响哪些模块 / 示例：
  - `src/request/browser.rs`
  - `src/download/browser.rs`
  - browser 相关测试
  - `README.md`
  - `TODO.md`
- 预期带来哪些用户可见结果：
  - `fingerprint_profile` 不再只是占位字段，而是支持一组内置 profile
  - `stealth = true` 不再直接报未实现，而是在 Playwright 路线上注入最小 stealth bootstrap
  - 对于不支持的 profile 名称或当前未覆盖的高级 fingerprint 能力，继续显式报错，而不是静默忽略

## 非目标

- 这次不实现通用插件化 stealth 平台，也不引入外部 stealth 脚本生态。
- 这次不承诺“绕过所有反爬检测”，只提供最小、明确、可测试的 browser fingerprint 与 stealth bootstrap。
- 这次不扩展公开 `Request` cookies 结构到 domain/path/expires/same-site，也不处理更完整的 browser context/page 复用池。
- 这次不把 DSL 一并扩面；仍然优先补代码爬虫与共享底层能力。

## 风险

- 是否存在兼容性或迁移风险：
  - 有轻微运行时行为变化风险。此前 `stealth` / `fingerprint_profile` 会直接失败；变更后，部分配置会进入真实执行路径，因此需要把支持边界写清楚并补测试。
- 是否存在 runtime、middleware 或 plugin 相关风险：
  - browser context 初始化顺序、route interception 与 init script 注入可能互相影响，需要保持现有 request override 与 session 复用语义不被破坏。
  - 不同浏览器引擎对 locale、timezone、UA 与 JS 环境补丁的支持度可能不同，因此这次需要把 profile 定义收敛到最小集合，并对 unsupported case 保持显式失败。
