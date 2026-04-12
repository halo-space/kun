# 规范增量

## ADDED Requirements

### Requirement: Spider Definition Identity Is Distinct From Job Identity

系统 MUST 把 spider definition identity 与运行时 job identity 分开建模，而不是继续只依赖 `spider.name()`。

#### Scenario: Multiple jobs can run from the same spider definition

- **WHEN** 调用方使用同一个 `spider_name` 连续启动多次运行
- **THEN** 系统可以为它们分配不同的 `job_id`
- **AND** 它们不会因为共用同一个 `spider_name` 就被视为同一轮 job

#### Scenario: Spider name remains a stable definition key

- **WHEN** worker 需要根据名称实例化某个 spider
- **THEN** `spider_name` 继续表示 spider definition identity
- **AND** 它不承担 worker identity 或 job identity 语义

### Requirement: Cluster Runtime Uses Explicit Worker Identity

系统 MUST 在多爬虫分布式运行里显式建模 `worker_id`，而不是把 worker 身份隐含在进程、主机名或 spider name 中。

#### Scenario: Multiple workers can serve one job

- **WHEN** 一个 job 由多个 worker 并发执行
- **THEN** 每个 worker 都有不同的 `worker_id`
- **AND** 它们共享同一个 scope 的调度状态

### Requirement: Scope Namespace Is A Separate Runtime Identity

系统 MUST 让 scheduler `scope / namespace` 成为独立运行时身份，而不是直接等同于 `spider_name`。

#### Scenario: Batch jobs can isolate scopes per job

- **WHEN** 系统以批次模式运行某个 spider
- **THEN** 它可以使用形如 `jobs:{spider_name}:{job_id}` 的独立 scope
- **AND** 同 spider 的不同 job 不会共享同一个任务池

#### Scenario: Long-running jobs can reuse a stable spider scope

- **WHEN** 系统以常驻模式运行某个 spider
- **THEN** 它也可以使用稳定 scope，例如 `jobs:{spider_name}`
- **AND** scope 语义由 job policy 决定，而不是被 `spider_name` 强行写死

### Requirement: Cluster Control Plane Is Explicit

系统 MUST 为真正的多爬虫分布式运行引入显式 control plane，而不是只把 Redis 当成唯一中心节点。

#### Scenario: Controller manages spider and job lifecycle

- **WHEN** 调用方提交、停止、暂停、恢复或巡检某个 spider job
- **THEN** 系统通过显式 controller / admin plane 暴露这些动作
- **AND** controller 负责管理 spider registry、job registry 与 job desired state

#### Scenario: Redis remains the recommended coordination data plane

- **WHEN** 系统运行默认的多爬虫分布式模式
- **THEN** Redis 作为推荐协调数据面保存 scheduler scope、worker lease、heartbeat 与最小 job registry 状态
- **AND** Redis 不单独承担全部控制逻辑

### Requirement: Workers Can Host Multiple Spider Definitions Through A Registry

系统 MUST 提供 spider registry 语义，使一个 worker 进程可以承载多个 spider 定义。

#### Scenario: Worker instantiates spider by spider name

- **WHEN** 某个 worker 收到一个待执行 job，其中声明了 `spider_name`
- **THEN** worker 可以通过 spider registry 找到对应 spider factory 并实例化该 spider
- **AND** 不要求每个 worker 进程只编译或只承载一个 spider
