# 任务清单

- [x] 1.1 复核 `browser-device-screen-profiles` 的 proposal、design 与 `specs/spider-api/spec.md`，确认公开 API 只保留 `DeviceProfile`，并删除公开 preset 与顶层 viewport。
- [x] 1.2 在 `src/request/browser.rs` 中新增公开 `DeviceProfile`，调整公开 `FingerprintProfile` 与 `ScreenProfile` 作为其可选子结构，并把 `browser::Config` 改成接受 `device_profile`。
- [x] 1.3 在 `src/request/browser.rs` 与 `src/download/browser.rs` 中删除公开 `fingerprint_preset` 和旧内部扁平 `FingerprintProfile` 模型，引入新的内部归一化 profile plan。
- [x] 1.4 在 `src/download/browser.rs` 中实现 `DeviceProfile + engine` 的编译、校验、headers 与 init script 生成逻辑，允许 `fingerprint` 部分填写，并补齐默认值规则。
- [x] 1.5 为 `DeviceProfile.screen` 的 `viewport / screen / avail` 三个子结构补 builder 与组合规则测试，覆盖默认推导、显式覆盖与冲突报错场景。
- [x] 1.6 更新 `examples/browser_advanced.rs`、`README.md` 与 `docs/capabilities.md`，用新 API 展示 `DeviceProfile` 的分层写法，并明确“画像类能力进 profile、运行策略留在 Config”的划分原则。
- [x] 1.7 运行验证命令并记录结果：`cargo fmt --all && cargo test -q`
