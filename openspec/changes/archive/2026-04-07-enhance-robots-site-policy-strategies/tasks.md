# 任务清单

- [x] 1.1 重新定义 `robots::Memory` 的 site policy 公开入口，把它从 exact-origin-only 模型改成显式 site matcher 模型。
- [x] 1.2 调整 `src/robots.rs` 内部站点策略存储与匹配逻辑，支持多条 site policy 同时命中同一个请求。
- [x] 1.3 明确并实现 site policy 的 precedence / merge 语义：`access` 与 `unavailable_policy` 走更具体 matcher，`delay` 取更严格值，`sitemaps` 做去重合并。
- [x] 1.4 为新的 site matcher 与 merge 行为补单元测试，覆盖 exact origin、host、pattern、多命中 precedence 与 unavailable override。
- [x] 1.5 同步 `openspec/specs/runtime-engine/spec.md`、`README.md`、`docs/capabilities.md` 与 `TODO.md`。
- [x] 1.6 运行 `cargo test --quiet` 验证整轮变更。
