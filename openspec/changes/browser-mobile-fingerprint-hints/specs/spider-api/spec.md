## ADDED Requirements

### Requirement: Browser fingerprint profile can explicitly request mobile defaults

browser request 必须允许调用方显式声明“这是一份移动端画像”，而不是只能依赖调用方手动拼完整 UA 与 screen 参数。

#### Scenario: Browser request opts into mobile fingerprint defaults

- **GIVEN** 一个 browser request 声明了 `device_profile.fingerprint.mobile = true`
- **WHEN** downloader 解析这次请求
- **THEN** downloader 会按当前 `engine` 选择移动端默认 browser fingerprint
- **AND** 如果调用方没有显式提供 screen 配置，也会切到移动端默认 viewport / screen

### Requirement: Browser mobile hint must propagate into Chromium userAgentData

Chromium 路线下，移动端画像不能只停留在 user agent 字符串层面。

#### Scenario: Chromium stealth bootstrap exposes mobile userAgentData

- **GIVEN** 一个 Chromium browser request 启用了 `stealth = true`
- **AND** 其解析后的 browser fingerprint 是移动端画像
- **WHEN** downloader 构建 init script
- **THEN** `navigator.userAgentData.mobile` 会反映这份移动端画像
