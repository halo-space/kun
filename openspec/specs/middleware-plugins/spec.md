# Middleware 与 Plugins 规范

## 目标

统一中间件配置、内建扩展点和 `plugins.toml` 清单语义，让扩展行为在 Engine 层可组合、可验证。

### Requirement: Middleware 配置使用统一结构

库必须使用 `enabled`、`type`、`order` 与 `options` 表示 middleware 配置。

#### Scenario: Middleware 配置声明 download 或 spider 类型

- Given middleware 配置来自 settings、runtime 或 DSL
- When 配置被校验
- Then middleware 类型只能是 `download` 或 `spider`

#### Scenario: Middleware 顺序保持显式

- Given 同时启用了多个 middleware 项
- When 构建中间件链
- Then 每个 middleware 都根据自己的 `order` 决定执行位置

### Requirement: Engine 支持内建与自定义 middleware

库必须允许 middleware 既可以直接以实例形式提供，也可以以注册工厂形式提供。

#### Scenario: 直接注册 middleware 会影响引擎链路

- Given 调用了 `Engine::add_middleware()`
- When 引擎准备 middleware 链
- Then 提供的实例会参与 request 与 response 处理

#### Scenario: 已注册的 middleware 工厂支撑具名配置项

- Given `Engine::register_middleware()` 以某个 key 注册了工厂
- When settings 或 DSL 引用了这个 key
- Then 引擎根据提供的 options 构建 middleware 实例

### Requirement: Built-in Network Middleware Has Concrete Runtime Behavior

库必须让内建 `cookies` 与 `proxy` middleware 具备真实、可验证的运行时语义，而不是只保留注册入口。

#### Scenario: Cookies middleware normalizes request cookie semantics

- Given 启用了 `cookies` middleware，且请求包含 `Cookie` header 或 middleware 配置了默认 session
- When download middleware 链处理该请求
- Then middleware 会把原始 `Cookie` header 归一到 request cookies 语义，并在缺少显式 session 时应用配置的 session 标识

#### Scenario: Proxy middleware selects request proxy before download

- Given 启用了 `proxy` middleware，且请求本身未显式声明 proxy
- When download middleware 链处理该请求
- Then middleware 会按固定 proxy 或 round-robin proxy pool 为该请求选择代理；失败重试仍由现有 retry 能力负责

### Requirement: Plugin manifest 来自 plugins.toml

库必须从 `plugins.toml` 加载 plugin manifest，并以 `(kind, name)` 作为 key 存入 registry。
当前 Engine 只支持 `kind = "middleware"` 的插件自动装载；其它已知 kind 必须和稳定的 engine 组件 owner 对齐，但不代表运行时已具备对应装配能力。

#### Scenario: middleware manifest 必须有对应的已注册工厂

- Given 某个 plugin manifest 声明 `kind = "middleware"`
- When `Engine::load_plugins()` 校验 registry
- Then 引擎要求存在同名的 middleware 工厂

#### Scenario: engine 对未自动装载的组件 plugin kind 显式失败

- Given registry 中存在 `kind = "store"`、`"scheduler"`、`"dedup"`、`"robots"`、`"http"` 或 `"browser"` 的 manifest
- When `Engine::load_plugins()` 校验 registry
- Then 引擎返回显式错误，说明当前只支持 `middleware` kind 自动装载

#### Scenario: 不同 plugin kind 可以复用同名

- Given 两个 manifest 使用相同的 `name`
- When 它们的 `kind` 不同
- Then 两个 manifest 可以在 registry 中共存

### Requirement: Plugin 替换必须显式声明 override

库必须拒绝重复的 `(kind, name)` 注册，除非新 manifest 明确设置了 `override = true`。

#### Scenario: 重复 plugin 且未声明 override 时失败

- Given plugin registry 已经包含某个 `(kind, name)` 的 manifest
- When 又注册一个相同 `(kind, name)` 但未声明 override 的 manifest
- Then registry 返回 plugin conflict error

#### Scenario: 重复 plugin 且声明 override 时成功

- Given plugin registry 已经包含某个 `(kind, name)` 的 manifest
- When 又注册一个相同 `(kind, name)` 且声明了 `override = true` 的 manifest
- Then 新 manifest 替换旧 manifest
