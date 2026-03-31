# 技术设计

## 概览

- 保持现有统一模型：`Request -> Downloader -> Response -> parse -> Pipeline`
- `browser` 与 `http` 的差异只体现在下载执行方式，不新增独立 runtime 概念。
- 本轮只把 browser 下载器补到“能消费统一 `Request` 模型中的 `method` / `body` / `session`”这一层。

## 关键决策

- `session`
  - 继续使用稳定的 Playwright persistent context user data dir。
  - 同一个 `session.id` 映射到同一个目录，从而复用 cookies、local storage 等浏览器态。
- `method` / `body`
  - `page.goto()` 仍作为页面导航入口。
  - 在导航前通过 Playwright `BrowserContext::route()` 注册一次请求覆写逻辑。
  - 只在首个目标主文档请求上覆写 `method` 与 `post_data_bytes`，其他资源请求直接原样放行。
- 未实现项
  - `stealth`
  - `fingerprint_profile`
  - 这些能力继续显式返回 download error。

## 模块影响

- `src/download/browser.rs`
  - 放宽 browser request contract 中对 `method` / `body` 的限制
  - 增加导航请求覆写辅助逻辑
  - 为 session、method、body 增补契约测试
- `README.md`
  - 更新 browser 已实现 / 未实现能力边界说明
- `TODO.md`
  - 更新 browser 剩余缺口说明
- `openspec/specs/spider-api/spec.md`
  - 更新 browser request 与共享 Request 模型对齐后的公开行为

## 验证方式

- 单元测试：
  - browser contract 允许 `session`
  - browser contract 允许非 `GET`
  - browser contract 允许 request body
  - 导航请求覆写逻辑只在需要时生成 method/body 覆写
- 回归验证：
  - `cargo fmt --all`
  - `cargo check --all-targets`
  - `cargo clippy --all-targets --all-features -- -W clippy::all`
  - `cargo test --all-targets`
  - `cargo test --all-targets --features browser`
