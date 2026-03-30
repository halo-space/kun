# 技术设计

## 概览

- 这次变更采用“先建立正式缺口台账，再按优先级逐步实现”的方式推进。
- 设计原则是：凡是代码爬虫应该直接可用的能力，都优先落到 `src/` 的共享底层能力中，再由 DSL 编译到同一条链路，而不是先在 DSL 上长出一套私有运行时。
- 这次设计不引入新的总控运行时，而是继续复用现有：
  - `Spider / Request / Response`
  - `Engine / Scheduler`
  - `runtime -> middleware`
  - `rules compile -> rules run`

## 模块影响

- `src/request.rs`
  - 后续补齐 request 级 timeout / proxy / cookies / follow 派生能力时会扩展
- `src/response/follow.rs`
  - 后续补齐 follow 派生语义时会扩展
- `src/engine.rs`
  - 后续补齐共享 validate、输出策略、错误分类时会扩展
- `src/scheduler/*`
  - 后续补齐 task identity、ack/nack 语义时会修改
- `src/middleware/*`
  - 后续补齐 `cookies`、`proxy`、以及与 runtime 的真实接线
- `src/download/*`
  - 后续补齐 HTTP timeout / proxy / cookie jar / browser 真正能力边界
- `src/parser/*`
  - 后续补齐 HTML XPath、OCR、parse 后处理
- `src/pipeline.rs`
  - 后续在单一 pipeline 模型上补齐 item 输出与持久化能力
- `src/plugins/*`
  - 后续补齐除 middleware 外的扩展边界时会扩展
- `src/rules/*`
  - 后续补齐“DSL 只是共享底层能力配置入口”的对齐工作
- `examples/...`
  - 当前不要求为每个缺口立即新增示例；只在某项能力真正落地并稳定后再补
- `openspec/specs/...`
  - 通过 delta spec 明确底层能力应有的行为边界

## 关键决策

- Runtime / middleware 影响：
  - `validate`、`proxy`、`cookies`、item 输出策略等能力应视情况进入统一 runtime / middleware / engine 链路，而不是停留在 DSL 私有字段。
- 对外 API 影响：
  - request/follow、scheduler identity、browser 边界、pipeline item 处理语义都可能产生 API 或行为收紧，但会按小步任务推进，不一次性大改。
- Plugin 或 DSL 影响：
  - DSL 继续只做共享能力的配置化入口。
  - plugin 体系当前只把 `middleware` 作为已支持的 engine 装配点。
  - `rules`、`provider`、`storage` 目前只保留为 manifest 命名空间；在对应底层运行时 owner 尚未稳定前，不开放新的 engine plugin kind。

## 验证方式

- 先用本 change 的 tasks 作为总台账。
- 后续每完成一个能力子项时：
  - 先更新对应 spec delta
  - 再修改 `src/` 代码
  - 最后运行针对性的 `cargo test` / `cargo check --examples`
- 对已有行为边界可能变化的模块，优先补单元测试，再补实现。
