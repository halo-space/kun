# 规范增量

## MODIFIED Requirements

### Requirement: Request 支持 HTTP 与 browser 执行模式

库必须通过统一的 `Request` 类型描述外发工作，并支持 `mode = http | browser`。

#### Scenario: browser 请求走 Playwright 兼容引擎配置

- **Given** 一个 browser 请求声明 `driver = playwright`
- **And** `engine` 为 `chromium`、`firefox` 或 `webkit`
- **When** browser downloader 执行该请求
- **Then** 它使用对应的 Playwright 浏览器引擎
- **And** `headless`、`viewport`、`wait_for`、timeout、headers、proxy 与已支持的 fingerprint profile 配置会被应用

#### Scenario: browser 请求沿用统一 Request 的 method 与 body

- **Given** 一个 browser 请求设置了非 `GET` method 或 request body
- **When** browser downloader 执行该请求
- **Then** downloader 会把这些值覆写到首个目标主文档导航请求
- **And** 最终仍返回渲染后的页面响应

#### Scenario: 未启用 browser feature 时显式失败

- **Given** 一个 browser 请求
- **When** 当前构建未启用 `browser` feature
- **Then** 执行返回显式 download error
- **And** 框架不能返回受限 stub response 冒充成功执行

#### Scenario: Browser request enables supported stealth bootstrap

- **Given** 一个 browser 请求启用了 `stealth = true`
- **When** browser downloader 执行该请求
- **Then** downloader 会在 Playwright browser context 上注入最小 stealth bootstrap
- **And** 请求不会因为 `stealth = true` 而直接被视为未实现

#### Scenario: Browser request enables supported fingerprint profile

- **Given** 一个 browser 请求声明了受支持的 `fingerprint_profile`
- **When** browser downloader 执行该请求
- **Then** downloader 会把该 profile 映射到确定性的 Playwright context options 与 bootstrap 脚本
- **And** 这些设置与已有 `headers`、`proxy`、`timeout`、cookies、session 语义共同生效

#### Scenario: Unsupported fingerprint profile still fails explicitly

- **Given** 一个 browser 请求声明了当前未支持的 `fingerprint_profile`
- **When** browser downloader 尝试执行它
- **Then** 执行返回显式 download error
- **And** 框架不能静默忽略该 profile 名称
