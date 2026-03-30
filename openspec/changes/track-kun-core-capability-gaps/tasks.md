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

## 3. Scheduler 与任务身份

- [x] 3.1 为 scheduler 引入稳定的 task identity，而不是只按 URL ack/nack。
- [x] 3.2 明确 retry、delayed task、inflight task 在 task identity 下的行为。
- [x] 3.3 校验 scheduler 在“同 URL 不同 meta/body/method”场景下的正确性。
- [x] 3.4 同步 `openspec/specs/runtime-engine/spec.md` 与 scheduler 测试。

## 4. Cookies / Proxy / HTTP 真实能力

- [x] 4.1 将 `cookies` middleware 从空壳实现补成真实行为，并与 request/session 语义对齐。
- [x] 4.2 将 `proxy` middleware 从空壳实现补成真实行为，并明确 rotate/failure/pool 的最小能力边界。
- [x] 4.3 为 HTTP downloader 补 timeout、cookie jar、proxy、redirect 相关能力的统一接线。
- [x] 4.4 同步 `openspec/specs/middleware-plugins/spec.md`、`openspec/specs/runtime-engine/spec.md` 与网络层测试。

## 5. Browser 能力边界

- [ ] 5.1 统一 browser request 配置模型与实际 downloader 实现，消除配置语义和实现语义不一致的问题。
- [ ] 5.2 明确未启用 browser feature 时的行为边界，应为显式失败还是受限 stub。
- [ ] 5.3 为 browser 模式补最小可验证的真实行为测试或契约测试。
- [ ] 5.4 同步 `openspec/specs/spider-api/spec.md` 与相关文档。

## 6. Parser 缺口补齐

- [ ] 6.1 补齐 HTML XPath 能力，或明确提供稳定的 HTML XPath 替代实现。
- [ ] 6.2 为 `ocr` selector_type 补真正 parser/runtime 支持，或从 schema 中移除占位能力。
- [ ] 6.3 规划 parse 后处理能力：多选择器兜底、normalize、类型转换、结构化校验。
- [ ] 6.4 同步 `README.md`、`TODO.md` 与 parser 测试。

## 7. Output / Pipeline / Sink 能力

- [ ] 7.1 将当前轻量 `Pipeline` 与 `ItemOutput` 梳理成更明确的输出能力边界。
- [ ] 7.2 定义最小 output/sink 能力：写入策略、失败策略、内存输出之外的持久化接口。
- [ ] 7.3 明确 pipeline 错误、sink 错误与 item 丢弃的运行时语义。
- [ ] 7.4 同步 `openspec/specs/runtime-engine/spec.md` 与输出层测试。

## 8. Plugin 扩展边界

- [ ] 8.1 明确 plugin 体系当前支持的 kind 边界，避免 registry 泛化而 engine 只支持部分 kind。
- [ ] 8.2 评估是否需要在 middleware 之外扩展 plugin kind，以及扩展前的底层前提条件。
- [ ] 8.3 同步 `openspec/specs/middleware-plugins/spec.md` 与插件加载校验测试。

## 9. DSL 与共享底层能力对齐

- [ ] 9.1 逐项梳理 DSL 字段里哪些能力已经进入共享底层，哪些仍然只是配置占位。
- [ ] 9.2 确保 `validate`、request、cookies、proxy、output 等能力优先作为底层能力实现，再映射到 DSL。
- [ ] 9.3 同步 `openspec/specs/rules-dsl/spec.md`，明确 DSL 不应发明独立运行时。

## 10. 文档与持续验证

- [ ] 10.1 在 `README.md` 中维护一份简短的“已完成底层能力 / 待补能力”说明。
- [ ] 10.2 按能力子项补充单元测试或集成测试，避免只补文档不补验证。
- [ ] 10.3 每完成一个能力分组后运行 `cargo test`。
- [ ] 10.4 如涉及示例变化，运行 `cargo check --examples`。
