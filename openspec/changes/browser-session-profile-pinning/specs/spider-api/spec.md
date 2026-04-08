## ADDED Requirements

### Requirement: Browser session pins the first resolved browser profile

同一个 browser `session` 第一次解析出的完整 browser profile 必须可以被后续同 session 请求稳定复用。

#### Scenario: Session reuses the first resolved browser profile

- **GIVEN** 一个 browser 请求带有 `session`
- **AND** 这个请求显式声明了 `device_profile`，或者通过 `stealth = true` 解析出一份内置 browser profile
- **WHEN** 下载器处理这次请求
- **THEN** 下载器会把解析后的完整 browser profile 固定到这个 session
- **AND** 后续同 session 请求即使不再重复声明 `device_profile`，也会继续复用这份 profile

### Requirement: Browser session rejects conflicting browser profiles

一旦某个 browser `session` 已经固定了一份 profile，后续冲突的 engine 或 profile 不能静默覆盖。

#### Scenario: Follow-up request declares a conflicting browser profile

- **GIVEN** 某个 browser `session` 已经固定了一份完整 browser profile
- **WHEN** 后续同 session 请求又显式声明了不同的 engine 或不同的解析后 profile
- **THEN** 下载器会返回显式错误
- **AND** 不会静默覆盖已固定的 session profile
