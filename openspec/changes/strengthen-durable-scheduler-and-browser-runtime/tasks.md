# 任务清单

- [x] 1.1 为 durable scheduler 与 browser 高级能力写 proposal、design 与 spec 增量，明确这次只补 inflight ownership 运维视图、结构化 custom profile 与显式 reuse 策略。
- [x] 1.2 扩展 `src/scheduler/redis.rs` 的 snapshot 公开结构与读取逻辑，返回 inflight task 的 ownership / lease / deadline 明细。
- [x] 1.3 为 `scheduler::Redis` 新增和更新测试，覆盖 snapshot 明细、reclaim 后明细变化，以及 namespace snapshot 聚合与明细一致性。
- [x] 1.4 在 `src/request/browser.rs` 定义结构化 custom fingerprint profile 与显式 session reuse 策略，并保持旧的内置 profile API 可用。
- [x] 1.5 调整 `src/download/browser.rs` 的执行计划与 session 路径，让内置 preset、自定义 profile 和 reuse 策略走同一条运行时逻辑。
- [x] 1.6 补 browser 高级能力示例，并同步 `README.md`、`docs/capabilities.md`。
- [x] 1.7 运行 `cargo test --quiet` 验证整轮变更。
