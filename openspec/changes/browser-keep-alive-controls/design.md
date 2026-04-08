# 技术设计

## 概览

- 这次变更保留当前 `keep_alive = isolated | context | page` 与 `keep_alive_scope = session | origin` 两层基础语义。
- 在此基础上，browser 配置新增四类更细粒度控制：
  - `keep_alive_key`
  - `keep_alive_max_idle: SignedDuration`
  - `keep_alive_max_uses`
  - `keep_alive_on_error = keep | reset`
- downloader 内部的 keep_alive cache 继续以当前 session 为前提，但 cache key 会扩展成：
  - `session id`
  - `keep_alive_scope` 派生值
  - 可选 `keep_alive_key`
- keep_alive entry 会增加生命周期元数据，例如最近一次归还时间、累计使用次数，用于 idle / uses 控制。
- keep_alive entry 的 idle 过期采用懒清理：
  - 只在取回或归还对应 bucket 时检查并淘汰过期 entry
  - 这次不引入独立后台 sweep 任务

## 模块影响

- `src/request/browser.rs`
  - `browser::Config` 新增 keep_alive 细粒度控制字段与 builder
  - 新增 `KeepAliveOnError` 枚举
  - `keep_alive_max_idle` 继续使用 `jiff::SignedDuration`
- `src/download/browser.rs`
  - 扩展 keep_alive cache key 生成逻辑
  - 扩展 keep_alive entry 元数据
  - 在取回 / 存回 keep_alive 时执行懒清理并检查 idle / uses 过期
  - 在 browser 级错误发生时按 `keep_alive_on_error` 决定保留还是重置
- `examples/browser_advanced.rs`
  - 增加 `keep_alive_key` 与生命周期控制示例
- `README.md`
  - browser keep_alive 能力边界更新
- `docs/capabilities.md`
  - browser keep_alive 控制项更新
- `openspec/specs/spider-api/spec.md`
  - 增加 browser keep_alive 公开配置 requirement
- `openspec/specs/runtime-engine/spec.md`
  - 增加 keep_alive 生命周期与错误处理的运行时 requirement

## 关键决策

- Runtime / middleware 影响：
  - 这次不影响 `Settings`、middleware 链或 plugin 自动装载。
  - 运行时影响集中在 browser downloader 内部 keep_alive cache 的 key、生命周期与错误恢复策略。
- 对外 API 影响：
  - `keep_alive` 继续只表达复用层级。
  - `keep_alive_scope` 继续只表达默认分桶范围。
  - `keep_alive_key` 只表达显式业务分桶，不替代 session。
  - `keep_alive_max_idle` 固定使用 `jiff::SignedDuration` 表达空闲窗口，不再额外引入另一套时长类型。
  - `keep_alive_max_idle <= 0` 属于非法配置，系统必须显式拒绝，而不是静默退化成“不启用”。
  - `keep_alive_max_idle` 与 `keep_alive_max_uses` 只约束 keep_alive entry 的存活，不改变稳定 user data dir 语义。
  - `keep_alive_on_error` 只表达 browser 级错误后的 entry 处置策略，不与 idle / uses 回收语义混用。
  - 这组 keep_alive 控制项继续留在 `browser::Config` 外层，不进入 `DeviceProfile`；它们属于运行策略，而不是身份画像。
- 生命周期实现策略：
  - keep_alive 的 idle 回收固定采用懒清理，避免额外后台任务、定时器和运行时治理复杂度。
  - 是否需要未来引入主动 sweep，不在这次 change 范围内。
- Plugin 或 DSL 影响：
  - 这次先不扩 DSL 字段，但后续 DSL 如果表达 browser keep_alive，直接复用这些命名。
  - 不提供代码回调式 key 计算接口，避免 DSL、序列化与示例出现两套表达。

## 验证方式

- 为 `browser::Config` 新增 keep_alive 细粒度 builder 单元测试。
- 为 keep_alive cache key 生成新增测试，覆盖：
  - `session`
  - `origin`
  - 显式 `keep_alive_key`
- 为 idle / uses 生命周期新增测试，确认过期 entry 会在懒清理路径上被关闭并重建。
- 为 `keep_alive_max_idle <= 0` 新增显式失败测试，确认非法配置不会被静默接受。
- 为 `keep_alive_on_error = keep | reset` 新增测试，确认 browser 级错误后 entry 的保留或丢弃行为符合声明。
- 更新 README 与 `examples/browser_advanced.rs`，确保调用方能看到最小使用方式。
