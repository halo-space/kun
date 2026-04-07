# 变更提案

## 为什么做

- 当前 `scheduler::Redis` 已经有最小 durable scheduler 语义，包括 `lease_timeout` reclaim、`worker_id` ownership 校验、heartbeat 续租与 namespace snapshot，但运维视图仍然偏聚合：调用方可以看到 namespace 级计数，却还不能直接看到“当前到底是哪条 inflight task 被哪个 worker 持有、lease/deadline 是什么”。这让分布式排障时还要回到 Redis keyspace 手工拼状态。
- 当前 browser 下载器已经有一组稳定内置 `fingerprint_profile` 和最小 `stealth` bootstrap，但还不支持结构化自定义 profile；同时 browser `session` 目前主要复用的是稳定 user data dir，还没有把“是否复用 live context / page”提升成显式运行时策略。
- 这两条能力都不是“没有基础”，而是已经可用但还不够完整；继续补齐后，`halo-spider` 在分布式运维和高阶 browser runtime 这两条主线会更接近一个长期可维护的代码爬虫框架。

## 范围

- 会影响哪些 capability specs：
  - `openspec/specs/runtime-engine/spec.md`
- 会影响哪些模块 / 示例：
  - `src/scheduler/redis.rs`
  - `src/request/browser.rs`
  - `src/download/browser.rs`
  - `README.md`
  - `docs/capabilities.md`
  - `examples/`
- 预期带来哪些用户可见结果：
  - 调用方可以直接从 durable scheduler snapshot 读取 inflight ownership / lease / deadline 明细，而不只是聚合计数
  - browser request 可以声明结构化自定义 fingerprint profile，而不再只接受内置 profile 名称
  - browser `session` 可以显式选择更细粒度的 live reuse 策略，而不再只停留在稳定 user data dir 复用

## 非目标

- 不在这次变更里重写 `scheduler::Redis` 的 ready / delayed / inflight 核心状态机，也不重做现有 Redis 原子脚本模型
- 不在这次变更里引入新的 distributed coordinator、选主机制或跨 Redis 实例事务
- 不在这次变更里接第三方 stealth 套件，也不把 browser 扩成通用自动化框架
- 不在这次变更里引入 browser 点击、滚动、脚本执行等页面动作 DSL

## 风险

- 是否存在兼容性或迁移风险：
  - 低到中。durable scheduler snapshot 的返回结构如果扩展字段，需要保证新增而不是破坏原有字段；browser 配置如果新增结构化 profile 与 reuse 策略，也要保证旧的内置 profile 名称仍然可用
- 是否存在 runtime、middleware 或 plugin 相关风险：
  - 有。browser live context / page reuse 一旦接入不当，容易引入资源泄漏、session 污染或并发竞态；因此这次必须把 reuse 边界和验证策略写清楚
