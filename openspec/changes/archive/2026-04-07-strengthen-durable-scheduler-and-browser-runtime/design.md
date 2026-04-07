# 技术设计

## 概览

- 这次变更分两条实现线：
  - durable scheduler：在不改现有 Redis 状态机前提下，把 inflight ownership / lease / deadline 视图提升成公开、可测试的 snapshot 明细
  - browser runtime：在保留现有内置 profile 与最小 stealth 路线的前提下，补结构化自定义 fingerprint profile，并把 session 的 live reuse 策略提升成显式配置

## 模块影响

- `src/scheduler/redis.rs`
  - 扩展 namespace snapshot 结构，暴露更细粒度 inflight task 运维信息
  - 同步 namespace / snapshot 读取逻辑与测试
- `src/request/browser.rs`
  - 新增结构化 fingerprint profile 配置
  - 新增 browser session reuse 策略配置
- `src/download/browser.rs`
  - 调整 profile 解析与 init script 生成逻辑，让内置 preset 与结构化 profile 走同一条执行计划
  - 按新的 reuse 策略决定 session 级 context / page 生命周期
- `examples/`
  - 补 browser 高级能力最小示例
- `README.md`
  - 同步 durable scheduler 运维视图与 browser 高级能力用法
- `docs/capabilities.md`
  - 更新 scheduler snapshot 能力说明、browser profile / reuse 说明
- `openspec/specs/runtime-engine/spec.md`
  - 补 scheduler snapshot 明细 requirement / scenario
  - 补 browser 自定义 profile 与 reuse 策略 requirement / scenario

## 关键决策

- Runtime / middleware 影响：
  - 不新增 middleware；durable scheduler 仍然挂在 `scheduler` owner 下，browser 高级能力仍然挂在统一 `Request.browser` 配置下
  - engine 不需要新增另一套 durable scheduler runtime；当前 heartbeat 机制继续复用已有 `Scheduler::heartbeat()` 接口
- 对外 API 影响：
  - `scheduler::Redis::snapshot()` 会新增 inflight 明细字段，但保留当前已有聚合字段
  - browser 公开配置会新增结构化 custom profile 与 reuse policy builder，旧的 `with_fingerprint_profile("desktop_...")` 保持可用
- Plugin 或 DSL 影响：
  - 不新增 plugin kind
  - DSL 这次不直接扩展，但 browser 配置和 durable scheduler 公开能力收口后，后续 DSL 才有稳定目标可映射
- Browser reuse 权衡：
  - 当前 session 已经串行化，所以 live reuse 也必须在同一 session 串行边界内工作
  - 先把 reuse 做成显式策略，而不是默认打开，避免无意引入跨请求污染
  - 如果某条策略会显著增加资源泄漏风险，就先只开放更保守的层级，不强行一步到位

## 验证方式

- 为 `scheduler::Redis` 增加 snapshot 明细测试，覆盖：
  - inflight task 的 worker / lease / deadline 可读
  - reclaim 后 snapshot 明细同步变化
- 为 browser 配置增加单元测试，覆盖：
  - 结构化 custom profile 与内置 preset 共存
  - 非法 profile / reuse 配置的显式报错
  - reuse 策略在 session 维度上的执行计划差异
- 同步 `README.md`、`docs/capabilities.md` 与 spec 增量
- 运行：
  - `cargo test --quiet`
