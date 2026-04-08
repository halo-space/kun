# 规范增量

## ADDED Requirements

### Requirement: Browser Structured Profiles Use A Device Aggregation Model

系统 MUST 把 browser 结构化画像配置收口成公开 `DeviceProfile` 聚合模型，而不是继续同时保留 preset、顶层 viewport 与旧扁平 profile 多套入口。

#### Scenario: Browser request can declare an explicit device profile

- **WHEN** 调用方为 browser request 显式声明 `DeviceProfile`
- **THEN** downloader 使用 `DeviceProfile.fingerprint` 与 `DeviceProfile.screen` 构造 browser 执行画像
- **AND** `DeviceProfile.fingerprint` 负责 browser headers 与对应的 navigator patch
- **AND** `DeviceProfile.screen` 负责 viewport、`screen.*` 与相关 patch
- **AND** 当前公开模型不再要求调用方在 `Config` 顶层分别传 fingerprint 与 screen

#### Scenario: Device profile and its sub-profiles can be partially declared

- **WHEN** 调用方为 browser request 只声明 `DeviceProfile.fingerprint`、只声明 `DeviceProfile.screen`，或完全不声明 `DeviceProfile`
- **THEN** 这些写法都属于合法配置
- **AND** downloader 会把缺失部分按 `engine` 与稳定默认规则补齐

#### Scenario: Browser request can declare an explicit fingerprint sub-profile

- **WHEN** 调用方为 browser request 显式声明 `DeviceProfile.fingerprint`
- **THEN** downloader 使用这组画像字段构造 browser headers 与对应的 navigator patch
- **AND** 当前公开字段至少包括 `user_agent`、`locale`、`timezone`、`accept_language`、`languages`、`platform` 与 `device_memory`
- **AND** `vendor`、`max_touch_points` 与 `hardware_concurrency` 不再作为当前公开 profile 字段暴露

#### Scenario: Browser request can declare an explicit screen sub-profile

- **WHEN** 调用方为 browser request 显式声明 `DeviceProfile.screen`
- **THEN** downloader 使用这组屏幕画像字段构造 viewport、`screen.*` 与相关 patch
- **AND** `DeviceProfile.screen` 同时表达 viewport、screen 与 avail 这组屏幕 / 布局语义
- **AND** `DeviceProfile.screen` 通过 `viewport / screen / avail` 三个子结构表达这三组尺寸

#### Scenario: Browser request no longer exposes a separate preset entry

- **WHEN** 调用方配置 browser request
- **THEN** 公开 API 只暴露 `DeviceProfile`
- **AND** 不再继续暴露单独的 `fingerprint_preset` 入口

#### Scenario: Runtime controls remain outside device profile

- **WHEN** 调用方配置 browser request
- **THEN** `keep_alive`、`session`、`proxy`、`wait_for_selector` 与 `stealth_script` 继续保留在 `browser::Config` 外层
- **AND** 这些运行策略不会被混入 `DeviceProfile`

#### Scenario: Inconsistent screen values fail explicitly

- **WHEN** 调用方提供 `DeviceProfile.screen`
- **AND** 其中 viewport、screen 与 avail 三组尺寸表达出明显冲突的屏幕语义
- **THEN** 系统返回显式 download error
- **AND** 不会静默接受这组不一致画像

## MODIFIED Requirements

### Requirement: Browser Execution Must Match Playwright Runtime Boundaries

系统 MUST 让 browser request 的配置语义与实际 downloader 实现保持一致，并把未实现能力收敛为显式失败。

#### Scenario: Browser request applies fingerprint and screen profiles through one normalized plan

- **WHEN** browser request 使用显式 `DeviceProfile`
- **THEN** downloader 先把这些输入与当前 `engine` 编译成一份统一的归一化 profile plan
- **AND** `FingerprintProfile` 允许部分填写，而不是要求调用方一次提供完整最终执行值
- **AND** 再把这份 plan 用于 Playwright headers、init script 与 runtime signature，而不是让多套 profile 输入在不同执行路径里各自生效
