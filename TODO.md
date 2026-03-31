# TODO

这里只保留当前仍然成立、并且会影响后续实现取舍的能力缺口。
更细的任务拆分以 `openspec/changes/track-kun-core-capability-gaps/tasks.md` 为准。

## 1. Parser 缺口

- HTML XPath 仍然暂缓。当前 XPath 基于 XML 解析器，在不规范 HTML 上不稳定，HTML 场景继续建议优先使用 CSS。
- `ocr` 相关解析能力仍未实现，当前不纳入落地范围。
- parse 后处理还比较薄：多选择器兜底、normalize、类型转换等能力还没有统一抽象。

## 2. Validation 缺口

- 共享 validation 目前已经支持 `type` 与 `rule.required`。
- `regex`、`min/max`、`enum` 等规则仍未进入共享 validation 能力。
- 嵌套对象、列表成员、字段级 normalize/转换后的再校验还没有统一语义。

## 3. Request / Browser / Middleware 缺口

- 代码爬虫侧的 request 能力已经比较完整，但 DSL 到共享 request 模型的映射还没有完全收敛。
- browser 路线当前已经接住统一 `Request` 上的 `method`、`body`、cookies 与最小 `session` 复用能力；`stealth`、`fingerprint_profile` 仍未实现。
- browser `Response` 现在已经接住真实的导航 `status` 与响应头；`protocol` 继续表示 browser 执行语义，`ip_address` / `certificate` 仍受 Playwright 当前接口限制而保持为空。
- `Response.text` 现在已经统一从 `Response.body` 解码，并支持 BOM / `Content-Type charset` / 文档声明；但还没有做统计型 `apparent encoding` 猜测。
- browser `session` 当前已经通过稳定的 Playwright user data dir 落了最小复用能力，并对相同 session id 做了最小串行协调；但还没有更细粒度的 browser context / page 复用策略。
- 统一 request cookies 目前只建模扁平的 key/value；domain、path、expires、same-site 这类更细粒度 cookie 属性还没有进入公开 `Request` API。
- proxy / cookies 已接到真实 HTTP 下载链路，但更细的 DSL 配置面和高级策略还没有统一。

## 4. Scheduler / Frontier 缺口

- 当前只有 `scheduler::Memory`，进程退出后 ready / delayed / inflight 状态不会持久化。
- 还没有明确 durable scheduler/frontier 的最小接口边界，例如本地磁盘、SQLite、Redis 或其它可恢复实现。

## 5. Pipeline 与输出能力

- 当前只有一条 `pipeline` item 处理链路，不再维护独立的 `output.sinks` 概念。
- 内置输出能力目前已有 `pipeline::Memory` 与 `pipeline::JsonLines`。
- 文件、数据库、消息队列等 pipeline 仍待按模块逐步补齐。

## 6. Plugin 边界

- `Engine::load_plugins()` 当前只支持 `middleware` kind 的自动装载。
- `rules`、`provider`、`storage` 目前只是已命名的 manifest kind，不代表底层 runtime 已经具备对应装配能力。
- 如果后面要扩展新的 plugin kind，必须先有清晰的底层 owner：例如稳定的 pipeline/storage runtime、provider 抽象或其它可验证的共享链路。

## 7. DSL 对齐缺口

- DSL 配置面仍然整体后置，优先跟随代码爬虫与共享底层能力收敛。
- `step.meta`、`links[].meta`、dedup / schedule / retry、以及 step validate 已经落到底层能力。
- `next_url_config` 已支持 `FIELD`、`TEMPLATE`、`JOIN`、`FUNCTION`，其中最小函数集为 `concat`、`replace`、`coalesce`。
- 日期/时间表达式、更多函数、以及更完整的 request / parse 配置还没有统一设计。

## 8. API 命名与导出

- 评估 `item::Item` 这个公开路径是否需要继续收口成更顺手的对外导出，同时保持与 `request::Request`、`response::Response`、`settings::Settings` 的公开 API 一致性。

## 9. Engine 结构缺口

- `engine` 当前主循环和任务执行逻辑已经拆分出独立子模块；如果后面继续增长，再评估是否把调度主循环、runtime 组装等职责继续细分。
