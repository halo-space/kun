# halo-spider

`halo-spider` 是一个受 Scrapy 启发的 Rust 异步爬虫框架。当前优先完善代码爬虫主线与共享底层能力；DSL 保留为后续配置化入口，不作为当前主线。

## 项目定位

- 统一 `Request / Response / follow` 模型
- 同时支持 `download::Http` 与 `download::Browser`
- 内置 `scheduler::Memory / Sqlite / Redis`
- 内置 `dedup / robots / pipeline / store / validator / telemetry`
- 核心能力优先走 trait + engine 显式注入；plugin 当前只用于 `middleware` 的声明式装配

## 当前状态

- 代码爬虫主线已经可用，当前重点是持续收口共享 runtime 边界
- durable scheduler 已具备统一读能力、control 能力和 batch 吞吐入口
- DSL 继续后置，先与代码爬虫主线共享同一套底层模型
- 规范与变更通过 `openspec/` 管理

## 文档导航

- [使用手册](docs/guide.md)
- [功能说明](docs/capabilities.md)
- [运维 / 观测指南](docs/operations.md)
- [分布式调度说明](docs/distributed_scheduler.md)
- [自定义 Scheduler 后端](docs/custom_scheduler_backend.md)
- [示例说明](examples/README.md)

## 快速看示例

- [custom_scheduler.rs](examples/custom_scheduler.rs)
- [browser_advanced.rs](examples/browser_advanced.rs)
- [middleware_plugin.rs](examples/middleware_plugin.rs)
- [robots_site_policy.rs](examples/robots_site_policy.rs)
- [concurrency_control.rs](examples/concurrency_control.rs)

## 协作

- 规范源位于 `openspec/specs/`
- 变更提案从 `openspec/changes/` 发起
- 协作入口位于 `.claude/commands/opsx/` 与 `.codex/skills/`
- 贡献说明见 [CONTRIBUTING.md](CONTRIBUTING.md)

## License

MIT
