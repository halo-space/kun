# halo-spider 使用手册

这份文档是 `halo-spider` 的手册首页。
README 只保留项目简介与导航；详细能力、边界和用法按章节收口在这里。

`halo-spider` 是一个受 Scrapy 启发的 Rust 异步爬虫框架。当前优先补齐和稳定代码爬虫与共享底层能力；DSL 入口仍保留，但配置面暂时后置，并使用 OpenSpec 管理规范与变更。

## 阅读顺序

建议按下面顺序阅读：

1. [入门](./guide/getting-started.md)
2. [Scheduler 与 Runtime](./guide/scheduler-and-runtime.md)
3. [示例、Pipeline 与 Store](./guide/examples-and-store.md)
4. [Browser 与 AI](./guide/browser-and-ai.md)
5. [DSL 与项目协作](./guide/dsl-and-project.md)
6. [Rules DSL 设计（v1）](./guide/rules-dsl.md)

## 章节说明

- [入门](./guide/getting-started.md)
  先看项目结构、文档分工、当前能力概览和最小可运行示例。
- [Scheduler 与 Runtime](./guide/scheduler-and-runtime.md)
  重点看 scheduler 选型、durable 语义、runtime 观测、并发控制与 http cache。
- [示例、Pipeline 与 Store](./guide/examples-and-store.md)
  重点看示例入口、item 主链、内置 store 与自定义 store 扩展。
- [Browser 与 AI](./guide/browser-and-ai.md)
  重点看 browser 下载边界、`device_profile`、`keep_alive`、stealth 与 AI selector。
- [DSL 与项目协作](./guide/dsl-and-project.md)
  重点看 DSL 当前定位、共享模型边界，以及文档 / OpenSpec 协作方式。
- [Rules DSL 设计（v1）](./guide/rules-dsl.md)
  重点看新一版 DSL 的链路模型、配置骨架和字段说明。

## 配套文档

- [功能说明](./capabilities.md)
- [运维 / 观测指南](./operations.md)
- [分布式调度说明](./distributed_scheduler.md)
- [自定义 Scheduler 后端](./custom_scheduler_backend.md)
- [示例说明](../examples/README.md)
- [项目首页](../README.md)
- [贡献指南](../CONTRIBUTING.md)

## 当前状态

- 代码爬虫主线已经可用，当前重点是持续收口共享 runtime 边界
- durable scheduler 已具备统一读能力、control 能力和 batch 吞吐入口
- plugin 当前只用于 `middleware` 的声明式装配
- DSL 继续后置，先与代码爬虫主线共享同一套底层模型

## 仓库定位

- `src/`：框架实现
- `examples/`：最小可运行示例
- `docs/`：使用手册、能力说明、运维说明
- `openspec/specs/`：当前规范源
- `openspec/changes/`：需求、方案、任务的变更入口
