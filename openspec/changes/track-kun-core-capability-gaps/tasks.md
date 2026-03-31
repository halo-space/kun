# 任务清单

## 1. 共享 Validation 能力

- [x] 1.1 定义共享 validation plan 结构，并把它从 `rules` 的配置字段提升为可复用的底层能力。
- [x] 1.2 在 DSL 执行链中实际执行字段校验，而不是只解析 `step.validate`。
- [x] 1.3 为代码爬虫提供直接调用同一套 validation 能力的 API，而不是要求代码模式自行手写重复逻辑。
- [x] 1.4 明确校验失败的运行时语义：报错、丢弃还是可配置策略。
- [x] 1.5 同步 `openspec/specs/spider-api/spec.md`、`openspec/specs/rules-dsl/spec.md` 与相关测试。

## 2. Request / Follow / Session 能力

- [x] 2.1 为 `Request` 补齐 timeout、proxy、cookies/session 等请求级能力建模。
- [x] 2.2 重新梳理 `response.follow()` 的派生语义，明确哪些 request 属性应继承、哪些可覆盖。
- [ ] 2.3 让代码爬虫与 DSL 生成的 request 都走同一套 request 能力模型。
- [x] 2.4 同步 `openspec/specs/spider-api/spec.md` 与 request/follow 相关测试。
- [x] 2.5 把 request-level cookies 从 `http` 私有配置进一步收口为真正的共享 request 能力，避免 browser request 使用 cookies 时退回 `Http` 模式。

## 3. Scheduler 与任务身份

- [x] 3.1 为 scheduler 引入稳定的 task identity，而不是只按 URL ack/nack。
- [x] 3.2 明确 retry、delayed task、inflight task 在 task identity 下的行为。
- [x] 3.3 校验 scheduler 在“同 URL 不同 meta/body/method”场景下的正确性。
- [x] 3.4 同步 `openspec/specs/runtime-engine/spec.md` 与 scheduler 测试。
- [x] 3.5 明确当前 scheduler 的 memory-only 边界，并规划 durable scheduler/state store 的最小接口或实现方向。

## 4. Cookies / Proxy / HTTP 真实能力

- [x] 4.1 将 `cookies` middleware 从空壳实现补成真实行为，并与 request/session 语义对齐。
- [x] 4.2 将 `proxy` middleware 从空壳实现补成真实行为，并明确 rotate/failure/pool 的最小能力边界。
- [x] 4.3 为 HTTP downloader 补 timeout、cookie jar、proxy、redirect 相关能力的统一接线。
- [x] 4.4 同步 `openspec/specs/middleware-plugins/spec.md`、`openspec/specs/runtime-engine/spec.md` 与网络层测试。

## 5. Browser 能力边界

- [x] 5.1 统一 browser request 配置模型与实际 downloader 实现，消除配置语义和实现语义不一致的问题。
- [x] 5.2 明确未启用 browser feature 时的行为边界，应为显式失败还是受限 stub。
- [x] 5.3 为 browser 模式补最小可验证的真实行为测试或契约测试。
- [x] 5.4 同步 `openspec/specs/spider-api/spec.md` 与相关文档。
- [x] 5.5 为 browser response 补真实的 `status`、`headers`、`protocol`、`ip_address`、`certificate` 等网络元数据，或明确并固化受限语义。
- [x] 5.6 让 browser downloader 接住统一 request-level cookies 语义，而不是只复用 session 持久态。
- [x] 5.7 为相同 browser session 的并发执行建立最小协调策略，避免共享 user data dir 时出现竞态。

## 6. Parser 缺口补齐

当前暂缓：HTML XPath 目前没有稳定好用的底层库方案，`ocr` 也暂无可落地实现；这组任务先不纳入当前实现范围。

- [ ] 6.1 补齐 HTML XPath 能力，或明确提供稳定的 HTML XPath 替代实现。
- [ ] 6.2 为 `ocr` selector_type 补真正 parser/runtime 支持，或从 schema 中移除占位能力。
- [ ] 6.3 规划 parse 后处理能力：多选择器兜底、normalize、类型转换、结构化校验。
- [ ] 6.4 同步 `README.md`、`TODO.md` 与 parser 测试。

## 7. Pipeline Item 处理能力

- [x] 7.1 将当前轻量 `Pipeline` 收口为唯一的 item 处理/输出链路，移除独立 `ItemOutput` / `sink` 运行时概念。
- [x] 7.2 提供最小内置 pipeline 输出能力：补齐 `pipeline::Memory`，并以 pipeline 方式承载持久化扩展点。
- [x] 7.3 明确 pipeline 错误与 item 丢弃的运行时语义。
- [x] 7.4 同步 `openspec/specs/runtime-engine/spec.md` 与 pipeline 输出测试。

## 8. Plugin 扩展边界

当前决策：`Engine::load_plugins()` 只支持 `middleware` kind；`rules`、`provider`、`storage` 先保留命名空间，不作为已落地运行时能力对外承诺。

- [x] 8.1 明确 plugin 体系当前支持的 kind 边界，避免 registry 泛化而 engine 只支持部分 kind。
- [x] 8.2 评估是否需要在 middleware 之外扩展 plugin kind，以及扩展前的底层前提条件。
- [x] 8.3 同步 `openspec/specs/middleware-plugins/spec.md` 与插件加载校验测试。

## 9. DSL 与共享底层能力对齐

当前后置：先补齐并稳定代码爬虫与共享底层能力，再回头收口 DSL 配置面与运行时映射。

- [ ] 9.1 逐项梳理 DSL 字段里哪些能力已经进入共享底层，哪些仍然只是配置占位。
- [ ] 9.2 确保 `validate`、request、cookies、proxy、output 等能力优先作为底层能力实现，再映射到 DSL。
- [ ] 9.3 同步 `openspec/specs/rules-dsl/spec.md`，明确 DSL 不应发明独立运行时。

## 10. 文档与持续验证

- [x] 10.1 在 `README.md` 中维护一份简短的“已完成底层能力 / 待补能力”说明。
- [x] 10.2 按能力子项补充单元测试或集成测试，避免只补文档不补验证。
- [x] 10.3 每完成一个能力分组后运行 `cargo test`。
- [x] 10.4 如涉及示例变化，运行 `cargo check --examples`。

## 11. Engine 结构整理

- [x] 11.1 收口 `engine` 中任务执行参数，把 task run / reservation 相关上下文整理成更稳定的结构体，而不是继续扩散参数列表。
- [x] 11.2 评估 `TaskOutcome` 的体积差异，明确是否通过 `Box` 或其它方式降低大枚举变体的拷贝/栈占用。
- [x] 11.3 在不改变现有运行时语义的前提下，为 `engine` 结构整理补最小回归测试。
- [x] 11.4 清理 `engine` 内部局部命名，去掉 `sem` 一类缩写，统一使用 `global_semaphore`、`domain_semaphore` 这类完整命名。
- [x] 11.5 如果本轮仍暂时保留单文件实现，下一轮评估将任务执行相关逻辑拆到独立子模块，至少明确 `task executor` / `task run apply` 的拆分边界。
- [x] 11.6 统一 `engine` 内部任务执行命名语义，避免 `TaskOutcome`、`TaskRun`、`run()`、`map_flow_to_task_outcome()` 这类跨层级混用；明确“middleware flow”“task run/outcome”“run apply/handle”各自的命名边界。

## 12. Response Body / Text 语义

- [x] 12.1 明确 `Response.body` 保存原始字节，`Response.text` 是从 `body` 解码得到的字符串视图，而不是单独维护另一份来源。
- [x] 12.2 为 `Response` 构造路径补统一的文本解码逻辑，优先使用 BOM、`Content-Type charset` 与文档声明，再回退 UTF-8 lossy。
- [x] 12.3 为 HTTP 下载链路补编码回归测试，并同步 `openspec/specs/spider-api/spec.md`、`README.md`、`TODO.md`。
