# 技术设计

## 概览

- v2 的目标不是只让多个 worker 共享一个 Redis 任务池，而是补齐“多爬虫统一管理”的完整运行模型。
- 推荐的整体结构是两层：
  - 控制面
    - 一个轻量 controller / admin service
    - 负责 spider 注册、job 创建、job 生命周期管理、跨 job 运维入口
  - 协调数据面
    - Redis 作为推荐后端
    - 负责 scheduler scope、worker lease、job registry、worker heartbeat、最小 job control state
- 其中 Redis 不是“全部逻辑都塞进去”的中心节点，而是集群运行时的共享状态中心；真正的控制逻辑仍然应该有显式 controller。
- 如果用户不想先上 controller，v2 也可以允许最小模式：
  - 通过 CLI 直接把 job manifest 写进 Redis / scheduler scope
  - worker 直接从共享状态里读取任务
  - 但这只是降级模式，不是推荐的“真正多爬虫统一管理”形态

## 模块影响

- `src/spider.rs`
  - `spider.name()` 继续保留，但只作为 spider definition identity。
  - 需要补“spider registry / spider factory”语义，让一个 worker 进程可以承载多个 spider 定义。
- `src/engine.rs`
  - 需要区分“本地单 spider engine run”和“cluster worker 内执行某个 job 的 engine run”。
  - engine 运行时日志 / signal / telemetry 需要同时带上 `spider_name` 与 `job_id`。
- `src/scheduler/*`
  - 现有 `scope` 模型继续保留，但需要明确 namespace 规则。
  - 推荐的 namespace 语义：
    - 长跑模式：`jobs:{spider_name}`
    - 批次模式：`jobs:{spider_name}:{job_id}`
- 候选新增：
  - `src/cluster.rs`
    - cluster 对外入口
  - `src/cluster/controller.rs`
    - spider/job 控制面
  - `src/cluster/worker.rs`
    - worker 注册、心跳、job 拉起、job 停止
  - `src/spider/registry.rs`
    - `spider_name -> spider factory` 注册表
- `docs/distributed_scheduler.md`
  - 需要明确“当前已有的是分布式调度原语，不等于多爬虫控制中心”。
- `docs/operations.md`
  - 需要新增 cluster 级读写操作说明。
- `openspec/specs/runtime-engine/spec.md`
  - 需要补 `spider_name / job_id / worker_id / scope` 的关系。
- `openspec/specs/distributed-spider-cluster/spec.md`
  - 新增 v2 capability，描述 cluster control plane 的需求边界。

## 关键决策

- Runtime / middleware 影响：
  - v2 的重点不是 middleware，而是 job / worker / scheduler scope 的编排边界。
  - request / runtime / dedup / download-before middleware / retry middleware 仍然沿用底层统一模型，不为 cluster 模式另起一套执行语义。
- 对外 API 影响：
  - `spider.name()` 不再被暗示成“分布式唯一标识”；它只表示 spider definition。
  - 需要新增 job 级公开模型，例如：
    - `JobSpec`
    - `JobId`
    - `SpiderName`
    - `WorkerId`
  - 需要新增 spider registry 入口，让 worker 可以按名称拉起对应 spider。
- 控制面 / Redis 影响：
  - 推荐 Redis 作为默认协调数据面，因为当前已有 durable scheduler、scope、lease、跨 scope control 原语。
  - 但 Redis 不应该单独承担全部控制逻辑；推荐仍有一个显式 controller。
  - controller 负责：
    - 发布 spider/job manifest
    - 选择 namespace / scope
    - 管理 job 状态
    - 对外提供 pause / resume / stop / purge / inspect
- 多爬虫身份模型：
  - `spider_name`
    - 哪个爬虫定义
  - `job_id`
    - 某次运行实例
  - `worker_id`
    - 某个执行进程 / 节点
  - `scope`
    - 共享任务池 identity
  - 这四个概念必须分开，不能继续用 `spider.name()` 一把梭。

## 推荐的 v2 架构

- 最小推荐形态：
  - `controller`
    - 单独进程或 HTTP service
    - 维护 spider registry manifest、job registry、job control state
  - `redis`
    - scheduler scope + leases + worker runtime + job registry backend
  - `worker`
    - 启动时注册自己
    - 从 controller 或 Redis 中获取可执行 job
    - 通过 spider registry 按 `spider_name` 拉起具体 spider

```text
Controller
  -> submit job(spider_name, job_id, params, scope)
  -> persist job manifest / desired state
  -> expose ops APIs

Redis
  -> scheduler scopes
  -> worker leases / heartbeats
  -> job registry / control state

Worker
  -> register worker_id
  -> accept or claim runnable jobs
  -> instantiate spider by spider_name
  -> run Engine against scope
```

## 当前与 v2 之间的关键缺口

- 现在已经有：
  - `scheduler::Redis`
  - `scope`
  - `worker_id`
  - `lease / heartbeat`
  - `pause_scope / resume_scope / release_scope / purge_scope`
- 但还没有：
  - spider registry
  - job manifest / job registry
  - controller / worker 协议
  - `spider_name / job_id / worker_id / scope` 的统一公开模型
  - 一个 worker 承载多个 spider 的标准方式

## 验证方式

- v2 规划阶段先验证文档和模型一致性：
  - `spider_name`
  - `job_id`
  - `worker_id`
  - `scope`
  - controller vs Redis 的职责边界
- 真正实现时需要的验证：
  - 多 spider registry 正常发现与实例化
  - 同一 spider 多 job 隔离
  - 同一 job 多 worker 共享任务池
  - 跨 job pause / resume / purge
  - controller 崩溃后，Redis 中的运行时状态仍可恢复巡检
