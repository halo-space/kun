# 任务清单

## 1. Browser Request Runtime 对齐

- [x] 1.1 为 browser `session` 建立稳定的 Playwright user data dir 复用能力。
- [x] 1.2 让 browser downloader 支持统一 `Request` 模型中的非 `GET` method。
- [x] 1.3 让 browser downloader 支持统一 `Request` 模型中的 request body。
- [x] 1.4 保持 `stealth`、`fingerprint_profile` 为显式未实现能力，不做静默忽略。

## 2. 文档与验证

- [x] 2.1 同步 `openspec/specs/spider-api/spec.md` 的 browser request 行为边界。
- [x] 2.2 同步 `README.md` 与 `TODO.md` 的 browser 能力说明。
- [x] 2.3 为 browser request contract 与导航覆写补单元测试。
- [x] 2.4 运行 `cargo fmt --all`、`cargo check --all-targets`、`cargo clippy --all-targets --all-features -- -W clippy::all`、`cargo test --all-targets` 与 `cargo test --all-targets --features browser`。
