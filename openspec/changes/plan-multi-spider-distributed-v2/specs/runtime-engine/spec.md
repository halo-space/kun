# 规范增量

## ADDED Requirements

### Requirement: Runtime Engine Exposes Multi-Identity Cluster Semantics

系统 MUST 在分布式多爬虫模式里明确区分 `spider_name`、`job_id`、`worker_id` 与 `scope`，而不是继续把这些概念混成一个标识。

#### Scenario: Engine telemetry can distinguish spider definition from one job run

- **WHEN** 某个 worker 执行一个分布式 spider job
- **THEN** engine 级日志、signal 或 telemetry 可以同时带出 `spider_name` 与 `job_id`
- **AND** 调用方可以区分“这是哪个 spider 定义”和“这是哪一次运行”

#### Scenario: Scheduler scope is not assumed to equal spider name

- **WHEN** engine 以分布式模式运行某个 spider job
- **THEN** 它使用显式的 `scope` 连接到底层 scheduler
- **AND** 这个 `scope` 可以等于、也可以不等于 `spider_name`

