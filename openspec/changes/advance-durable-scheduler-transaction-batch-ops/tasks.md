# 任务清单

## 1. 事务边界

- [x] 1.1 梳理 `engine -> store -> scheduler resolve -> follow/retry enqueue` 的当前提交路径，明确成功、失败、重试、lease lost 的结果边界。
- [x] 1.2 实现这一轮的最小事务边界收口，并把语义补到测试里。
- [x] 1.3 在 README / distributed scheduler 文档里明确这层边界，不再模糊表述为“整体事务”。

## 2. Batch 调度接口

- [x] 2.1 在共享 `Scheduler` trait 上增加统一 batch API，命名统一使用 `batch`。
- [x] 2.2 为 `scheduler::Memory` 实现 batch API，保证本地后端先跑通。
- [x] 2.3 为 `scheduler::Redis` 实现 batch API，并补单元测试 / 契约测试。
- [ ] 2.4 评估 engine 主循环是否需要接入 batch claim 路径，并补最小接线。

## 3. 跨 Job 运维控制面

- [ ] 3.1 定义统一的跨 job 运维入口，明确哪些是读接口，哪些是控制动作。
- [ ] 3.2 实现最小控制动作，并保证 `Memory / Redis / 后续后端` 可以沿同一抽象扩展。
- [ ] 3.3 补 `docs/distributed_scheduler.md` 与 `examples/custom_scheduler.rs` 示例，给出真实操作方式。

## 4. 文档与收口

- [ ] 4.1 同步 `openspec/specs/runtime-engine/spec.md`、`README.md`、`docs/capabilities.md`。
- [ ] 4.2 完成实现后运行相关 `cargo test` 与 `cargo check --examples`。
