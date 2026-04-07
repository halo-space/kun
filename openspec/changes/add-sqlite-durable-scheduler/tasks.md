# 任务清单

## 1. SQLite durable scheduler 核心后端

- [x] 1.1 新增 `scheduler::Sqlite` 模块与公开导出，确定数据库 schema、scope 组织方式与基础构造 API。
- [x] 1.2 实现 `enqueue / take_ready / complete / requeue / heartbeat / release_inflight` 这组基础调度流转，并保证 task identity、priority、depth、delayed 语义一致。
- [x] 1.3 实现 `take_batch_ready / complete_batch / requeue_batch / complete_and_enqueue_batch`，保证 batch API 在 SQLite 后端上可用。

## 2. 统一运行态与运维控制

- [x] 2.1 为 `scheduler::Sqlite` 实现 `snapshot / scopes / snapshots / overview`，让它复用统一 runtime 观测形状。
- [x] 2.2 为 `scheduler::Sqlite` 实现 `Control`：`pause_scope / resume_scope / release_scope / purge_scope`。
- [x] 2.3 实现 SQLite 后端下的 worker ownership、stale lease、heartbeat 续租与 reclaim 语义。

## 3. 示例与文档

- [x] 3.1 在 `examples/custom_scheduler.rs` 增加 `scheduler::Sqlite` 使用示例。
- [x] 3.2 同步 `README.md`、`docs/capabilities.md` 与 `docs/distributed_scheduler.md`，说明 `Memory / Sqlite / Redis` 的选择边界。

## 4. 验证

- [x] 4.1 为 `scheduler::Sqlite` 增加契约测试，覆盖基础流转、batch、snapshot、control、lease/heartbeat/reclaim。
- [x] 4.2 运行 `cargo check`、`cargo check --examples` 与 SQLite 相关测试并记录结果。
