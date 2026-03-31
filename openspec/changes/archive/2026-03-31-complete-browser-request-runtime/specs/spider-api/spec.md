# 规范增量

## MODIFIED Requirements

### Requirement: Browser Execution Must Match Playwright Runtime Boundaries

系统 MUST 让 browser request 继续使用统一 `Request` 模型，并与 Playwright 实际可执行边界保持一致。

#### Scenario: Browser request forwards method and body through the initial navigation request

- **WHEN** browser request 设置了非 `GET` method 或 request body
- **THEN** downloader 在首个目标主文档请求上把这些值覆写到 Playwright 导航请求
- **AND** 后续仍返回渲染后的最终页面 HTML 响应

#### Scenario: Browser session can reuse persisted profile state

- **WHEN** 多个 browser request 使用相同的 session id
- **THEN** downloader 复用同一个稳定的 Playwright user data dir
- **AND** cookies 与 local storage 等浏览器态数据可以随 session 继续复用

#### Scenario: Unsupported browser options fail explicitly

- **WHEN** browser request 启用了尚未接线的 `stealth` 或 `fingerprint_profile`
- **THEN** 系统返回显式 download error，而不是静默忽略这些配置
