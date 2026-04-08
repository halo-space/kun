# 规范增量

## ADDED Requirements

### Requirement: Browser Keep Alive Exposes Granular Control Fields

系统 MUST 在当前 `keep_alive` 与 `keep_alive_scope` 之外，为 browser request 提供更细粒度的 keep_alive 控制字段。

#### Scenario: Browser request can declare an explicit keep alive key

- **WHEN** 调用方为 browser request 显式声明 `keep_alive_key`
- **THEN** downloader 在当前 session 与 `keep_alive_scope` 计算出的分桶基础上，再叠加这段业务 key
- **AND** `keep_alive_key` 不会替代 session 本身

#### Scenario: Browser request can limit keep alive idle lifetime

- **WHEN** 调用方为 browser request 显式声明 `keep_alive_max_idle`
- **AND** 该值大于 `0`
- **THEN** downloader 只在该 idle 窗口内继续复用对应 keep_alive
- **AND** 超出窗口后会重建新的 keep_alive entry，而不是继续复用旧 entry

#### Scenario: Non-positive keep alive idle lifetime is rejected explicitly

- **WHEN** 调用方为 browser request 显式声明 `keep_alive_max_idle`
- **AND** 该值小于或等于 `0`
- **THEN** 系统返回显式配置错误
- **AND** 不会把该值静默解释成“无限复用”或“不启用 idle 控制”

#### Scenario: Browser request can limit keep alive use count

- **WHEN** 调用方为 browser request 显式声明 `keep_alive_max_uses`
- **THEN** downloader 只在该使用次数上限内继续复用对应 keep_alive
- **AND** 达到上限后会重建新的 keep_alive entry

#### Scenario: Browser request can declare keep alive behavior on browser errors

- **WHEN** 调用方为 browser request 显式声明 `keep_alive_on_error = keep | reset`
- **THEN** downloader 在 browser 级错误发生后，按声明决定保留或重置当前 keep_alive

## MODIFIED Requirements

### Requirement: Browser Execution Must Match Playwright Runtime Boundaries

系统 MUST 让 browser request 的配置语义与实际 downloader 实现保持一致，并把未实现能力收敛为显式失败。

#### Scenario: Browser keep alive scope and explicit key compose into one stable bucket

- **WHEN** browser request 同时声明 `keep_alive_scope` 与 `keep_alive_key`
- **THEN** downloader 使用统一、稳定的 bucket 规则管理 keep_alive
- **AND** 不会把 session、scope 与业务 key 分散成互相独立的复用语义
