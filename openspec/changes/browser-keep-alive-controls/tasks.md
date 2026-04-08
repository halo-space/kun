# 任务清单

- [ ] 1.1 复核 `browser-keep-alive-controls` 的 proposal、design 与 `specs/spider-api/spec.md`、`specs/runtime-engine/spec.md`，确认 `keep_alive_key`、懒清理生命周期、`SignedDuration` 边界与错误策略一致。
- [ ] 1.2 在 `src/request/browser.rs` 中为 `browser::Config` 增加 `keep_alive_key`、`keep_alive_max_idle: SignedDuration`、`keep_alive_max_uses`、`KeepAliveOnError` 与对应 builder，并拒绝 `keep_alive_max_idle <= 0`。
- [ ] 1.3 在 `src/download/browser.rs` 中扩展 keep_alive cache key 生成逻辑，让 session、scope 与显式 `keep_alive_key` 收口成同一条 bucket 规则。
- [ ] 1.4 在 `src/download/browser.rs` 中为 keep_alive entry 增加 idle / uses 元数据与懒清理过期逻辑，并为 `keep_alive_on_error = keep | reset` 实现显式行为。
- [ ] 1.5 为 keep_alive key、idle 懒清理、非法 idle 时长、uses 上限与 error policy 增加单元测试，并更新 `examples/browser_advanced.rs`、`README.md` 与 `docs/capabilities.md`。
- [ ] 1.6 运行验证命令并记录结果：`cargo fmt --all && cargo test -q`
