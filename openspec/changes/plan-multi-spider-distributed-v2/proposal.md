# 变更提案

## 为什么做

- 当前 `halo-spider` 已经有 `scheduler::Redis`、跨 scope 读取、跨 scope 控制、worker heartbeat / lease 这些分布式调度原语，但还没有真正的“多爬虫统一管理”模型。
- 现在如果有三份代码爬虫，`spider.name()` 只能表示“这是哪个爬虫定义”，还不足以表达分布式运行时真正需要的几个身份：
  - `spider_name`
    - 哪个爬虫定义 / 哪套抓取逻辑
  - `job_id`
    - 这次启动的是哪一轮任务 / 哪个运行实例
  - `worker_id`
    - 哪个实际进程 / 节点在执行
  - `scope / namespace`
    - 哪一份共享任务池
- 如果这几个概念不拆开，后续会很难统一解决：
  - 多爬虫同时运行时，任务池怎么隔离
  - 任务控制台怎么区分“爬虫定义”和“一次运行 job”
  - 一个 worker 是固定绑定一个 spider，还是可以承载多个 spider
  - Redis 只做调度状态，还是也承担 spider 注册 / job 发布
- 对 `halo-spider` 来说，v2 更合理的方向不是把 `spider.name()` 继续当成唯一标识，而是补一层真正的多爬虫分布式控制面。
- 当前已有的 `scheduler::Redis` 和 `scope / worker / lease / control` 语义已经提供了很好的地基，所以 v2 更适合“在现有 durable scheduler 之上加 cluster control plane”，而不是再发明另一套任务协调模型。

## 范围

- 会影响哪些 capability specs：
  - `openspec/specs/runtime-engine/spec.md`
  - 新增 `openspec/specs/distributed-spider-cluster/spec.md`
- 会影响哪些模块 / 示例：
  - `src/spider.rs`
  - `src/engine.rs`
  - `src/scheduler/*`
  - 新增候选模块：
    - `src/cluster.rs`
    - `src/cluster/controller.rs`
    - `src/cluster/worker.rs`
    - `src/spider/registry.rs`
  - 候选示例：
    - `examples/distributed_multi_spider_controller.rs`
    - `examples/distributed_multi_spider_worker.rs`
  - 候选文档：
    - `docs/distributed_scheduler.md`
    - `docs/operations.md`
    - 新增多爬虫分布式运维 / 部署文档
- 预期带来哪些用户可见结果：
  - 分布式运行时不再只看 `spider.name()`，而是有清晰的 `spider_name / job_id / worker_id / scope` 四层身份模型。
  - 支持一个统一控制面管理多个代码爬虫，而不是每个 spider 各自单独起进程、手工约定 namespace。
  - 推荐 Redis 作为 v2 的协调数据面，但把“中心控制节点”和“Redis 持久化状态”分开建模。
  - 后续 DSL 也可以复用同一套多爬虫 job / worker / scope 模型，不需要再单独发明集群编排语义。

## 非目标

- 这次 change 只做 v2 规划，不在当前版本里实现真正的多爬虫控制中心。
- 这次不替换当前已有的 `scheduler::Redis` / `scheduler::Sqlite` / `scheduler::Memory` 后端。
- 这次不把 Redis 强行设成唯一后端；它是 v2 的推荐协调面，不是未来所有集群模式的唯一合法实现。
- 这次不顺带实现 UI dashboard、权限系统、租户系统、告警平台或完整作业编排系统。

## 风险

- 是否存在兼容性或迁移风险：
  - 存在。v2 一旦落地，用户对 `spider.name()`、namespace、job identity 的理解会从“一个名字”升级成多层身份模型，旧的部署习惯需要迁移。
- 是否存在 runtime、middleware 或 plugin 相关风险：
  - 存在。多爬虫分布式控制面会跨 `spider / engine / scheduler / operations` 多个模块，需要明确区分“控制面状态”和“调度面状态”，避免 Redis 与控制节点职责混乱。
