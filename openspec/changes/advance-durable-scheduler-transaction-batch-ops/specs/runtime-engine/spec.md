# 规范增量

## ADDED Requirements

### Requirement: Engine defines explicit scheduler transaction boundaries

系统必须明确任务执行完成后 `store`、follow/retry enqueue 与 scheduler resolve 的提交边界，而不是只依赖“单条 scheduler 状态迁移原子”来隐含整个引擎事务语义。

#### Scenario: Successful task completion has an explicit commit boundary

- **WHEN** 某条任务成功执行并产出 item 与 follow request
- **THEN** 系统对 `store`、follow enqueue 与 scheduler complete 的顺序和失败语义是明确、可测试的
- **AND** 调用方可以从文档和运行结果中理解这条路径的 at-least-once 边界

#### Scenario: Store failure does not silently commit scheduler completion

- **WHEN** 某条任务已经完成解析，但 `store` 写入失败
- **THEN** 系统不会把该任务静默当成已成功完成
- **AND** 调用方可以稳定区分这是 store failure，而不是 scheduler completion

#### Scenario: Scheduler resolve failure after store commit stays diagnosable

- **WHEN** 某条任务已经成功写入 `store`，但后续 `scheduler complete / complete-and-enqueue` 失败
- **THEN** 已写出的 item 不会被回滚
- **AND** 调用方可以通过测试、文档与日志明确理解这条 at-least-once 边界

### Requirement: Shared scheduler trait supports batch operations

系统必须为共享 `scheduler::Scheduler` trait 提供统一 batch 调度接口，并统一使用 `batch` 命名，而不是只让高吞吐路径依赖单条接口并发堆叠。

#### Scenario: Backends can claim ready tasks in batch

- **WHEN** 调用方对某个 scheduler 请求一批 ready task
- **THEN** `Memory`、`Redis` 以及后续其它后端都可以通过统一 batch API 返回这批 claim 结果

#### Scenario: Backends can resolve multiple leases in one batch API

- **WHEN** 调用方需要批量 complete、requeue，或 complete-and-enqueue 多条任务
- **THEN** 共享 scheduler trait 提供统一的 batch 入口
- **AND** 默认实现允许旧后端先回退到单条接口循环执行

### Requirement: Scheduler operations plane is distinct from batch execution

系统必须把跨 job 运维控制动作与 batch 执行接口区分开，不把 `pause / resume / release / purge` 这类运维动作混入 batch 调度 API。

#### Scenario: Control actions are not modeled as batch task resolution

- **WHEN** 调用方需要对 scope、worker 或 job 执行运维动作
- **THEN** 这些动作通过独立的运维控制入口暴露
- **AND** 它们不与 `take_batch_ready / complete_batch / requeue_batch` 混成同一组接口

### Requirement: Cross-job scheduler control is explicit

系统必须为多 job / 多 scope 的 scheduler 运行提供统一、明确的运维控制入口，而不是只提供快照读取。

#### Scenario: Multiple scopes can be inspected through one control interface

- **WHEN** 调用方在同一个 backend 上运行多个 job / scope
- **THEN** 它可以通过统一入口读取 scopes、overview、workers 等跨 job 视图

#### Scenario: Cross-job control actions have concrete examples

- **WHEN** 调用方需要暂停、恢复、释放 worker 持有任务，或清理某个 scope
- **THEN** 文档和示例提供具体操作方式
- **AND** 这些动作的边界与影响范围是明确、可测试的
