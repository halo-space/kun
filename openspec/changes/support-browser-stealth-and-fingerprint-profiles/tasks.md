# 任务清单

## 1. Browser Profile 与 Stealth Runtime

- [x] 1.1 定义内置 `fingerprint_profile` 集合，并把 profile 名称映射到稳定的 browser context 选项。
- [x] 1.2 为 browser context 注入最小 stealth / fingerprint bootstrap，不破坏现有 request override、cookies、proxy、timeout 与 session 语义。
- [x] 1.3 对未支持的 `fingerprint_profile` 保持显式失败，同时让 `stealth = true` 进入受支持执行路径。

## 2. 测试与文档

- [x] 2.1 为 profile 解析、context options 合并、bootstrap 生成补单元测试。
- [x] 2.2 同步 `README.md` 与 `TODO.md` 的 browser 能力说明。
- [x] 2.3 运行 `cargo fmt --all` 与相关测试；如果整库测试仍被当前已知问题阻塞，在结果中明确阻塞点。
