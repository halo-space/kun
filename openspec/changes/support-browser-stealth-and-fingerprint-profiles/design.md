# 技术设计

## 概览

- 在当前 `playwright-rs` 路线下，把 `BrowserConfig::fingerprint_profile` 从占位字段补成“内置 profile 名称 -> Playwright context options + JS bootstrap”的确定性映射。
- 把 `BrowserConfig::stealth` 从“直接报未实现”调整为“启用最小 stealth bootstrap”。这层 bootstrap 只覆盖当前能稳定表达、且能通过本地 API 注入的内容，例如：
  - `navigator.webdriver`
  - `navigator.languages`
  - `navigator.platform`
  - 部分 `window.chrome` / permissions 查询补丁
- 对 `fingerprint_profile` 支持范围继续收紧为内置 profile 集合；未识别的 profile 名称仍然显式失败。
- 保持现有 request override、cookies、proxy、timeout、session user data dir 与 session 串行化逻辑不变，只在 context options 构建与 context 初始化阶段增加 profile / stealth 应用。

## 模块影响

- `src/request/browser.rs`
  - 保持已有公开字段不变。
  - 如有必要，增加 profile 解析辅助类型，但不扩大公开配置面。
- `src/download/browser.rs`
  - 增加内置 fingerprint profile 定义。
  - 在 `validate_browser_request_contract()` 中把“全部报未实现”改成“支持的配置允许执行，不支持的 profile 继续失败”。
  - 在 `build_context_options()` 中把 profile 映射到 `user_agent`、`locale`、`timezone_id`、额外请求头等 Playwright context options。
  - 在 context 创建后，通过 `BrowserContext::add_init_script()` 注入 stealth / profile 相关 bootstrap。
- `README.md`
  - 更新 browser capability 说明，区分“已支持的最小 stealth/profile”与“仍未实现的高级能力”。
- `TODO.md`
  - 收口 browser 缺口描述，去掉已经落地的 `stealth` / `fingerprint_profile` 占位说明，保留更高阶缺口。
- `openspec/specs/spider-api/spec.md`
  - 更新 browser request 行为边界。

## 关键决策

- Runtime / middleware 影响：
  - 这次只影响 browser downloader 的 context 初始化阶段，不新增 middleware，也不改变 engine/scheduler/pipeline 行为。
  - `Request` 仍然是唯一能力入口，browser 配置只是 request 的执行细节。
- 对外 API 影响：
  - 保持 `BrowserConfig { stealth, fingerprint_profile }` 字段与 builder 方法不变，避免额外迁移成本。
  - profile 名称先收敛为内置集合，例如 `desktop_zh_cn`、`desktop_en_us`。未识别名称继续显式失败。
- Plugin 或 DSL 影响：
  - 本轮不扩 DSL。
  - DSL 未来若要映射这些 browser 能力，应直接复用这次落地的底层 profile / stealth 语义，而不是再发明一套独立配置解释器。
- 权衡：
  - 不追求“完整 stealth 套件”，而是优先实现本地 Playwright API 已明确支持的最小集。
  - 把 stealth 拆成稳定的 init script 片段，避免引入额外第三方脚本资产或复杂的运行时依赖。

## 验证方式

- 补单元测试，覆盖：
  - 支持的 `fingerprint_profile` 会生成预期的 context options
  - `stealth = true` 会生成非空 init script/bootstrap
  - 未识别的 profile 名称继续显式失败
  - profile 与现有 headers/proxy/timeout 合并时不破坏旧行为
- 运行：
  - `cargo fmt --all`
  - `cargo test --all-targets`
- 如果整库测试仍被当前已知的非本变更问题阻塞，需要在结果里明确说明阻塞点，而不是把阻塞误判成 browser 实现错误。
