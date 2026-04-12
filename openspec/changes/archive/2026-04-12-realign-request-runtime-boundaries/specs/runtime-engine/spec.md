# 规范增量

## ADDED Requirements

### Requirement: Request Execution Policies Run On Explicit Lifecycle Boundaries

系统 MUST 把 request-scoped execution policy 放在明确的生命周期边界执行，而不是继续混成 engine 全局组件或模糊的 `schedule` 语义。

#### Scenario: Request execution policy order is deterministic

- **WHEN** 一条新 request 被发现，并最终完成一次 download attempt
- **THEN** 系统按固定顺序执行这些边界：
- **AND** 先在 admission 阶段执行 allowed-domain 检查与 request dedup
- **AND** request 进入 scheduler 并被 claim 后，再在 download attempt 前执行 download-before middleware
- **AND** download 返回错误或 retryable response 后，才执行 retry middleware

#### Scenario: Admission policies run before a request enters the scheduler

- **WHEN** start request、follow request、manual enqueue request 或其它新发现 request 准备进入 scheduler
- **THEN** 系统先执行 request admission 边界上的策略
- **AND** 这条边界至少明确包含 allowed-domain 检查与 request dedup

#### Scenario: Download-before middleware runs before each download attempt

- **WHEN** 某条 task 已被 claim，并即将发起一次 download attempt
- **THEN** 系统先执行当前 request 的 effective download-before middleware
- **AND** 如果该 attempt 需要退避，任务会带 delay 回到 scheduler，而不是阻塞 worker

#### Scenario: Delay is distinct from retry

- **WHEN** 某条 request 在 download 前因为 `concurrency`、`interval`、`rate_limit`、`auto_throttle`、crawl-delay 或其它同类原因被延迟
- **THEN** 系统把它视为一次 `Delay`
- **AND** 这次事件不会增加 retry 次数
- **AND** 它会回到 scheduler delayed bucket，而不是走 retry 计数路径

#### Scenario: Retry runs after a failed download attempt or retryable response

- **WHEN** download 返回错误，或 response 命中 retry policy
- **THEN** 系统在这次 attempt 之后执行 request 的 effective retry policy
- **AND** 如果命中 retry，下一次 attempt 继续沿用同一条 request 的有效运行时上下文
- **AND** retry 不会倒置到 download-before middleware 或 dedup 之前执行

### Requirement: Middleware Lifecycle Uses Object-Scoped Flow And Context Types

系统 MUST 按对象生命周期组织 middleware 的 flow 与 context，而不是继续让所有 hook 共享一个总 `Flow` 和一个大一统上下文。

#### Scenario: Middleware flow families are scoped by lifecycle object

- **WHEN** 框架为 middleware 暴露控制流类型
- **THEN** 它至少区分 `enqueue`、`download`、`parse` 与 `item` 四类生命周期对象
- **AND** admission hook 只使用 enqueue flow
- **AND** download 相关 hook 共享 download flow
- **AND** parse 相关 hook 共享 parse flow
- **AND** item 相关 hook 共享 item flow

#### Scenario: Observational hooks do not pretend to be control-flow hooks

- **WHEN** 某个 hook 只是做收尾、副作用、日志、状态释放或埋点
- **THEN** 它返回普通结果而不是 flow
- **AND** 系统不会要求调用方为这类 hook 返回无意义的 `Continue`

#### Scenario: Contexts are object-scoped and event payload is passed separately

- **WHEN** middleware 在 download、parse 或 item 生命周期中执行
- **THEN** 框架按对象生命周期提供对应 context
- **AND** `response`、`error` 这类事件数据按需作为 hook 参数传入
- **AND** 框架不会继续把 request、response、error、item 强行混进一个充满可选字段的统一 context

### Requirement: Request Dedup Is A Request-Scoped Admission Policy

系统 MUST 把 dedup 建模成 request-scoped admission policy，而不是继续默认把所有 request 都挂在 engine 全局 dedup 组件上。

#### Scenario: Requests without dedup policy skip request dedup

- **WHEN** 某条 request 没有声明 dedup policy，且没有命中默认 request runtime dedup
- **THEN** 系统不会对它执行 request dedup
- **AND** 该 request 仍然可以继续走 allowed-domain 检查和后续调度

#### Scenario: Different requests can use different dedup policies in one spider run

- **WHEN** 同一轮 spider 运行里，列表页 request、详情页 request 或其它请求声明了不同 dedup policy
- **THEN** 系统按各自 request 的 effective dedup policy 做 admission 决策
- **AND** 不要求它们共享同一个 engine 全局 dedup 规则

#### Scenario: Internal retries are not rejected as fresh duplicates

- **WHEN** 某条 request 已经进入 retry 路径
- **THEN** 系统不会把这次内部 retry 当成一条新的外部发现 request 再次按原始 dedup policy 拒绝
- **AND** retry 路径的 admission 语义保持显式、可测试

### Requirement: Download-Before Middleware Uses Explicit Shared Buckets

系统 MUST 让 download-before middleware 基于显式 bucket 生效，而不是继续依赖 middleware instance 的匿名本地状态。

#### Scenario: Requests in the same bucket share limit state

- **WHEN** 两条 request 解析到同一个 limit bucket
- **THEN** 它们共享该 bucket 的 `concurrency`、`interval`、`rate_limit` 或 `auto_throttle` 状态

#### Scenario: Requests in different buckets do not accidentally share state

- **WHEN** 两条 request 解析到不同的 limit bucket
- **THEN** 它们的 limit state 相互隔离
- **AND** 系统不会因为复用同一个 step chain 或 middleware instance 就错误串桶

### Requirement: Engine Process Controls Stay Distinct From Request Policies

系统 MUST 把 engine worker/process 级控制，与 request-scoped execution policy 区分开。

#### Scenario: Global worker concurrency remains an engine throughput control

- **WHEN** 调用方配置 engine 的全局并发或 per-domain 并发
- **THEN** 这些值继续控制 worker 能同时 claim / 执行多少任务
- **AND** 它们不等价于某条 request 自己的 download-before middleware policy

#### Scenario: Download-before middleware and engine throughput controls can coexist

- **WHEN** engine 配了全局 worker 并发，同时某些 request 额外声明了更严格的 download-before middleware
- **THEN** 系统同时尊重这两层边界
- **AND** 不会把 request 级 download-before middleware 退化成 engine 全局吞吐开关

### Requirement: Engine Processes Spider Callback Outputs As Request-Scoped Work

系统 MUST 把 spider callback 返回的 request / item 收口回 engine 的固定执行边界，而不是让调用方自己推断后续执行顺序。

#### Scenario: Callback output requests re-enter admission after callback returns

- **WHEN** spider callback 通过 `Output { items, requests }` 返回了一条新的 request
- **THEN** engine 在 callback 返回后统一接管这条 request
- **AND** 它会重新进入 admission 边界，再按自己的 effective request runtime 执行 dedup / download-before middleware / retry middleware

#### Scenario: Callback output handling does not bypass runtime boundaries

- **WHEN** spider callback 返回 `Output { items, requests }`
- **THEN** 这些输出只表达“产出下一批工作”
- **AND** 它们不会绕过 scheduler、admission、download attempt 或 store/pipeline 这些既定 engine 边界

### Requirement: Request Middleware Resolution Uses Global, Step, And Request Layers

系统 MUST 以 engine global、current step default、current request override 三层来解析 request middleware，并且不允许 step 间或父子 request 间发生隐式覆盖继承。

#### Scenario: Request override wins over step and engine defaults

- **WHEN** 某条 request 显式给某个 middleware 写入 `Use(config)` 或 `Skip`
- **THEN** 该 request 的显式覆盖优先于当前 step 默认值与 engine 全局默认值

#### Scenario: Step default wins over engine global default

- **WHEN** 当前 step 给某个 middleware 配置了默认值，而 request 本身没有显式覆盖
- **THEN** 系统使用该 step 默认值
- **AND** 不再回退到 engine 全局默认值

#### Scenario: Middleware overrides do not inherit from parent request

- **WHEN** 某条 request 派生出 follow request 或 callback 中又构造出新的 request
- **THEN** 新 request 默认不继承父 request 的 middleware override
- **AND** 它只解析自己的 override、目标 step 默认值与 engine 全局默认值

### Requirement: Middleware Trait Uses Native Async Functions

系统 MUST 在 `Spider`、`Middleware` 与相关回调 trait 上使用 Rust 原生 `async fn in trait`，而不是依赖 `#[async_trait]` 宏。

#### Scenario: Middleware hooks use native async fn in trait

- **WHEN** 框架定义 middleware hook 或 spider callback trait
- **THEN** 这些 trait 使用 Rust 原生 `async fn in trait`
- **AND** 本次变更不引入 `#[async_trait]`

## MODIFIED Requirements

### Requirement: Engine Supports Minimal AutoThrottle

系统 MUST 继续提供最小 `AutoThrottle` 能力，但它应当属于 download-before middleware 语义，而不是继续伪装成 `runtime.schedule`。

#### Scenario: AutoThrottle is derived as a default download-before middleware policy

- **WHEN** 调用方通过 engine/settings builder 开启 `AutoThrottle`
- **THEN** 系统把它归一化为默认 download-before middleware
- **AND** `download_delay` 继续表示起始/最小 delay，而不是另一条独立执行阶段

#### Scenario: AutoThrottle feedback stays inside the resolved limit bucket

- **WHEN** 同一个 bucket 最近请求变慢、返回 `429 / 5xx`，或下载直接失败
- **THEN** `auto_throttle` 只调整该 bucket 的后续 delay
- **AND** 不会错误影响其它 limit bucket

## REMOVED Requirements

### Requirement: Request Dedup Is An Explicit Engine Component

**Reason**: request dedup 不再建模成 engine 全局组件；它改成 request-scoped admission policy，并按每条 request 的 effective runtime 决定是否执行。

**Migration**: 旧的“全局 engine dedup 激活”路径不再保留为稳定语义。调用方需要迁移到新的 request runtime / admission policy 模型，并使用显式字段区分 dedup bypass 与 domain bypass。
