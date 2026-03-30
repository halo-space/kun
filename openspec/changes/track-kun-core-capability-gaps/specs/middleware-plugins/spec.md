# 规范增量

## ADDED Requirements

### Requirement: Built-in Network Middleware Must Have Concrete Behavior

系统必须让内建 `cookies` 与 `proxy` middleware 具备真实、可验证的运行时行为，而不是仅保留注册入口。

#### Scenario: Cookies middleware applies request/session behavior

- **WHEN** 请求启用了 cookies 或 session 相关能力
- **THEN** middleware 对请求和后续链路产生真实影响

#### Scenario: Cookies middleware can assign a stable default session

- **WHEN** middleware 配置了默认 session，而请求没有显式 session
- **THEN** 请求会进入该默认 session 语义，而不是退化成一次性 cookies 占位

#### Scenario: Proxy middleware applies proxy routing behavior

- **WHEN** 请求启用了 proxy 相关能力
- **THEN** middleware 对请求路由或代理选择产生真实影响

#### Scenario: Proxy pool boundary is explicit

- **WHEN** middleware 配置了 proxy pool
- **THEN** 系统至少支持请求时的固定代理或 round-robin 选择，并明确失败健康检查仍由现有 retry 链路负责

## MODIFIED Requirements

### Requirement: Engine 支持内建与自定义 middleware

库必须允许 middleware 既可以直接以实例形式提供，也可以以注册工厂形式提供；对于内建 middleware，公开名称必须对应真实实现而不是空壳占位。

#### Scenario: Built-in middleware keys map to concrete behavior

- **WHEN** settings、runtime 或 DSL 使用内建 middleware key
- **THEN** 引擎加载的 middleware 具备可验证的运行时语义

### Requirement: Plugin manifest 来自 plugins.toml

库必须从 `plugins.toml` 加载 plugin manifest，并明确 engine 当前真正支持的 plugin kind 边界。

#### Scenario: Unsupported plugin kind is explicit

- **WHEN** registry 中存在 engine 尚未支持的 plugin kind
- **THEN** 系统给出显式、稳定的边界说明，而不是留下隐含能力错觉

#### Scenario: Engine only auto-loads middleware plugins

- **WHEN** plugin manifest 使用 `rules`、`provider` 或 `storage` kind
- **THEN** manifest 可以进入 registry 命名空间
- **AND** `Engine::load_plugins()` 不会把它们当成已支持的运行时扩展点
