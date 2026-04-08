# 变更提案

## 为什么做

- 当前 browser 已经支持 `keep_alive = isolated | context | page` 与 `keep_alive_scope = session | origin`，并且运行时已经有对应的 keep_alive 缓存实现。
- 但现有模型仍然偏粗：调用方只能决定“复用到哪层”和“按什么默认范围分桶”，还不能显式指定业务分桶 key，也不能限制单个 keep_alive 的空闲时间、使用次数，或定义错误后的保留/重置策略。
- 对真实爬虫场景来说，这些控制项比继续堆更多 stealth patch 更直接。不同账号、不同站点子域、不同页面类型，经常都需要更细的 browser 复用策略；如果公开 API 迟迟不收口，调用方就只能继续绕回外部胶水逻辑。

## 范围

- 会影响哪些 capability specs：
  - `openspec/specs/spider-api/spec.md`
  - `openspec/specs/runtime-engine/spec.md`
- 会影响哪些模块 / 示例：
  - `src/request/browser.rs`
  - `src/download/browser.rs`
  - `README.md`
  - `docs/capabilities.md`
  - `examples/browser_advanced.rs`
- 预期带来哪些用户可见结果：
  - browser 配置在 `keep_alive` 与 `keep_alive_scope` 之外，新增更细粒度的 keep_alive 控制项
  - 调用方可以显式声明 `keep_alive_key`
  - 调用方可以限制 keep_alive 的 idle / uses 生命周期
  - 调用方可以声明 keep_alive 遇到 browser 级错误后是保留还是重置

## 非目标

- 这次不修改稳定 session user data dir 的基本语义，session 仍然是 browser 存储态复用的前提。
- 这次不把 keep_alive 控制做成代码回调或动态闭包接口，避免 DSL、序列化与示例表达失控。
- 这次不同时引入更高阶 `DeviceProfile` / `ScreenProfile` 画像结构，这部分单独作为另一个 change 讨论。
- 这次不扩成通用 browser pool 管理器，也不承诺一次解决所有并发与资源治理问题。

## 风险

- 是否存在兼容性或迁移风险：
  - 主要是命名和默认行为风险。需要保证现有 `keep_alive / keep_alive_scope` 用户不改代码也能保持当前语义。
- 是否存在 runtime、middleware 或 plugin 相关风险：
  - keep_alive key、idle 回收、error reset 等能力都会直接影响 downloader 内部缓存命中与复用时机；如果边界定义不清，容易让 session、keep_alive 与错误恢复三条语义互相覆盖。
