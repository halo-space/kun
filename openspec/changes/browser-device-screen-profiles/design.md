# 技术设计

## 概览

- 这次变更把 browser 画像配置收口成一个公开聚合结构：
  - `DeviceProfile`
    - `fingerprint: Option<FingerprintProfile>`
    - `screen: Option<ScreenProfile>`
- `fingerprint_preset` 会从公开 API 中删除；下载器内部不再保留旧的扁平 `FingerprintProfile` 历史模型。
- 下载器会把 `DeviceProfile` 与当前 `engine` 编译成一份归一化的 browser profile plan，用于：
  - browser context headers
  - Playwright init script
  - browser keep_alive 签名校验
- 这次不引入 session 级稳定画像，不把 browser 画像结构与 `keep_alive` 复用策略绑定在一起。

## 模块影响

- `src/request/browser.rs`
  - 新增公开 `DeviceProfile`
  - 调整公开 `FingerprintProfile`
  - 新增公开 `ScreenProfile`
  - 新增公开 `Size`
  - `browser::Config` 改为接受 `device_profile`
  - 删除 `fingerprint_preset`
  - 删除顶层 `viewport`
  - `ScreenProfile` 使用 `viewport / screen / avail` 三个 `Size` 子结构承接尺寸语义
- `src/download/browser.rs`
  - 新增内部归一化 profile plan
  - 更新 `DeviceProfile` 解析、request headers 构造、init script 构造、signature 比较逻辑
  - `screen.viewport / screen.screen / screen.avail` 会共同参与布局与 `screen.*` 相关 patch
- `examples/browser_advanced.rs`
  - 示例改成显式使用 `DeviceProfile`
- `README.md`
  - browser 配置示例与能力边界说明更新
- `docs/capabilities.md`
  - browser 能力边界说明更新
- `openspec/specs/spider-api/spec.md`
  - 增加 browser 结构化设备 / 屏幕画像 requirement

## 关键决策

- Runtime / middleware 影响：
  - 这次不影响 `Settings`、engine middleware 链或 plugin 装载机制。
  - 影响范围主要在 browser request 配置解释与 browser downloader 执行计划编译阶段。
- 对外 API 影响：
  - browser 公共配置只保留 `DeviceProfile` 这一层聚合入口。
  - `DeviceProfile` 本身可选；调用方可以只提供 `fingerprint`、只提供 `screen`，或两者都不提供。
  - `DeviceProfile.fingerprint` 负责地区 / 语言 / UA / 平台等身份画像。
  - `DeviceProfile.screen` 负责同时表达 viewport、screen 与 avail 这三组屏幕 / 布局语义；顶层 `viewport` 会删除。
  - `DeviceProfile.screen` 内部采用三个子结构：
    - `viewport: Size`
    - `screen: Size`
    - `avail: Size`
  - 当前公开 `fingerprint` 字段先不暴露 `vendor`、`max_touch_points`、`hardware_concurrency`。
  - 当前公开 `fingerprint` 字段包括 `user_agent`、`locale`、`timezone`、`accept_language`、`languages`、`platform` 与 `device_memory`。
  - `FingerprintProfile` 允许部分填写；下载器内部按 `engine` 与默认规则补齐最终执行值；用户显式填写的字段优先级更高。
- 配置分层原则：
  - profile 体系只承载“浏览器呈现出来的身份画像”。
  - `keep_alive`、`session`、`proxy`、`wait_for_selector`、`stealth_script` 等运行策略继续留在 `browser::Config` 外层。
  - 后续如果新增其它画像类能力，应继续作为 `DeviceProfile` 的子结构扩展，而不是重新回到顶层扁平字段。
- Plugin 或 DSL 影响：
  - 这次不直接扩 DSL 字段面，但 DSL 后续如果接 browser 配置，应直接复用 `DeviceProfile` 这套分层边界，而不是重新发明字段。
  - plugin 自动装载与 browser profile 结构无直接关系。

## 验证方式

- 为 `browser::Config` 新增结构化 profile builder 单元测试。
- 为下载器的 profile 编译逻辑新增测试，确认它会按 `engine` 与默认规则补齐最终执行值。
- 为 browser init script 与 headers 构造新增测试，确认：
  - `DeviceProfile.fingerprint` 影响 navigator / headers
  - `DeviceProfile.screen` 影响 viewport、`window.screen.*` 与相关 patch
- 为 `DeviceProfile.screen` 内部 `viewport / screen / avail` 的组合规则新增显式失败测试。
- 更新 README 与 `examples/browser_advanced.rs`，确保新 API 有最小可用示例。
